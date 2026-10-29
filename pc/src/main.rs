use std::net::UdpSocket;
use std::time::Duration;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::Controls::{
    CreateSyntheticPointerDevice, HSYNTHETICPOINTERDEVICE,
    POINTER_FEEDBACK_DEFAULT, POINTER_TYPE_INFO, POINTER_TYPE_INFO_0,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, MOUSE_EVENT_FLAGS,
};
use windows::Win32::UI::Input::Pointer::{
    POINTER_FLAG_DOWN, POINTER_FLAG_FIRSTBUTTON, POINTER_FLAG_INCONTACT, POINTER_FLAG_INRANGE,
    POINTER_FLAG_NEW, POINTER_FLAG_UP, POINTER_FLAG_UPDATE, POINTER_INFO, POINTER_PEN_INFO,
    GetPointerPenInfo, InjectSyntheticPointerInput,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetSystemMetrics, PeekMessageW,
    RegisterClassW, SetLayeredWindowAttributes, ShowWindow, TranslateMessage, LWA_ALPHA, MSG,
    PEN_MASK_PRESSURE, PEN_MASK_TILT_X, PEN_MASK_TILT_Y, PM_REMOVE, PT_PEN, SM_CXSCREEN,
    SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SW_SHOWNOACTIVATE, WM_POINTERENTER, WM_POINTERUPDATE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::BOOL;

const PORT: u16 = 47800;
const MAGIC: u8 = 0x4E;
const CMD_MAGIC: u8 = 0x4F;
const PACKET_LEN: usize = 20;

// Действия из пакета
const HOVER: u8 = 0; // перо над экраном
const DOWN: u8 = 1; // касание
const DRAG: u8 = 2; // движение с касанием
const UP: u8 = 3; // отрыв
const LEAVE: u8 = 4; // перо ушло из зоны

struct Packet {
    action: u8,
    x: f32,
    y: f32,
    pressure: f32,
    tilt_x: i16,
    tilt_y: i16,
}

// Формат (20 байт, little-endian):
// magic u8 | action u8 | buttons u8 | pad u8 | x f32 | y f32 | pressure f32 | tilt_x i16 | tilt_y i16
fn parse(buf: &[u8]) -> Option<Packet> {
    if buf.len() < PACKET_LEN || buf[0] != MAGIC {
        return None;
    }
    let f = |i: usize| f32::from_le_bytes(buf[i..i + 4].try_into().unwrap());
    let s = |i: usize| i16::from_le_bytes(buf[i..i + 2].try_into().unwrap());
    Some(Packet {
        action: buf[1],
        x: f(4),
        y: f(8),
        pressure: f(12),
        tilt_x: s(16),
        tilt_y: s(18),
    })
}

unsafe extern "system" fn collect_monitor(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let list = &mut *(data.0 as *mut Vec<(u32, RECT)>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(h, &mut info as *mut _ as *mut MONITORINFO).as_bool() {
        let name = String::from_utf16_lossy(&info.szDevice);
        let num = name.trim_end_matches('\0').rsplit("DISPLAY").next().and_then(|n| n.parse().ok());
        list.push((num.unwrap_or(0), info.monitorInfo.rcMonitor));
    }
    true.into()
}

// Номер совпадает с \\.\DISPLAYn из Windows
fn monitors() -> Vec<(u32, RECT)> {
    let mut list: Vec<(u32, RECT)> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect_monitor), LPARAM(&mut list as *mut _ as isize));
    }
    list
}

struct Pen {
    device: HSYNTHETICPOINTERDEVICE,
    in_range: bool,
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
}

