// Сетевая часть: перо и мышь по UDP, экран по TCP, поиск ПК телефоном
use crate::auth;
use crate::capture::{capture_gdi, encode_jpeg};
use crate::dxgi::Dxgi;
use crate::log;
use crate::monitors::{self, Rect};
use crate::pen::{self, Pen};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const PORT: u16 = 47800;
const SCREEN_PORT: u16 = 47801;
const DISCOVERY_PORT: u16 = 47802;

static CLIENTS: AtomicUsize = AtomicUsize::new(0);

pub fn serve_udp() {
    let Ok(mut pen) = Pen::new() else {
        log!("не удалось создать виртуальное перо (нужен Windows 10 1809 или новее)");
        return;
    };
    let Ok(sock) = UdpSocket::bind(("0.0.0.0", PORT)) else {
        log!("порт {PORT} занят: возможно, программа уже запущена");
        return;
    };
    log!("перо и мышь: UDP {PORT}");
    let mut buf = [0u8; 64];
    loop {
        match sock.recv_from(&mut buf) {
            Ok((n, from)) => {
                // байты 2-3 каждого пакета — код сессии, выданный ПК после подтверждения
                if n < 4 || !auth::session_ok(from.ip(), u16::from_le_bytes([buf[2], buf[3]])) {
                    continue;
                }
                if buf[0] == pen::CMD_MAGIC {
                    pen::command(&buf[..n]);
                } else if let Some(p) = pen::parse(&buf[..n]) {
                    pen.inject(&p);
                }
            }
            Err(e) => log!("ошибка приёма UDP: {e}"),
        }
    }
}

fn seed(r: Rect) -> impl FnOnce() -> Option<Vec<u8>> {
    move || capture_gdi(r.0, r.1, r.2, r.3, r.2, r.3)
}

struct Guard(u16);

impl Drop for Guard {
    fn drop(&mut self) {
        auth::end_session(self.0);
        if CLIENTS.fetch_sub(1, Ordering::SeqCst) == 1 {
            crate::tray::set_status(false, "Ждёт телефон", None);
        }
    }
}

// Приветствие: "NERD" | версия 2 | id телефона 16 байт | ключ 16 байт | длина имени u8 | имя.
// Ответ ПК: 0 — отказ; 1 | код сессии u16 | текущий монитор u8 | число мониторов u8 | (номер u8, ширина u16, высота u16)...
fn handshake(s: &mut TcpStream, ip: std::net::IpAddr) -> std::io::Result<Option<String>> {
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut h = [0u8; 38];
    s.read_exact(&mut h)?;
    if &h[..4] != b"NERD" || h[4] != 2 {
        return Ok(None);
    }
    let mut name = vec![0u8; h[37] as usize];
    s.read_exact(&mut name)?;
    s.set_read_timeout(None)?;
    let name = auth::clean_name(&name);
    let id: [u8; 16] = h[5..21].try_into().unwrap();
    let token: [u8; 16] = h[21..37].try_into().unwrap();
    // здесь может показаться окно «Разрешить / Отклонить»
    if !auth::authorize(id, token, &name, ip) {
        s.write_all(&[0])?;
        return Ok(None);
    }
    Ok(Some(name))
}

