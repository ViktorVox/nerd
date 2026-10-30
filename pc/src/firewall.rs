// Правило брандмауэра Windows: входящие подключения к Nerd.exe во всех типах сети. Создаётся один раз с запросом прав
use crate::log;
use std::os::windows::process::CommandExt;
use std::process::Command;
use windows::core::HSTRING;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK, MB_TOPMOST, SW_HIDE};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const RULE: &str = "Nerd";

fn rule_exists(exe: &str) -> bool {
    let out = Command::new("netsh")
        .args(["advfirewall", "firewall", "show", "rule", &format!("name={RULE}"), "verbose"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let Ok(out) = out else { return false };
    if !out.status.success() {
        return false;
    }
    // вывод netsh в кодировке консоли: путь с русскими буквами не сверить, тогда хватает самого правила
    !exe.is_ascii() || String::from_utf8_lossy(&out.stdout).to_lowercase().contains(&exe.to_lowercase())
}

// Спрашиваем один раз на каждый путь к программе: отказ в окне прав повторно не беспокоит
pub fn ensure() {
    let Ok(exe) = std::env::current_exe() else { return };
    let exe = exe.display().to_string();
    if rule_exists(&exe) {
        return;
    }
    let marker = crate::settings::dir().join("firewall.asked");
    if std::fs::read_to_string(&marker).is_ok_and(|t| t.trim() == exe) {
        log!("правила брандмауэра нет, но уже спрашивали");
        return;
    }
    let _ = std::fs::write(&marker, &exe);
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(
                "Чтобы устройство могло подключиться к компьютеру, Nerd нужно разрешить входящие подключения в брандмауэре Windows.\n\nСейчас Windows один раз попросит права администратора. Нажмите «Да».",
            ),
            &HSTRING::from("Nerd"),
            MB_OK | MB_ICONINFORMATION | MB_TOPMOST,
        );
        let params = format!(
            "/c netsh advfirewall firewall delete rule name={RULE} & netsh advfirewall firewall add rule name={RULE} dir=in action=allow program=\"{exe}\" enable=yes profile=any"
        );
        let r = ShellExecuteW(None, &HSTRING::from("runas"), &HSTRING::from("cmd.exe"), &HSTRING::from(params), None, SW_HIDE);
        log!("правило брандмауэра: запрос прав отправлен, код {}", r.0 as isize);
    }
}