impl Pen {
    fn new(monitor: Option<u32>) -> windows::core::Result<Self> {
        let device = unsafe { CreateSyntheticPointerDevice(PT_PEN, 1, POINTER_FEEDBACK_DEFAULT)? };
        let (mut x0, mut y0) = (0, 0);
        let (mut w, mut h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        if let Some(r) = monitor.and_then(|n| monitors().into_iter().find(|m| m.0 == n)).map(|m| m.1) {
            (x0, y0, w, h) = (r.left, r.top, r.right - r.left, r.bottom - r.top);
        }
        Ok(Self { device, in_range: false, x0, y0, w, h })
    }

    // Доля монитора (0..1) в пиксели рабочего стола
    fn pixel(&self, x: f32, y: f32) -> (i32, i32) {
        (
            self.x0 + (x.clamp(0.0, 1.0) * (self.w - 1) as f32) as i32,
            self.y0 + (y.clamp(0.0, 1.0) * (self.h - 1) as f32) as i32,
        )
    }

    fn inject(&mut self, p: &Packet) {
        let (x, y) = self.pixel(p.x, p.y);
        self.inject_px(p, x, y);
    }

    // Система считает координаты от левого верхнего угла всего рабочего стола
    fn inject_px(&mut self, p: &Packet, x: i32, y: i32) {
        let (x, y) = unsafe {
            (
                x - GetSystemMetrics(SM_XVIRTUALSCREEN),
                y - GetSystemMetrics(SM_YVIRTUALSCREEN),
            )
        };
        let mut flags = match p.action {
            HOVER => POINTER_FLAG_INRANGE | POINTER_FLAG_UPDATE,
            DOWN => {
                POINTER_FLAG_INRANGE
                    | POINTER_FLAG_INCONTACT
                    | POINTER_FLAG_DOWN
                    | POINTER_FLAG_FIRSTBUTTON
            }
            DRAG => {
                POINTER_FLAG_INRANGE
                    | POINTER_FLAG_INCONTACT
                    | POINTER_FLAG_UPDATE
                    | POINTER_FLAG_FIRSTBUTTON
            }
            UP => POINTER_FLAG_INRANGE | POINTER_FLAG_UP,
            LEAVE => POINTER_FLAG_UPDATE,
            _ => return,
        };
        // Первое сообщение после входа в зону должно быть NEW
        if p.action != LEAVE && !self.in_range {
            flags |= POINTER_FLAG_NEW;
            self.in_range = true;
        }
        if p.action == LEAVE {
            self.in_range = false;
        }

        let touching = matches!(p.action, DOWN | DRAG);
        let pressure = if touching {
            ((p.pressure.clamp(0.0, 1.0) * 1024.0) as u32).max(1)
        } else {
            0
        };

        let pen = POINTER_PEN_INFO {
            pointerInfo: POINTER_INFO {
                pointerType: PT_PEN,
                pointerId: 0,
                pointerFlags: flags,
                ptPixelLocation: POINT { x, y },
                ..Default::default()
            },
            penMask: PEN_MASK_PRESSURE | PEN_MASK_TILT_X | PEN_MASK_TILT_Y,
            pressure,
            tiltX: p.tilt_x.clamp(-90, 90) as i32,
            tiltY: p.tilt_y.clamp(-90, 90) as i32,
            ..Default::default()
        };
        let info = POINTER_TYPE_INFO {
            r#type: PT_PEN,
            Anonymous: POINTER_TYPE_INFO_0 { penInfo: pen },
        };
        if let Err(e) = unsafe { InjectSyntheticPointerInput(self.device, &[info]) } {
            eprintln!("inject error: {e}");
        }
    }
}

// Проверка без телефона: перо рисует круг в центре экрана
fn demo(pen: &mut Pen, at: Option<(i32, i32, i32)>) {
    let pk = |action, pressure| Packet { action, x: 0.0, y: 0.0, pressure, tilt_x: 0, tilt_y: 0 };
    let (cx, cy, rx, ry) = at.map(|(x, y, r)| (x, y, r, r)).unwrap_or((
        pen.x0 + pen.w / 2,
        pen.y0 + pen.h / 2,
        (0.15 * pen.w as f32) as i32,
        (0.2 * pen.h as f32) as i32,
    ));
    let pt = |t: f32| ((cx as f32 + rx as f32 * t.cos()) as i32, (cy as f32 + ry as f32 * t.sin()) as i32);
    let step = Duration::from_millis(8);
    let (x, y) = pt(0.0);
    pen.inject_px(&pk(HOVER, 0.0), x, y);
    std::thread::sleep(Duration::from_millis(300));
    pen.inject_px(&pk(DOWN, 0.5), x, y);
    for i in 1..=300 {
        let t = i as f32 / 300.0 * std::f32::consts::TAU;
        let (x, y) = pt(t);
        // давление плавно растёт и падает
        pen.inject_px(&pk(DRAG, 0.1 + 0.9 * (t / 2.0).sin().abs()), x, y);
        std::thread::sleep(step);
    }
    let (x, y) = pt(std::f32::consts::TAU);
    pen.inject_px(&pk(UP, 0.0), x, y);
    pen.inject_px(&pk(LEAVE, 0.0), x, y);
}

unsafe extern "system" fn probe_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_POINTERUPDATE || msg == WM_POINTERENTER {
        let mut info = POINTER_PEN_INFO::default();
        if GetPointerPenInfo((wp.0 & 0xFFFF) as u32, &mut info).is_ok() {
            let p = info.pointerInfo.ptPixelLocation;
            println!("  пришло в систему: ({}, {})", p.x, p.y);
        }
    }
    DefWindowProcW(h, msg, wp, lp)
}

