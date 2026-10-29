// Проверки без телефона (запуск с --demo или --probe): вывод идёт в консоль, из которой запущена программа
use crate::monitors;
use crate::pen::{Packet, Pen, DOWN, DRAG, HOVER, LEAVE, UP};
use std::time::Duration;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::Pointer::{GetPointerPenInfo, POINTER_PEN_INFO};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetSystemMetrics, PeekMessageW,
    RegisterClassW, SetLayeredWindowAttributes, ShowWindow, TranslateMessage, LWA_ALPHA, MSG, PM_REMOVE,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_SHOWNOACTIVATE,
    WM_POINTERENTER, WM_POINTERUPDATE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

pub fn run(args: &[String]) -> bool {
    let has = |f: &str| args.iter().any(|a| a == f);
    if !has("--probe") && !has("--demo") {
        return false;
    }
    for m in monitors::list() {
        println!("монитор {}: {}x{} в точке ({}, {})", m.num, m.rect.2, m.rect.3, m.rect.0, m.rect.1);
    }
    let mut pen = Pen::new().expect("не удалось создать виртуальное перо (нужен Windows 10 1809+)");
    if has("--probe") {
        probe(&mut pen);
    } else {
        println!("демо: через 3 секунды перо нарисует круг. Открой Paint или Krita.");
        std::thread::sleep(Duration::from_secs(3));
        let at = args.iter().position(|a| a == "--at").and_then(|i| {
            let v: Vec<i32> = args.get(i + 1)?.split(',').filter_map(|n| n.parse().ok()).collect();
            (v.len() == 3).then(|| (v[0], v[1], v[2]))
        });
        demo(&mut pen, at);
    }
    true
}

// Перо рисует круг в центре монитора (или в заданной точке)
fn demo(pen: &mut Pen, at: Option<(i32, i32, i32)>) {
    let pk = |action, pressure| Packet { action, x: 0.0, y: 0.0, pressure, tilt_x: 0, tilt_y: 0 };
    let (x0, y0, w, h) = monitors::rect();
    let (cx, cy, rx, ry) = at
        .map(|(x, y, r)| (x, y, r, r))
        .unwrap_or((x0 + w / 2, y0 + h / 2, (0.15 * w as f32) as i32, (0.2 * h as f32) as i32));
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
