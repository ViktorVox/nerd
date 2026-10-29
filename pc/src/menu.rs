// Меню значка в трее в стиле приложения Nerd: тёмный фон, мягкий синий акцент.
// Окно рисуется целиком вручную: фигуры со сглаживанием, текст через GDI (ClearType).
use crate::{auth, monitors, settings, tray};
use std::cell::RefCell;
use std::ffi::c_void;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW,
    EndPaint, GetMonitorInfoW, InvalidateRect, MonitorFromPoint, SelectObject, SetBkMode, SetTextColor,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    DIB_RGB_COLORS, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, HBITMAP, HDC,
    HFONT, HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTONEAREST, OUT_DEFAULT_PRECIS, PAINTSTRUCT, SRCCOPY,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, RegisterClassW, SetForegroundWindow,
    SetWindowPos, ShowWindow, CS_DROPSHADOW, HWND_TOPMOST, SWP_NOACTIVATE, SW_SHOW, WA_INACTIVE, WM_ACTIVATE,
    WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

// Палитра Nerd (как в приложении на телефоне)
const BG: u32 = 0x15171D;
const BORDER: u32 = 0x2A2D3A;
const HOVER: u32 = 0x232739;
const HOVER_DANGER: u32 = 0x2B1C21;
const TEXT: u32 = 0xE6E8EE;
const MUTED: u32 = 0x8B91A3;
const FAINT: u32 = 0x666C7F;
const ACCENT: u32 = 0x8FA8FF;
const ACCENT_INK: u32 = 0x0B1330;
const OK: u32 = 0x4ADE80;
const DANGER: u32 = 0xFF7B82;
const OFF_TRACK: u32 = 0x2E3140;

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

struct Canvas {
    w: i32,
    h: i32,
    bits: *mut u32,
    dc: HDC,
    bmp: HBITMAP,
    fonts: [HFONT; 4], // обычный, мелкий, заголовок, подпись раздела
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

// ---------- рисование ----------

fn blend(px: &mut u32, rgb: u32, a: f32) {
    let d = *px;
    let mix = |s: u32, d: u32| (s as f32 * a + d as f32 * (1.0 - a)) as u32;
    let r = mix((rgb >> 16) & 255, (d >> 16) & 255);
    let g = mix((rgb >> 8) & 255, (d >> 8) & 255);
    let b = mix(rgb & 255, d & 255);
    *px = 0xFF00_0000 | (r << 16) | (g << 8) | b;
}

impl Canvas {
    // Заливает область по знаковому расстоянию до фигуры (отрицательное — внутри): края получаются сглаженными
    fn paint(&mut self, bbox: (f32, f32, f32, f32), color: impl Fn(f32, f32) -> u32, sdf: impl Fn(f32, f32) -> f32) {
        let x0 = (bbox.0.floor() as i32).max(0);
        let y0 = (bbox.1.floor() as i32).max(0);
        let x1 = (bbox.2.ceil() as i32).min(self.w);
        let y1 = (bbox.3.ceil() as i32).min(self.h);
        for y in y0..y1 {
            for x in x0..x1 {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let a = (0.5 - sdf(fx, fy)).clamp(0.0, 1.0);
                if a > 0.0 {
                    let p = unsafe { &mut *self.bits.add((y * self.w + x) as usize) };
                    blend(p, color(fx, fy), a);
                }
            }
        }
    }

    fn rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32) {
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        self.paint(
            (x - 1.0, y - 1.0, x + w + 1.0, y + h + 1.0),
            |_, _| color,
            |px, py| rrect_sdf(px - cx, py - cy, w / 2.0, h / 2.0, r),
        );
    }

    fn rrect_border(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, width: f32, color: u32) {
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        self.paint(
            (x - 1.0, y - 1.0, x + w + 1.0, y + h + 1.0),
            |_, _| color,
            |px, py| rrect_sdf(px - cx, py - cy, w / 2.0, h / 2.0, r).abs() - width / 2.0,
        );
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: u32) {
        self.paint(
            (cx - r - 1.0, cy - r - 1.0, cx + r + 1.0, cy + r + 1.0),
            |_, _| color,
            |px, py| (px - cx).hypot(py - cy) - r,
        );
    }

