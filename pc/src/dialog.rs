// Окно «Разрешить подключение устройства?» в стиле Nerd
use crate::ui::*;
use std::cell::RefCell;
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, KillTimer,
    PostQuitMessage, RegisterClassW, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TranslateMessage,
    CS_DROPSHADOW, HTCAPTION, HTCLIENT, HWND_TOPMOST, MSG, SWP_NOSIZE, SW_SHOW, WM_DESTROY, WM_ERASEBKGND,
    WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCHITTEST, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

const WM_MOUSELEAVE: u32 = 0x02A3;
const W: f32 = 440.0;
const H: f32 = 272.0;
const TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone, Copy, PartialEq)]
enum Btn {
    Allow,
    Deny,
}

struct Dialog {
    canvas: Canvas,
    scale: f32,
    name: String,
    ip: String,
    hover: Option<Btn>,
    started: Instant,
    allowed: bool,
    tracking: bool,
}

thread_local! {
    static DIALOG: RefCell<Option<Dialog>> = const { RefCell::new(None) };
}

// Прямоугольник кнопки в логических пикселях: x, y, ширина, высота
fn rect(b: Btn) -> (f32, f32, f32, f32) {
    match b {
        Btn::Allow => (W - 24.0 - 140.0, H - 24.0 - 42.0, 140.0, 42.0),
        Btn::Deny => (W - 24.0 - 140.0 - 12.0 - 124.0, H - 24.0 - 42.0, 124.0, 42.0),
    }
}

fn button_at(d: &Dialog, x: i32, y: i32) -> Option<Btn> {
    let (fx, fy) = (x as f32 / d.scale, y as f32 / d.scale);
    [Btn::Allow, Btn::Deny].into_iter().find(|&b| {
        let r = rect(b);
        fx >= r.0 && fx < r.0 + r.2 && fy >= r.1 && fy < r.1 + r.3
    })
}

fn render(d: &mut Dialog) {
    let s = d.scale;
    let hover = d.hover;
    let left = TIMEOUT.saturating_sub(d.started.elapsed()).as_secs_f32() / TIMEOUT.as_secs_f32();
    let c = &mut d.canvas;
    c.fill(BG);
    let (w, h) = (c.w as f32, c.h as f32);
    c.rrect_border(0.5, 0.5, w - 1.0, h - 1.0, 8.0 * s, 1.0, BORDER);

    c.logo(24.0 * s, 24.0 * s, 46.0 * s);
    c.text("Разрешить подключение?", F_TITLE, TEXT, 84.0 * s, 24.0 * s, (W - 108.0) * s, 26.0 * s, Align::Left);
    c.text(
        "Устройство хочет видеть экран и управлять ПК",
        F_SMALL,
        MUTED,
        84.0 * s,
        50.0 * s,
        (W - 108.0) * s,
        20.0 * s,
        Align::Left,
    );

    // карточка устройства
    let (cx, cy, cw, ch) = (24.0 * s, 88.0 * s, (W - 48.0) * s, 64.0 * s);
    c.rrect(cx, cy, cw, ch, 12.0 * s, CARD);
    c.rrect_border(cx + 0.5, cy + 0.5, cw - 1.0, ch - 1.0, 12.0 * s, 1.0, BORDER);
    c.circle(cx + 32.0 * s, cy + 32.0 * s, 20.0 * s, 0x242A45);
    c.rrect_border(cx + 24.5 * s, cy + 21.0 * s, 15.0 * s, 22.0 * s, 4.0 * s, 1.8 * s, ACCENT);
    c.circle(cx + 32.0 * s, cy + 38.5 * s, 1.1 * s, ACCENT);
    c.text(&d.name, F_BOLD, TEXT, cx + 66.0 * s, cy + 10.0 * s, cw - 80.0 * s, 24.0 * s, Align::Left);
    c.text(&d.ip, F_SMALL, MUTED, cx + 66.0 * s, cy + 34.0 * s, cw - 80.0 * s, 20.0 * s, Align::Left);

    c.paragraph(
        "Разрешайте, только если это ваше устройство. Без ответа запрос отклонится сам.",
        F_SMALL,
        FAINT,
        24.0 * s,
        166.0 * s,
        (W - 48.0) * s,
        34.0 * s,
    );

    for b in [Btn::Deny, Btn::Allow] {
        let (x, y, bw, bh) = rect(b);
        let hot = hover == Some(b);
        let (bg, fg, label) = match b {
            Btn::Allow => (if hot { ACCENT_HOVER } else { ACCENT }, ACCENT_INK, "Разрешить"),
            Btn::Deny => (if hot { 0x2B3050 } else { HOVER }, TEXT, "Отклонить"),
        };
        c.rrect(x * s, y * s, bw * s, bh * s, 12.0 * s, bg);
        let font = if b == Btn::Allow { F_BOLD } else { F_NORMAL };
        c.text(label, font, fg, x * s, y * s, bw * s, bh * s, Align::Center);
    }

    // оставшееся время: тонкая полоска вдоль верхнего края
    c.rrect(14.0 * s, 4.0 * s, (w - 28.0 * s) * left, 2.5 * s, 1.2 * s, 0x3B4575);
}

