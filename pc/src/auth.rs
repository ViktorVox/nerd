// Защита подключения: список разрешённых устройств, окно «Разрешить / Отклонить», код сессии для UDP-пакетов
use crate::log;
use std::hash::BuildHasher;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const DENY_LOCKOUT: Duration = Duration::from_secs(30);

struct Session {
    ip: IpAddr,
    tag: u16,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
// Окно запроса показывается по одному; после «Отклонить» новые запросы с неизвестных устройств отбиваются на время
static ASKING: Mutex<()> = Mutex::new(());
static DENIED_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

pub struct Device {
    pub id: [u8; 16],
    pub token: [u8; 16],
    pub name: String,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

fn path() -> std::path::PathBuf {
    crate::settings::dir().join("devices.txt")
}

fn load() -> Vec<Device> {
    let text = std::fs::read_to_string(path()).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let mut p = l.splitn(3, ' ');
            Some(Device { id: unhex(p.next()?)?, token: unhex(p.next()?)?, name: p.next()?.to_string() })
        })
        .collect()
}

fn save(devices: &[Device]) {
    let text: String = devices.iter().map(|d| format!("{} {} {}\n", hex(&d.id), hex(&d.token), d.name)).collect();
    let _ = std::fs::write(path(), text);
}

pub fn count() -> usize {
    load().len()
}

pub fn forget_all() {
    let _ = std::fs::remove_file(path());
    *SESSION.lock().unwrap() = None;
    log!("все устройства забыты");
}

fn random_u16() -> u16 {
    loop {
        let v = (std::collections::hash_map::RandomState::new().hash_one(0u8) & 0xFFFF) as u16;
        if v != 0 {
            return v;
        }
    }
}

// Одна активная сессия; её код и адрес должны совпасть у каждого UDP-пакета
pub fn start_session(ip: IpAddr) -> u16 {
    let tag = random_u16();
    *SESSION.lock().unwrap() = Some(Session { ip, tag });
    tag
}

pub fn end_session(tag: u16) {
    let mut s = SESSION.lock().unwrap();
    if s.as_ref().is_some_and(|x| x.tag == tag) {
        *s = None;
    }
}

pub fn session_ok(ip: IpAddr, tag: u16) -> bool {
    SESSION.lock().unwrap().as_ref().is_some_and(|s| s.tag == tag && s.ip == ip)
}

// Имя приходит с устройства, ему не доверяем: без служебных символов и разумной длины
pub fn clean_name(raw: &[u8]) -> String {
    let s: String = String::from_utf8_lossy(raw).chars().filter(|c| !c.is_control()).take(40).collect();
    if s.trim().is_empty() { "устройство".into() } else { s }
}

// Известное устройство пускаем сразу, неизвестное спрашиваем у пользователя
pub fn authorize(id: [u8; 16], token: [u8; 16], name: &str, ip: IpAddr) -> bool {
    let mut devices = load();
    if let Some(d) = devices.iter().find(|d| d.id == id) {
        let ok = d.token == token;
        if !ok {
            log!("отказано {ip}: неверный ключ у известного устройства «{}»", d.name);
        }
        return ok;
    }
    if DENIED_UNTIL.lock().unwrap().is_some_and(|t| Instant::now() < t) {
        log!("отказано {ip}: недавно отклонили запрос");
        return false;
    }
    let _one_at_a_time = ASKING.lock().unwrap();
    // пока ждали очереди, другой запрос мог разрешить это же устройство
    if load().iter().any(|d| d.id == id && d.token == token) {
        return true;
    }
    log!("запрос на подключение: «{name}» {ip}");
    if crate::dialog::ask(name, &ip.to_string()) {
        devices.push(Device { id, token, name: name.to_string() });
        save(&devices);
        log!("разрешено: «{name}»");
        true
    } else {
        *DENIED_UNTIL.lock().unwrap() = Some(Instant::now() + DENY_LOCKOUT);
        log!("отклонено: «{name}»");
        false
    }
}
