// Проверки без устройства (запуск с --demo или --probe): вывод идёт в консоль, из которой запущена программа
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
    if has("--h264") {
        h264_test(args);
        return true;
    }
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

// Кодирует живой экран в H.264 (3 секунды) и пишет отчёт в %TEMP%\nerd-h264-test.txt: время, размер, состав потока
fn h264_test(args: &[String]) {
    use crate::capture::capture_gdi;
    use crate::dxgi::Dxgi;
    use crate::h264::H264;
    use std::time::Instant;
    let kbps: u32 = args.iter().position(|a| a == "--kbps").and_then(|i| args.get(i + 1)?.parse().ok()).unwrap_or(6000);
    let (ow, oh, fps, frames) = (1280, 720, 30u32, 90usize);
    let dir = std::env::temp_dir();
    let mut report = format!("цель: {ow}x{oh}, {fps} к/с, {kbps} кбит/с\n");
    let mon = monitors::rect();
    let mut gpu = Dxgi::new(mon, || capture_gdi(mon.0, mon.1, mon.2, mon.3, mon.2, mon.3));
    report += &format!("захват: {}\n", if gpu.is_some() { "видеокарта" } else { "GDI" });
    let started = Instant::now();
    let soft = args.iter().any(|a| a == "--soft");
    let enc = H264::new(ow, oh, fps, kbps, !soft);
    report += &format!(
        "создание кодера: {:.0} мс, {}\n",
        started.elapsed().as_secs_f32() * 1000.0,
        enc.as_ref().map_or("НЕ УДАЛОСЬ".to_string(), |e| format!("{} ({})", e.name, if e.hardware() { "видеокарта" } else { "процессор" }))
    );
    if let Some(mut enc) = enc {
        let (mut cap_t, mut enc_t, mut stream, mut sizes) = (0.0f32, 0.0f32, Vec::new(), Vec::new());
        for i in 0..frames {
            let t0 = Instant::now();
            let shot = gpu
                .as_mut()
                .and_then(|g| g.grab(0, 0, mon.2, mon.3, ow, oh))
                .or_else(|| capture_gdi(mon.0, mon.1, mon.2, mon.3, ow, oh));
            cap_t += t0.elapsed().as_secs_f32();
            let t1 = Instant::now();
            if let Some(Some(d)) = shot.map(|b| enc.encode(&b, i == 0)) {
                sizes.push(d.len());
                stream.extend_from_slice(&d);
            }
            enc_t += t1.elapsed().as_secs_f32();
            std::thread::sleep(Duration::from_millis(1000 / fps as u64).saturating_sub(t0.elapsed()));
        }
        let n = sizes.len().max(1) as f32;
        report += &format!(
            "кадров получено {} из {frames}; съёмка {:.1} мс, конвертация+кодирование {:.1} мс на кадр\n",
            sizes.len(), cap_t * 1000.0 / frames as f32, enc_t * 1000.0 / frames as f32
        );
        report += &format!(
            "всего {} КБ, в среднем {:.0} КБ на кадр (первый {} КБ), {:.1} Мбит/с\n",
            stream.len() / 1024, stream.len() as f32 / n / 1024.0, sizes.first().copied().unwrap_or(0) / 1024,
            stream.len() as f32 * 8.0 / (frames as f32 / fps as f32) / 1e6
        );
        let mut types = std::collections::BTreeMap::new();
        for w in stream.windows(4) {
            if w[0] == 0 && w[1] == 0 && w[2] == 1 {
                *types.entry(w[3] & 0x1f).or_insert(0u32) += 1;
            }
        }
        report += &format!("типы NAL (7 SPS, 8 PPS, 5 ключевой, 1 обычный, 6 SEI, 9 AUD): {types:?}\n");
        let _ = std::fs::write(dir.join("nerd-h264-test.h264"), &stream);
    }
    let _ = std::fs::write(dir.join("nerd-h264-test.txt"), report);
}