fn finish(hwnd: HWND, allowed: bool) {
    DIALOG.with(|d| {
        if let Some(d) = d.borrow_mut().as_mut() {
            d.allowed = allowed;
        }
    });
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

fn redraw(hwnd: HWND, d: &mut Dialog) {
    render(d);
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let xy = || (((lp.0 & 0xFFFF) as i16) as i32, (((lp.0 >> 16) & 0xFFFF) as i16) as i32);
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            DIALOG.with(|d| {
                if let Some(d) = d.borrow().as_ref() {
                    d.canvas.blit(hdc);
                }
            });
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        // Окно можно перетаскивать за любое место, кроме кнопок
        WM_NCHITTEST => {
            let mut pt = POINT { x: (lp.0 & 0xFFFF) as i16 as i32, y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32 };
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut pt);
            let on_button = DIALOG.with(|d| d.borrow().as_ref().is_some_and(|d| button_at(d, pt.x, pt.y).is_some()));
            LRESULT(if on_button { HTCLIENT } else { HTCAPTION } as isize)
        }
        WM_MOUSEMOVE => {
            let (x, y) = xy();
            DIALOG.with(|d| {
                if let Some(d) = d.borrow_mut().as_mut() {
                    if !d.tracking {
                        let mut t = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut t);
                        d.tracking = true;
                    }
                    let now = button_at(d, x, y);
                    if now != d.hover {
                        d.hover = now;
                        redraw(hwnd, d);
                    }
                }
            });
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            DIALOG.with(|d| {
                if let Some(d) = d.borrow_mut().as_mut() {
                    d.tracking = false;
                    d.hover = None;
                    redraw(hwnd, d);
                }
            });
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = xy();
            match DIALOG.with(|d| d.borrow().as_ref().and_then(|d| button_at(d, x, y))) {
                Some(Btn::Allow) => finish(hwnd, true),
                Some(Btn::Deny) => finish(hwnd, false),
                None => {}
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            // Enter намеренно ничего не разрешает: подтверждение только осознанным нажатием
            if wp.0 as u16 == VK_ESCAPE.0 {
                finish(hwnd, false);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            let expired = DIALOG.with(|d| {
                let mut d = d.borrow_mut();
                let Some(d) = d.as_mut() else { return false };
                if d.started.elapsed() >= TIMEOUT {
                    return true;
                }
                redraw(hwnd, d);
                false
            });
            if expired {
                finish(hwnd, false);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), 1);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// Показывает окно и ждёт ответ; true - разрешено. Вызывать из потока, который может ждать (не из окна трея)
pub fn ask(name: &str, ip: &str) -> bool {
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let (scale, work) = monitor_at(pt);
        let (w, h) = ((W * scale).round() as i32, (H * scale).round() as i32);
        let x = work.left + (work.right - work.left - w) / 2;
        let y = work.top + (work.bottom - work.top - h) / 2;

        let inst = GetModuleHandleW(None).unwrap();
        let cls = w!("NerdDialog");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: inst.into(),
            lpszClassName: cls,
            style: CS_DROPSHADOW,
            ..Default::default()
        };
        RegisterClassW(&wc);

        let Some(canvas) = Canvas::new(w, h, scale) else { return false };
        let mut d = Dialog {
            canvas,
            scale,
            name: name.to_string(),
            ip: ip.to_string(),
            hover: None,
            started: Instant::now(),
            allowed: false,
            tracking: false,
        };
        render(&mut d);
        DIALOG.with(|s| *s.borrow_mut() = Some(d));

        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            cls,
            w!("Nerd - запрос на подключение"),
            WS_POPUP,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(inst.into()),
            None,
        ) else {
            return false;
        };
        style_window(hwnd);
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOSIZE);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        SetTimer(Some(hwnd), 1, 250, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        DIALOG.with(|s| s.borrow_mut().take().is_some_and(|d| d.allowed))
    }
}