    fn ring(&mut self, cx: f32, cy: f32, r: f32, width: f32, color: u32) {
        self.paint(
            (cx - r - 2.0, cy - r - 2.0, cx + r + 2.0, cy + r + 2.0),
            |_, _| color,
            |px, py| ((px - cx).hypot(py - cy) - r).abs() - width / 2.0,
        );
    }

    fn text(&mut self, s: &str, font: usize, color: u32, x: f32, y: f32, w: f32, h: f32, right: bool) {
        if s.is_empty() {
            return;
        }
        unsafe {
            SelectObject(self.dc, HGDIOBJ(self.fonts[font].0));
            SetTextColor(self.dc, COLORREF(((color & 255) << 16) | (color & 0xFF00) | ((color >> 16) & 255)));
            let mut rect = RECT { left: x as i32, top: y as i32, right: (x + w) as i32, bottom: (y + h) as i32 };
            let mut text: Vec<u16> = s.encode_utf16().collect();
            let align = if right { DT_RIGHT } else { DT_LEFT };
            DrawTextW(self.dc, &mut text, &mut rect, align | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS);
        }
    }
}

fn rrect_sdf(px: f32, py: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let (qx, qy) = (px.abs() - (hw - r), py.abs() - (hh - r));
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

fn seg_dist(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((px - a.0) * dx + (py - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (px - (a.0 + t * dx)).hypot(py - (a.1 + t * dy))
}

// Иконка Nerd: тёмный скруглённый квадрат, буква N одним штрихом пера, белая точка
fn draw_logo(c: &mut Canvas, x: f32, y: f32, size: f32) {
    let k = size / 108.0;
    let (cx, cy) = (x + size / 2.0, y + size / 2.0);
    c.paint(
        (x - 1.0, y - 1.0, x + size + 1.0, y + size + 1.0),
        |px, py| {
            let t = (((px - x) + (py - y)) / (2.0 * size)).clamp(0.0, 1.0);
            let lerp = |a: f32, b: f32| (a + (b - a) * t) as u32;
            (lerp(27.0, 11.0) << 16) | (lerp(31.0, 12.0) << 8) | lerp(51.0, 17.0)
        },
        |px, py| rrect_sdf(px - cx, py - cy, size / 2.0, size / 2.0, size * 0.22),
    );
    // рисунок чуть крупнее, чем на иконке Android
    let zoom = 1.28;
    let p = |vx: f32, vy: f32| (cx + (vx - 54.0) * k * zoom, cy + (vy - 54.0) * k * zoom);
    let pts = [p(37.0, 76.0), p(37.0, 34.0), p(71.0, 74.0), p(71.0, 32.0)];
    let half = 4.5 * k * zoom;
    c.paint(
        (x, y, x + size, y + size),
        |_, _| ACCENT,
        |px, py| (0..3).map(|i| seg_dist(px, py, pts[i], pts[i + 1])).fold(f32::MAX, f32::min) - half,
    );
    let dot = p(71.0, 26.0);
    c.circle(dot.0, dot.1, 4.0 * k * zoom, 0xFFFFFF);
}

// ---------- содержимое ----------

fn build_items(connected: bool) -> Vec<Item> {
    let mut items = vec![Item::new(Kind::Header, "", "", Action::None), Item::new(Kind::Separator, "", "", Action::None)];
    items.push(Item::new(Kind::Label, "МОНИТОР ДЛЯ ТЕЛЕФОНА", "", Action::None));
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
        "Забыть все телефоны",
        &if n > 0 { n.to_string() } else { String::new() },
        Action::ForgetPhones,
    );
    forget.enabled = n > 0;
    items.push(forget);
    items.push(Item::new(Kind::Separator, "", "", Action::None));
    items.push(Item::new(Kind::Plain { danger: true }, "Выход", "", Action::Exit));
    let _ = connected;
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
    unsafe {
        std::slice::from_raw_parts_mut(c.bits, (c.w * c.h) as usize).fill(0xFF00_0000 | BG);
    }
    let (w, h) = (c.w as f32, c.h as f32);
    c.rrect_border(0.5, 0.5, w - 1.0, h - 1.0, 8.0 * s, 1.0, BORDER);

    for (i, it) in st.items.iter().enumerate() {
        let (x, y, ih) = (PAD * s, it.y * s, it.h * s);
        let iw = w - 2.0 * PAD * s;
        let hot = st.hover == Some(i) && it.clickable();
        match &it.kind {
            Kind::Header => {
                draw_logo(c, x + 8.0 * s, y + 10.0 * s, 40.0 * s);
                c.text("Nerd", 2, TEXT, x + 58.0 * s, y + 8.0 * s, 150.0 * s, 24.0 * s, false);
                let dot = if st.connected { OK } else { FAINT };
                c.circle(x + 62.0 * s, y + 41.0 * s, 3.5 * s, dot);
                c.text(&st.status, 1, MUTED, x + 72.0 * s, y + 32.0 * s, iw - 80.0 * s, 18.0 * s, false);
            }
            Kind::Separator => {
                c.rrect(x + 6.0 * s, y + ih / 2.0, iw - 12.0 * s, 1.0_f32.max(s * 0.8), 0.0, BORDER);
            }
            Kind::Label => {
                c.text(&it.text, 3, FAINT, x + 12.0 * s, y + 4.0 * s, iw - 24.0 * s, ih - 4.0 * s, false);
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
                c.text(&it.text, 0, TEXT, x + 42.0 * s, y, iw - 120.0 * s, ih, false);
                c.text(&it.right, 1, FAINT, x + iw - 80.0 * s, y, 68.0 * s, ih, true);
            }
            Kind::Toggle(on) => {
                if hot {
                    c.rrect(x, y, iw, ih, 10.0 * s, HOVER);
                }
                c.text(&it.text, 0, TEXT, x + 14.0 * s, y, iw - 80.0 * s, ih, false);
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
                c.text(&it.text, 0, color, x + 14.0 * s, y, iw - 80.0 * s, ih, false);
                c.text(&it.right, 1, FAINT, x + iw - 80.0 * s, y, 68.0 * s, ih, true);
            }
        }
    }
}

// ---------- окно ----------

fn item_at(st: &State, y: i32) -> Option<usize> {
    let fy = y as f32 / st.scale;
    st.items.iter().position(|it| fy >= it.y && fy < it.y + it.h && it.clickable())
}

fn close(hwnd: HWND) {
    unsafe {
        let _ = DestroyWindow(hwnd);
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
                    let _ = BitBlt(hdc, 0, 0, st.canvas.w, st.canvas.h, Some(st.canvas.dc), 0, 0, SRCCOPY);
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
                        render(st);
                        let _ = InvalidateRect(Some(hwnd), None, false);
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
                        render(st);
                        let _ = InvalidateRect(Some(hwnd), None, false);
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
                        render(st);
                        let _ = InvalidateRect(Some(hwnd), None, false);
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
            STATE.with(|s| {
                if let Some(st) = s.borrow_mut().take() {
                    unsafe {
                        let _ = DeleteDC(st.canvas.dc);
                        let _ = DeleteObject(HGDIOBJ(st.canvas.bmp.0));
                        for f in st.canvas.fonts {
                            let _ = DeleteObject(HGDIOBJ(f.0));
                        }
                    }
                }
            });
            OPEN.with(|o| *o.borrow_mut() = 0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn font(height: f32, weight: i32) -> HFONT {
    unsafe {
        CreateFontW(
            -(height.round() as i32),
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        )
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
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let scale = dx as f32 / 96.0;
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let work = mi.rcWork;

        let items = build_items(connected);
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

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let Ok(bmp) = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else { return };
        let dc = CreateCompatibleDC(None);
        SelectObject(dc, HGDIOBJ(bmp.0));
        SetBkMode(dc, TRANSPARENT);

        let canvas = Canvas {
            w,
            h,
            bits: bits as *mut u32,
            dc,
            bmp,
            fonts: [font(14.0 * scale, 400), font(12.0 * scale, 400), font(17.0 * scale, 600), font(11.0 * scale, 600)],
        };
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

        // Windows 11: скруглённые углы и цвет рамки в тон меню; на Windows 10 вызовы просто не сработают
        let round = 2u32;
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(33), &round as *const _ as *const c_void, 4);
        let border = ((BORDER & 255) << 16) | (BORDER & 0xFF00) | ((BORDER >> 16) & 255);
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(34), &border as *const _ as *const c_void, 4);
        let dark = 1u32;
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &dark as *const _ as *const c_void, 4);

        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = PCWSTR::null();
    }
}
