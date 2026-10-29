use std::net::UdpSocket;
use std::time::Duration;
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Controls::{
    CreateSyntheticPointerDevice, HSYNTHETICPOINTERDEVICE,
    POINTER_FEEDBACK_DEFAULT, POINTER_TYPE_INFO, POINTER_TYPE_INFO_0,
};
use windows::Win32::UI::Input::Pointer::{
    POINTER_FLAG_DOWN, POINTER_FLAG_FIRSTBUTTON, POINTER_FLAG_INCONTACT, POINTER_FLAG_INRANGE,
    POINTER_FLAG_NEW, POINTER_FLAG_UP, POINTER_FLAG_UPDATE, POINTER_INFO, POINTER_PEN_INFO,
    InjectSyntheticPointerInput,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, PEN_MASK_PRESSURE, PEN_MASK_TILT_X, PEN_MASK_TILT_Y, PT_PEN, SM_CXSCREEN,
    SM_CYSCREEN,
};

const PORT: u16 = 47800;
const MAGIC: u8 = 0x4E;
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

struct Pen {
    device: HSYNTHETICPOINTERDEVICE,
    in_range: bool,
    w: i32,
    h: i32,
}

impl Pen {
    fn new() -> windows::core::Result<Self> {
        let device = unsafe { CreateSyntheticPointerDevice(PT_PEN, 1, POINTER_FEEDBACK_DEFAULT)? };
        let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        Ok(Self { device, in_range: false, w, h })
    }

    fn inject(&mut self, p: &Packet) {
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

        let x = (p.x.clamp(0.0, 1.0) * (self.w - 1) as f32) as i32;
        let y = (p.y.clamp(0.0, 1.0) * (self.h - 1) as f32) as i32;
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
fn demo(pen: &mut Pen) {
    let pk = |action, x: f32, y: f32, pressure| Packet { action, x, y, pressure, tilt_x: 0, tilt_y: 0 };
    let pt = |t: f32| (0.5 + 0.15 * t.cos(), 0.5 + 0.2 * t.sin());
    let step = Duration::from_millis(8);
    let (x, y) = pt(0.0);
    pen.inject(&pk(HOVER, x, y, 0.0));
    std::thread::sleep(Duration::from_millis(300));
    pen.inject(&pk(DOWN, x, y, 0.5));
    for i in 1..=300 {
        let t = i as f32 / 300.0 * std::f32::consts::TAU;
        let (x, y) = pt(t);
        // давление плавно растёт и падает
        pen.inject(&pk(DRAG, x, y, 0.1 + 0.9 * (t / 2.0).sin().abs()));
        std::thread::sleep(step);
    }
    let (x, y) = pt(std::f32::consts::TAU);
    pen.inject(&pk(UP, x, y, 0.0));
    pen.inject(&pk(LEAVE, x, y, 0.0));
}

fn main() {
    let mut pen = Pen::new().expect("не удалось создать виртуальное перо (нужен Windows 10 1809+)");

    if std::env::args().any(|a| a == "--demo") {
        println!("демо: через 3 секунды перо нарисует круг. Открой Paint или Krita.");
        std::thread::sleep(Duration::from_secs(3));
        demo(&mut pen);
        return;
    }

    let sock = UdpSocket::bind(("0.0.0.0", PORT)).expect("порт занят");
    println!("Слушаю UDP :{PORT}. Экран {}x{}", pen.w, pen.h);
    let mut buf = [0u8; 64];
    loop {
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => {
                if let Some(p) = parse(&buf[..n]) {
                    pen.inject(&p);
                }
            }
            Err(e) => eprintln!("recv error: {e}"),
        }
    }
}
