// Меню значка в трее в стиле приложения Nerd: тёмный фон, мягкий синий акцент.
use crate::ui::*;
use crate::{auth, monitors, settings, tray};
use std::cell::RefCell;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, RegisterClassW, SetForegroundWindow,
    SetWindowPos, ShowWindow, CS_DROPSHADOW, HWND_TOPMOST, SWP_NOACTIVATE, SW_SHOW, WA_INACTIVE, WM_ACTIVATE,
    WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WNDCLASSW, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

const WM_MOUSELEAVE: u32 = 0x02A3;
const WIDTH: f32 = 316.0;
const PAD: f32 = 8.0;

#[derive(Clone, Copy, PartialEq)]
pub enum Action {
    None,
    SelectMonitor(u32),
    ToggleAutostart,
    ForgetPhones,
    Exit,
}

#[derive(Clone)]
enum Kind {
    Header,
    Separator,
    Label,
    Radio(bool),
    Toggle(bool),
    Plain { danger: bool },
}

#[derive(Clone)]
struct Item {
    kind: Kind,
    text: String,
    right: String,
    action: Action,
    enabled: bool,
    y: f32,
    h: f32,
}

impl Item {
    fn new(kind: Kind, text: &str, right: &str, action: Action) -> Self {
        let h = match kind {
            Kind::Header => 62.0,
            Kind::Separator => 9.0,
            Kind::Label => 28.0,
            Kind::Radio(_) => 36.0,
            Kind::Toggle(_) | Kind::Plain { .. } => 38.0,
        };
        Self { kind, text: text.into(), right: right.into(), action, enabled: true, y: 0.0, h }
    }

    fn clickable(&self) -> bool {
        self.enabled && !matches!(self.kind, Kind::Header | Kind::Separator | Kind::Label)
    }
}

struct State {
    items: Vec<Item>,
    hover: Option<usize>,
    scale: f32,
    canvas: Canvas,
    status: String,
    connected: bool,
    tracking: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static OPEN: RefCell<isize> = const { RefCell::new(0) };
}

fn build_items() -> Vec<Item> {
    let mut items = vec![Item::new(Kind::Header, "", "", Action::None), Item::new(Kind::Separator, "", "", Action::None)];
    items.push(Item::new(Kind::Label, "МОНИТОР ДЛЯ УСТРОЙСТВА", "", Action::None));
    let selected = monitors::selected();
    for m in monitors::list() {
        let tag = if m.primary { "основной" } else { "" };
        items.push(Item::new(
            Kind::Radio(selected == Some(m.num)),
            &format!("Монитор {}  ·  {}×{}", m.num, m.rect.2, m.rect.3),
            tag,
            Action::SelectMonitor(m.num),
        ));
    }
    items.push(Item::new(Kind::Separator, "", "", Action::None));
    items.push(Item::new(Kind::Toggle(settings::autostart()), "Запускать вместе с Windows", "", Action::ToggleAutostart));
    let n = auth::count();
    let mut forget = Item::new(
        Kind::Plain { danger: false },
        "Забыть все устройства",
        &if n > 0 { n.to_string() } else { String::new() },
        Action::ForgetPhones,
    );
    forget.enabled = n > 0;
    items.push(forget);
    items.push(Item::new(Kind::Separator, "", "", Action::None));
    items.push(Item::new(Kind::Plain { danger: true }, "Выход", "", Action::Exit));
    let mut y = PAD;
    for it in &mut items {
        it.y = y;
        y += it.h;
    }
    items
}

