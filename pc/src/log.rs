// Журнал в %APPDATA%\Nerd\nerd.log (у программы нет окна консоли); при каждом запуске начинается заново
use std::fs::File;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static FILE: OnceLock<Mutex<(File, Instant)>> = OnceLock::new();

pub fn init() {
    let dir = crate::settings::dir();
    if let Ok(f) = File::create(dir.join("nerd.log")) {
        let _ = FILE.set(Mutex::new((f, Instant::now())));
    }
}

pub fn line(text: &str) {
    if let Some(m) = FILE.get() {
        if let Ok(mut g) = m.lock() {
            let t = g.1.elapsed().as_secs_f32();
            let _ = writeln!(g.0, "[{t:8.1}] {text}");
        }
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::log::line(&format!($($arg)*)) };
}
