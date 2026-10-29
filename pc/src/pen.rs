// Виртуальное перо Windows и мышь
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Controls::{
    CreateSyntheticPointerDevice, HSYNTHETICPOINTERDEVICE, POINTER_FEEDBACK_DEFAULT, POINTER_TYPE_INFO,
    POINTER_TYPE_INFO_0,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
    MOUSE_EVENT_FLAGS,
};
use windows::Win32::UI::Input::Pointer::{
    InjectSyntheticPointerInput, POINTER_FLAG_DOWN, POINTER_FLAG_FIRSTBUTTON, POINTER_FLAG_INCONTACT,
    POINTER_FLAG_INRANGE, POINTER_FLAG_NEW, POINTER_FLAG_UP, POINTER_FLAG_UPDATE, POINTER_INFO,
    POINTER_PEN_INFO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, PEN_MASK_PRESSURE, PEN_MASK_TILT_X, PEN_MASK_TILT_Y, PT_PEN, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

pub const MAGIC: u8 = 0x4E;
pub const CMD_MAGIC: u8 = 0x4F;
const PACKET_LEN: usize = 20;

// Действия из пакета
pub const HOVER: u8 = 0; // перо над экраном
pub const DOWN: u8 = 1; // касание
pub const DRAG: u8 = 2; // движение с касанием
pub const UP: u8 = 3; // отрыв
pub const LEAVE: u8 = 4; // перо ушло из зоны

pub struct Packet {
    pub action: u8,
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
    pub tilt_x: i16,
    pub tilt_y: i16,
}

// Формат (20 байт, little-endian):
// magic u8 | action u8 | buttons u8 | pad u8 | x f32 | y f32 | pressure f32 | tilt_x i16 | tilt_y i16
pub fn parse(buf: &[u8]) -> Option<Packet> {
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

pub struct Pen {
    device: HSYNTHETICPOINTERDEVICE,
    in_range: bool,
    contact: bool,
}

// Доля выбранного монитора (0..1) в пиксели рабочего стола
pub fn pixel(x: f32, y: f32) -> (i32, i32) {
    let (x0, y0, w, h) = crate::monitors::rect();
    (
        x0 + (x.clamp(0.0, 1.0) * (w - 1) as f32) as i32,
        y0 + (y.clamp(0.0, 1.0) * (h - 1) as f32) as i32,
    )
}

impl Pen {
    pub fn new() -> windows::core::Result<Self> {
        let device = unsafe { CreateSyntheticPointerDevice(PT_PEN, 1, POINTER_FEEDBACK_DEFAULT)? };
        Ok(Self { device, in_range: false, contact: false })
    }

    pub fn inject(&mut self, p: &Packet) {
        let (x, y) = pixel(p.x, p.y);
        self.inject_px(p, x, y);
    }

    // Система считает координаты от левого верхнего угла всего рабочего стола
    pub fn inject_px(&mut self, p: &Packet, x: i32, y: i32) {
        let (x, y) = unsafe {
            (
                x - GetSystemMetrics(SM_XVIRTUALSCREEN),
                y - GetSystemMetrics(SM_YVIRTUALSCREEN),
            )
        };
        // Windows отвергает отрыв без касания и движение с касанием без начала: исправляем последовательность
        let action = match (p.action, self.contact) {
            (DOWN, true) => DRAG,
            (DRAG, false) => DOWN,
            (UP, false) => return,
            (a, _) => a,
        };
        self.contact = matches!(action, DOWN | DRAG);
        let mut flags = match action {
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
        if action != LEAVE && !self.in_range {
            flags |= POINTER_FLAG_NEW;
            self.in_range = true;
        }
        if action == LEAVE {
            self.in_range = false;
        }

        let touching = matches!(action, DOWN | DRAG);
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
            crate::log!("ошибка вставки пера: {e}");
        }
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
pub fn command(buf: &[u8]) {
    let f = |i: usize| f32::from_le_bytes(buf[i..i + 4].try_into().unwrap());
    match buf.get(1) {
        Some(0) if buf.len() >= 12 => {
            let (x, y) = pixel(f(4), f(8));
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
