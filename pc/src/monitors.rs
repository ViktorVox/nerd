// Мониторы и выбор того, который показывается на телефоне
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::core::BOOL;

// Прямоугольник на рабочем столе: x, y, ширина, высота
pub type Rect = (i32, i32, i32, i32);

pub struct Monitor {
    pub num: u32,
    pub rect: Rect,
    pub primary: bool,
}

unsafe extern "system" fn collect(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let list = &mut *(data.0 as *mut Vec<Monitor>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(h, &mut info as *mut _ as *mut MONITORINFO).as_bool() {
        let name = String::from_utf16_lossy(&info.szDevice);
        let num = name.trim_end_matches('\0').rsplit("DISPLAY").next().and_then(|n| n.parse().ok());
        let r = info.monitorInfo.rcMonitor;
        list.push(Monitor {
            num: num.unwrap_or(0),
            rect: (r.left, r.top, r.right - r.left, r.bottom - r.top),
            primary: info.monitorInfo.dwFlags & 1 != 0,
        });
    }
    true.into()
}

// Номер совпадает с \\.\DISPLAYn из Windows
pub fn list() -> Vec<Monitor> {
    let mut list: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut list as *mut _ as isize));
    }
    list
}

static SELECTED: Mutex<Option<u32>> = Mutex::new(None);
static RECT_NOW: Mutex<Rect> = Mutex::new((0, 0, 1920, 1080));
static GENERATION: AtomicU32 = AtomicU32::new(0);

// None — основной монитор Windows
pub fn select(num: Option<u32>) {
    let all = list();
    let chosen = num
        .and_then(|n| all.iter().find(|m| m.num == n))
        .or_else(|| all.iter().find(|m| m.primary))
        .or_else(|| all.first());
    if let Some(m) = chosen {
        *RECT_NOW.lock().unwrap() = m.rect;
        *SELECTED.lock().unwrap() = Some(m.num);
        GENERATION.fetch_add(1, Ordering::SeqCst);
    }
}

pub fn selected() -> Option<u32> {
    *SELECTED.lock().unwrap()
}

pub fn rect() -> Rect {
    *RECT_NOW.lock().unwrap()
}

// Меняется при каждой смене монитора: захват экрана тогда создаётся заново
pub fn generation() -> u32 {
    GENERATION.load(Ordering::SeqCst)
}