fn pump(ms: u64) {
    let end = std::time::Instant::now() + Duration::from_millis(ms);
    let mut m = MSG::default();
    while std::time::Instant::now() < end {
        unsafe {
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

// Почти невидимое окно на всём рабочем столе принимает события пера и печатает пиксели
fn probe(pen: &mut Pen) {
    unsafe {
        let inst = GetModuleHandleW(None).unwrap();
        let cls = w!("nerd_probe");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(probe_proc),
            hInstance: inst.into(),
            lpszClassName: cls,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let (vx, vy) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
        let (vw, vh) = (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));
        println!("рабочий стол: {vw}x{vh} в точке ({vx}, {vy})");
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            cls, w!(""), WS_POPUP, vx, vy, vw, vh, None, None, Some(inst.into()), None,
        )
        .unwrap();
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 2, LWA_ALPHA);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        pump(300);

        let points = [(-1500, 540), (-300, 540), (300, 540), (1500, 540)];
        for (x, y) in points {
            println!("отправлено ({x}, {y})");
            let p = Packet { action: HOVER, x: 0.0, y: 0.0, pressure: 0.0, tilt_x: 0, tilt_y: 0 };
            pen.inject_px(&p, x, y);
            pump(700);
        }
        let p = Packet { action: LEAVE, x: 0.0, y: 0.0, pressure: 0.0, tilt_x: 0, tilt_y: 0 };
        pen.inject_px(&p, 0, 0);
        pump(200);
        let _ = DestroyWindow(hwnd);
    }
}

fn send_mouse(dx: i32, dy: i32, flags: MOUSE_EVENT_FLAGS) {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { dx, dy, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    };
    unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
}

fn mouse_move(px: i32, py: i32) {
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    let dx = ((px - vx) as i64 * 65535 / (vw - 1).max(1) as i64) as i32;
    let dy = ((py - vy) as i64 * 65535 / (vh - 1).max(1) as i64) as i32;
    send_mouse(dx, dy, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK);
}

// Формат команд: CMD_MAGIC | kind u8 | ... ; kind 0: x f32, y f32 (курсор); kind 1: кнопка u8 (0 левая, 1 правая), down u8
fn command(pen: &Pen, buf: &[u8]) {
    let f = |i: usize| f32::from_le_bytes(buf[i..i + 4].try_into().unwrap());
    match buf.get(1) {
        Some(0) if buf.len() >= 12 => {
            let (x, y) = pen.pixel(f(4), f(8));
            mouse_move(x, y);
        }
        Some(1) if buf.len() >= 4 => {
            let flag = match (buf[2], buf[3] != 0) {
                (0, true) => MOUSEEVENTF_LEFTDOWN,
                (0, false) => MOUSEEVENTF_LEFTUP,
                (1, true) => MOUSEEVENTF_RIGHTDOWN,
                (1, false) => MOUSEEVENTF_RIGHTUP,
                _ => return,
            };
            send_mouse(0, 0, flag);
        }
        _ => {}
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let monitor = args.iter().position(|a| a == "--monitor").and_then(|i| args.get(i + 1)?.parse().ok());
    for (n, r) in monitors() {
        println!("монитор {n}: {}x{} в точке ({}, {})", r.right - r.left, r.bottom - r.top, r.left, r.top);
    }
    let mut pen = Pen::new(monitor).expect("не удалось создать виртуальное перо (нужен Windows 10 1809+)");
    if monitor.is_none() {
        println!("монитор не выбран, берётся основной. Выбрать: --monitor N");
    }

    if args.iter().any(|a| a == "--probe") {
        probe(&mut pen);
        return;
    }

    if args.iter().any(|a| a == "--demo") {
        println!("демо: через 3 секунды перо нарисует круг. Открой Paint или Krita.");
        std::thread::sleep(Duration::from_secs(3));
        let at = args.iter().position(|a| a == "--at").and_then(|i| {
            let v: Vec<i32> = args.get(i + 1)?.split(',').filter_map(|n| n.parse().ok()).collect();
            (v.len() == 3).then(|| (v[0], v[1], v[2]))
        });
        demo(&mut pen, at);
        return;
    }

    let sock = UdpSocket::bind(("0.0.0.0", PORT)).expect("порт занят");
    println!("Слушаю UDP :{PORT}. Экран {}x{}", pen.w, pen.h);
    let mut buf = [0u8; 64];
    loop {
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => {
                if n > 0 && buf[0] == CMD_MAGIC {
                    command(&pen, &buf[..n]);
                } else if let Some(p) = parse(&buf[..n]) {
                    pen.inject(&p);
                }
            }
            Err(e) => eprintln!("recv error: {e}"),
        }
    }
}
