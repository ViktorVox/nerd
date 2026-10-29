// Общие средства для окон Nerd (меню в трее, запрос на подключение): палитра, рисование фигур
// со сглаживанием и текст через GDI (ClearType). Окна рисуются в память и выводятся целиком.
use std::ffi::c_void;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, RECT};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW, GetMonitorInfoW,
    MonitorFromPoint, SelectObject, SetBkMode, SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT,
    DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, HBITMAP, HDC, HFONT, HGDIOBJ, MONITORINFO,
    MONITOR_DEFAULTTONEAREST, OUT_DEFAULT_PRECIS, SRCCOPY, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::Foundation::POINT;

// Палитра Nerd (как в приложении на устройстве)
pub const BG: u32 = 0x15171D;
pub const CARD: u32 = 0x1B1E27;
pub const BORDER: u32 = 0x2A2D3A;
pub const HOVER: u32 = 0x232739;
pub const HOVER_DANGER: u32 = 0x2B1C21;
pub const TEXT: u32 = 0xE6E8EE;
pub const MUTED: u32 = 0x8B91A3;
pub const FAINT: u32 = 0x666C7F;
pub const ACCENT: u32 = 0x8FA8FF;
pub const ACCENT_HOVER: u32 = 0xA9BBFF;
pub const ACCENT_INK: u32 = 0x0B1330;
pub const OK: u32 = 0x4ADE80;
pub const DANGER: u32 = 0xFF7B82;
pub const OFF_TRACK: u32 = 0x2E3140;

// Номера шрифтов в Canvas::fonts
pub const F_NORMAL: usize = 0;
pub const F_SMALL: usize = 1;
pub const F_TITLE: usize = 2;
pub const F_SECTION: usize = 3;
pub const F_BOLD: usize = 4;

#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Right,
    Center,
}

pub struct Canvas {
    pub w: i32,
    pub h: i32,
    bits: *mut u32,
    pub dc: HDC,
    bmp: HBITMAP,
    fonts: [HFONT; 5],
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.dc);
            let _ = DeleteObject(HGDIOBJ(self.bmp.0));
            for f in self.fonts {
                let _ = DeleteObject(HGDIOBJ(f.0));
            }
        }
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

fn blend(px: &mut u32, rgb: u32, a: f32) {
    let d = *px;
    let mix = |s: u32, d: u32| (s as f32 * a + d as f32 * (1.0 - a)) as u32;
    let r = mix((rgb >> 16) & 255, (d >> 16) & 255);
    let g = mix((rgb >> 8) & 255, (d >> 8) & 255);
    let b = mix(rgb & 255, d & 255);
    *px = 0xFF00_0000 | (r << 16) | (g << 8) | b;
}