// Дальше запросы телефона: тип u8. 0 — кадр (24 байта): x f32, y f32, w f32, h f32 (доли монитора) |
// out_w u16, out_h u16 | quality u8 | 3 байта. 1 — выбрать монитор (номер u8).
// Ответ на кадр: x f32, y f32, w f32, h f32 | mon_w u16, mon_h u16 | len u32 | jpeg
fn serve_client(mut s: TcpStream) -> std::io::Result<()> {
    s.set_nodelay(true)?;
    let ip = s.peer_addr()?.ip();
    log!("подключается {ip}");
    let Some(name) = handshake(&mut s, ip)? else { return Ok(()) };
    let tag = auth::start_session(ip);
    let _guard = Guard(tag);
    CLIENTS.fetch_add(1, Ordering::SeqCst);
    crate::tray::set_status(true, &format!("Подключён: {name}"), Some(format!("Подключён: {name}")));
    log!("подключён «{name}» {ip}");

    let mons = monitors::list();
    let mut reply = vec![1u8];
    reply.extend_from_slice(&tag.to_le_bytes());
    reply.push(monitors::selected().unwrap_or(0) as u8);
    reply.push(mons.len() as u8);
    for m in &mons {
        reply.push(m.num as u8);
        reply.extend_from_slice(&(m.rect.2 as u16).to_le_bytes());
        reply.extend_from_slice(&(m.rect.3 as u16).to_le_bytes());
    }
    s.write_all(&reply)?;

    let mut req = [0u8; 24];
    let (mut frames, mut spent, mut bytes) = (0u32, Duration::ZERO, 0usize);
    let mut cap_spent = Duration::ZERO;
    let mut since = Instant::now();
    let mut generation = monitors::generation();
    let mut gpu = Dxgi::new(monitors::rect(), seed(monitors::rect()));
    log!("захват экрана: {}", if gpu.is_some() { "видеокарта (DXGI)" } else { "GDI" });
    loop {
        let mut kind = [0u8; 1];
        s.read_exact(&mut kind)?;
        match kind[0] {
            0 => {}
            1 => {
                let mut n = [0u8; 1];
                s.read_exact(&mut n)?;
                monitors::select(Some(n[0] as u32));
                crate::settings::save(&crate::settings::Settings { monitor: monitors::selected() });
                continue;
            }
            _ => return Ok(()),
        }
        s.read_exact(&mut req)?;
        if generation != monitors::generation() {
            generation = monitors::generation();
            gpu = Dxgi::new(monitors::rect(), seed(monitors::rect()));
        }
        let mon = monitors::rect();
        let (x0, y0, w, h) = mon;
        let f = |i: usize| f32::from_le_bytes(req[i..i + 4].try_into().unwrap());
        let (rx, ry) = (f(0).clamp(0.0, 1.0), f(4).clamp(0.0, 1.0));
        let (rw, rh) = (f(8).clamp(0.0, 1.0 - rx), f(12).clamp(0.0, 1.0 - ry));
        let ow = (u16::from_le_bytes([req[16], req[17]]) as i32).clamp(16, 3840);
        let oh = (u16::from_le_bytes([req[18], req[19]]) as i32).clamp(16, 2160);
        let quality = req[20].clamp(10, 95);
        let sx = x0 + (rx * w as f32) as i32;
        let sy = y0 + (ry * h as f32) as i32;
        let sw = ((rw * w as f32) as i32).max(1);
        let sh = ((rh * h as f32) as i32).max(1);
        let t0 = Instant::now();
        let shot = match gpu.as_mut().map(|g| g.grab(sx - x0, sy - y0, sw, sh, ow, oh)) {
            Some(Some(buf)) => Some(buf),
            Some(None) => {
                gpu = Dxgi::new(mon, seed(mon));
                capture_gdi(sx, sy, sw, sh, ow, oh)
            }
            None => capture_gdi(sx, sy, sw, sh, ow, oh),
        };
        let cap = t0.elapsed();
        let Some(jpeg) = shot.and_then(|buf| encode_jpeg(&buf, ow, oh, quality)) else {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        };
        frames += 1;
        spent += t0.elapsed();
        cap_spent += cap;
        bytes += jpeg.len();
        if since.elapsed() >= Duration::from_secs(3) {
            log!(
                "кадров {frames} ({:.0}/с), съёмка {:.1} мс + JPEG {:.1} мс, {} КБ/кадр",
                frames as f32 / since.elapsed().as_secs_f32(),
                cap_spent.as_secs_f32() * 1000.0 / frames as f32,
                (spent - cap_spent).as_secs_f32() * 1000.0 / frames as f32,
                bytes / frames as usize / 1024
            );
            (frames, spent, bytes, since, cap_spent) = (0, Duration::ZERO, 0, Instant::now(), Duration::ZERO);
        }
        let mut head = Vec::with_capacity(24);
        for v in [rx, ry, rw, rh] {
            head.extend_from_slice(&v.to_le_bytes());
        }
        head.extend_from_slice(&(w as u16).to_le_bytes());
        head.extend_from_slice(&(h as u16).to_le_bytes());
        head.extend_from_slice(&(jpeg.len() as u32).to_le_bytes());
        s.write_all(&head)?;
        s.write_all(&jpeg)?;
    }
}

pub fn serve_screen() {
    let Ok(listener) = TcpListener::bind(("0.0.0.0", SCREEN_PORT)) else {
        log!("порт {SCREEN_PORT} занят: возможно, программа уже запущена");
        return;
    };
    log!("экран: TCP {SCREEN_PORT}");
    for stream in listener.incoming().flatten() {
        std::thread::spawn(move || {
            let _ = serve_client(stream);
        });
    }
}

// Телефон рассылает «NERD?» по локальной сети, ПК отвечает «NERD!» и своим именем
pub fn serve_discovery() {
    let Ok(sock) = UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)) else {
        log!("порт поиска {DISCOVERY_PORT} занят: телефон не найдёт ПК сам");
        return;
    };
    let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "ПК".into());
    let reply = format!("NERD!{name}");
    let mut buf = [0u8; 16];
    while let Ok((n, from)) = sock.recv_from(&mut buf) {
        if &buf[..n] == b"NERD?" {
            let _ = sock.send_to(reply.as_bytes(), from);
        }
    }
}