fn render(st: &mut State) {
    let s = st.scale;
    let c = &mut st.canvas;
    c.fill(BG);
    let (w, h) = (c.w as f32, c.h as f32);
    c.rrect_border(0.5, 0.5, w - 1.0, h - 1.0, 8.0 * s, 1.0, BORDER);

    for (i, it) in st.items.iter().enumerate() {
        let (x, y, ih) = (PAD * s, it.y * s, it.h * s);
        let iw = w - 2.0 * PAD * s;
        let hot = st.hover == Some(i) && it.clickable();
        match &it.kind {
            Kind::Header => {
                c.logo(x + 8.0 * s, y + 10.0 * s, 40.0 * s);
                c.text("Nerd", F_TITLE, TEXT, x + 58.0 * s, y + 8.0 * s, 150.0 * s, 24.0 * s, Align::Left);
                let dot = if st.connected { OK } else { FAINT };
                c.circle(x + 62.0 * s, y + 41.0 * s, 3.5 * s, dot);
                c.text(&st.status, F_SMALL, MUTED, x + 72.0 * s, y + 32.0 * s, iw - 80.0 * s, 18.0 * s, Align::Left);
            }
            Kind::Separator => {
                c.rrect(x + 6.0 * s, y + ih / 2.0, iw - 12.0 * s, 1.0_f32.max(s * 0.8), 0.0, BORDER);
            }
            Kind::Label => {
                c.text(&it.text, F_SECTION, FAINT, x + 12.0 * s, y + 4.0 * s, iw - 24.0 * s, ih - 4.0 * s, Align::Left);
            }
            Kind::Radio(on) => {
                if hot {
                    c.rrect(x, y, iw, ih, 10.0 * s, HOVER);
                }
                let (cx, cy) = (x + 22.0 * s, y + ih / 2.0);
                if *on {
                    c.ring(cx, cy, 7.0 * s, 1.8 * s, ACCENT);
                    c.circle(cx, cy, 3.6 * s, ACCENT);
                } else {
                    c.ring(cx, cy, 7.0 * s, 1.5 * s, FAINT);
                }
                c.text(&it.text, F_NORMAL, TEXT, x + 42.0 * s, y, iw - 120.0 * s, ih, Align::Left);
                c.text(&it.right, F_SMALL, FAINT, x + iw - 80.0 * s, y, 68.0 * s, ih, Align::Right);
            }
            Kind::Toggle(on) => {
                if hot {
                    c.rrect(x, y, iw, ih, 10.0 * s, HOVER);
                }
                c.text(&it.text, F_NORMAL, TEXT, x + 14.0 * s, y, iw - 80.0 * s, ih, Align::Left);
                let (tw, th) = (36.0 * s, 20.0 * s);
                let (tx, ty) = (x + iw - 12.0 * s - tw, y + (ih - th) / 2.0);
                c.rrect(tx, ty, tw, th, th / 2.0, if *on { ACCENT } else { OFF_TRACK });
                let kx = if *on { tx + tw - th / 2.0 } else { tx + th / 2.0 };
                c.circle(kx, ty + th / 2.0, 7.0 * s, if *on { ACCENT_INK } else { MUTED });
            }
            Kind::Plain { danger } => {
                if hot {
                    c.rrect(x, y, iw, ih, 10.0 * s, if *danger { HOVER_DANGER } else { HOVER });
                }
                let color = if !it.enabled {
                    FAINT
                } else if *danger {
                    DANGER
                } else {
                    TEXT
                };
                c.text(&it.text, F_NORMAL, color, x + 14.0 * s, y, iw - 80.0 * s, ih, Align::Left);
                c.text(&it.right, F_SMALL, FAINT, x + iw - 80.0 * s, y, 68.0 * s, ih, Align::Right);
            }
        }
    }
}

fn item_at(st: &State, y: i32) -> Option<usize> {
    let fy = y as f32 / st.scale;
    st.items.iter().position(|it| fy >= it.y && fy < it.y + it.h && it.clickable())
}

