// Настройки в %APPDATA%\Nerd и запуск вместе с Windows
use std::path::PathBuf;
use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

pub fn dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let dir = base.join("Nerd");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub struct Settings {
    pub monitor: Option<u32>,
}

pub fn load() -> Settings {
    let text = std::fs::read_to_string(dir().join("settings.ini")).unwrap_or_default();
    let monitor = text
        .lines()
        .find_map(|l| l.strip_prefix("monitor="))
        .and_then(|v| v.trim().parse().ok());
    Settings { monitor }
}

pub fn save(s: &Settings) {
    let text = match s.monitor {
        Some(n) => format!("monitor={n}\n"),
        None => String::new(),
    };
    let _ = std::fs::write(dir().join("settings.ini"), text);
}

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

pub fn autostart() -> bool {
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Nerd"), RRF_RT_REG_SZ, None, None, None).is_ok()
    }
}

pub fn set_autostart(on: bool) {
    unsafe {
        if on {
            let Ok(exe) = std::env::current_exe() else { return };
            let value: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain(Some(0)).collect();
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                w!("Nerd"),
                REG_SZ.0,
                Some(value.as_ptr() as *const _),
                (value.len() * 2) as u32,
            );
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Nerd"));
        }
    }
}