pub fn rrect_sdf(px: f32, py: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let (qx, qy) = (px.abs() - (hw - r), py.abs() - (hh - r));
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

fn seg_dist(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((px - a.0) * dx + (py - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (px - (a.0 + t * dx)).hypot(py - (a.1 + t * dy))
}

impl Canvas {
    pub fn new(w: i32, h: i32, scale: f32) -> Option<Self> {
        unsafe {
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
            let bmp = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
            let dc = CreateCompatibleDC(None);
            SelectObject(dc, HGDIOBJ(bmp.0));
            SetBkMode(dc, TRANSPARENT);
            Some(Self {
                w,
                h,
                bits: bits as *mut u32,
                dc,
                bmp,
                fonts: [
                    font(14.0 * scale, 400),
                    font(12.0 * scale, 400),
                    font(17.0 * scale, 600),
                    font(11.0 * scale, 600),
                    font(14.0 * scale, 600),
                ],
            })
        }
    }

    pub fn fill(&mut self, rgb: u32) {
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize).fill(0xFF00_0000 | rgb) }
    }

    pub fn blit(&self, hdc: HDC) {
        unsafe {
            let _ = BitBlt(hdc, 0, 0, self.w, self.h, Some(self.dc), 0, 0, SRCCOPY);
        }
    }

    // Заливает область по знаковому расстоянию до фигуры (отрицательное - внутри): края получаются сглаженными
    pub fn paint(&mut self, bbox: (f32, f32, f32, f32), color: impl Fn(f32, f32) -> u32, sdf: impl Fn(f32, f32) -> f32) {
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

    pub fn rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32) {
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        self.paint(
            (x - 1.0, y - 1.0, x + w + 1.0, y + h + 1.0),
            |_, _| color,
            |px, py| rrect_sdf(px - cx, py - cy, w / 2.0, h / 2.0, r),
        );
    }

    pub fn rrect_border(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, width: f32, color: u32) {
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        self.paint(
            (x - 1.0, y - 1.0, x + w + 1.0, y + h + 1.0),
            |_, _| color,
            |px, py| rrect_sdf(px - cx, py - cy, w / 2.0, h / 2.0, r).abs() - width / 2.0,
        );
    }

    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, color: u32) {
        self.paint(
            (cx - r - 1.0, cy - r - 1.0, cx + r + 1.0, cy + r + 1.0),
            |_, _| color,
            |px, py| (px - cx).hypot(py - cy) - r,
        );
    }

    pub fn ring(&mut self, cx: f32, cy: f32, r: f32, width: f32, color: u32) {
        self.paint(
            (cx - r - 2.0, cy - r - 2.0, cx + r + 2.0, cy + r + 2.0),
            |_, _| color,
            |px, py| ((px - cx).hypot(py - cy) - r).abs() - width / 2.0,
        );
    }

    // Одна строка в прямоугольнике, по вертикали по центру
    pub fn text(&mut self, s: &str, font: usize, color: u32, x: f32, y: f32, w: f32, h: f32, align: Align) {
        let a = match align {
            Align::Left => DT_LEFT,
            Align::Right => DT_RIGHT,
            Align::Center => DT_CENTER,
        };
        self.draw_text(s, font, color, (x, y, w, h), a | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
    }

    // Несколько строк с переносом по словам, от верхнего края
    pub fn paragraph(&mut self, s: &str, font: usize, color: u32, x: f32, y: f32, w: f32, h: f32) {
        self.draw_text(s, font, color, (x, y, w, h), DT_LEFT | DT_WORDBREAK);
    }

    fn draw_text(&mut self, s: &str, font: usize, color: u32, r: (f32, f32, f32, f32), flags: windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT) {
        if s.is_empty() {
            return;
        }
        unsafe {
            SelectObject(self.dc, HGDIOBJ(self.fonts[font].0));
            SetTextColor(self.dc, COLORREF(bgr(color)));
            let mut rect = RECT { left: r.0 as i32, top: r.1 as i32, right: (r.0 + r.2) as i32, bottom: (r.1 + r.3) as i32 };
            let mut text: Vec<u16> = s.encode_utf16().collect();
            DrawTextW(self.dc, &mut text, &mut rect, flags | DT_NOPREFIX);
        }
    }

    // Иконка Nerd: тёмный скруглённый квадрат, буква N одним штрихом пера, белая точка
    pub fn logo(&mut self, x: f32, y: f32, size: f32) {
        let k = size / 108.0;
        let (cx, cy) = (x + size / 2.0, y + size / 2.0);
        self.paint(
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
        self.paint(
            (x, y, x + size, y + size),
            |_, _| ACCENT,
            |px, py| (0..3).map(|i| seg_dist(px, py, pts[i], pts[i + 1])).fold(f32::MAX, f32::min) - half,
        );
        let dot = p(71.0, 26.0);
        self.circle(dot.0, dot.1, 4.0 * k * zoom, 0xFFFFFF);
    }
}

pub fn bgr(rgb: u32) -> u32 {
    ((rgb & 255) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 255)
}

// Масштаб (1.0 = 96 dpi) и рабочая область монитора, на котором сейчас находится точка
pub fn monitor_at(pt: POINT) -> (f32, RECT) {
    unsafe {
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        (dx as f32 / 96.0, mi.rcWork)
    }
}

// Windows 11: скруглённые углы, рамка и тёмная тема окна в тон Nerd; на Windows 10 вызовы просто не сработают
pub fn style_window(hwnd: HWND) {
    unsafe {
        let round = 2u32;
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(33), &round as *const _ as *const c_void, 4);
        let border = bgr(BORDER);
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(34), &border as *const _ as *const c_void, 4);
        let dark = 1u32;
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &dark as *const _ as *const c_void, 4);
    }
}