fn close(hwnd: HWND) {
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

fn redraw(hwnd: HWND, st: &mut State) {
    render(st);
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            STATE.with(|s| {
                if let Some(st) = s.borrow().as_ref() {
                    st.canvas.blit(hdc);
                }
            });
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let y = ((lp.0 >> 16) & 0xFFFF) as i16 as i32;
            STATE.with(|s| {
                if let Some(st) = s.borrow_mut().as_mut() {
                    if !st.tracking {
                        let mut t = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut t);
                        st.tracking = true;
                    }
                    let now = item_at(st, y);
                    if now != st.hover {
                        st.hover = now;
                        redraw(hwnd, st);
                    }
                }
            });
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            STATE.with(|s| {
                if let Some(st) = s.borrow_mut().as_mut() {
                    st.tracking = false;
                    if st.hover.is_some() {
                        st.hover = None;
                        redraw(hwnd, st);
                    }
                }
            });
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let y = ((lp.0 >> 16) & 0xFFFF) as i16 as i32;
            let action = STATE.with(|s| s.borrow().as_ref().and_then(|st| item_at(st, y).map(|i| st.items[i].action)));
            if let Some(a) = action {
                close(hwnd);
                tray::run_action(a);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let key = wp.0 as u16;
            if key == VK_ESCAPE.0 {
                close(hwnd);
            } else if key == VK_DOWN.0 || key == VK_UP.0 {
                STATE.with(|s| {
                    if let Some(st) = s.borrow_mut().as_mut() {
                        let n = st.items.len();
                        let start = st.hover.map_or(if key == VK_DOWN.0 { n - 1 } else { 0 }, |h| h);
                        for step in 1..=n {
                            let i = if key == VK_DOWN.0 { (start + step) % n } else { (start + n - step) % n };
                            if st.items[i].clickable() {
                                st.hover = Some(i);
                                break;
                            }
                        }
                        redraw(hwnd, st);
                    }
                });
            } else if key == VK_RETURN.0 {
                let action = STATE.with(|s| s.borrow().as_ref().and_then(|st| st.hover.map(|i| st.items[i].action)));
                if let Some(a) = action {
                    close(hwnd);
                    tray::run_action(a);
                }
            }
            LRESULT(0)
        }
        WM_ACTIVATE => {
            // клик мимо меню или переключение окна закрывает его
            if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE {
                close(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            STATE.with(|s| *s.borrow_mut() = None);
            OPEN.with(|o| *o.borrow_mut() = 0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// Показывает меню у курсора (там, где щёлкнули по значку)
pub fn show(connected: bool, status: &str) {
    if OPEN.with(|o| *o.borrow()) != 0 {
        return;
    }
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let (scale, work) = monitor_at(pt);

        let items = build_items();
        let content: f32 = items.iter().map(|i| i.h).sum::<f32>() + 2.0 * PAD;
        let (w, h) = ((WIDTH * scale).round() as i32, (content * scale).round() as i32);

        // под значком, если панель задач сверху; над значком, если снизу
        let mut x = pt.x - w / 2;
        let mut y = if pt.y < (work.top + work.bottom) / 2 { pt.y + (10.0 * scale) as i32 } else { pt.y - h - (10.0 * scale) as i32 };
        x = x.clamp(work.left + 8, (work.right - w - 8).max(work.left + 8));
        y = y.clamp(work.top + 8, (work.bottom - h - 8).max(work.top + 8));

        let inst = GetModuleHandleW(None).unwrap();
        let cls = w!("NerdMenu");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: inst.into(),
            lpszClassName: cls,
            style: CS_DROPSHADOW,
            ..Default::default()
        };
        RegisterClassW(&wc);

        let Some(canvas) = Canvas::new(w, h, scale) else { return };
        let mut st = State { items, hover: None, scale, canvas, status: status.to_string(), connected, tracking: false };
        render(&mut st);
        STATE.with(|s| *s.borrow_mut() = Some(st));

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            cls,
            w!("Nerd"),
            WS_POPUP,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(inst.into()),
            None,
        )
        .unwrap();
        OPEN.with(|o| *o.borrow_mut() = hwnd.0 as isize);
        style_window(hwnd);
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}
