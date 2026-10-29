// Значок в трее: подпись со статусом, уведомления, открытие меню (menu.rs)
use crate::menu::Action;
use crate::{auth, menu, monitors, settings};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetSystemMetrics, LoadImageW, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, TranslateMessage, HICON, IMAGE_ICON,
    LR_DEFAULTCOLOR, MSG, SM_CXSMICON, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_LBUTTONUP, WM_RBUTTONUP,
    WNDCLASSW,
};

const WM_TRAY: u32 = WM_APP + 1;
const WM_STATUS: u32 = WM_APP + 2;

struct Status {
    connected: bool,
    text: String,
    balloon: Option<String>,
}

static HWND_RAW: AtomicIsize = AtomicIsize::new(0);
static STATUS: Mutex<Status> = Mutex::new(Status { connected: false, text: String::new(), balloon: None });
static TASKBAR_CREATED: Mutex<u32> = Mutex::new(0);

// Вторую копию программы не запускаем
pub fn single_instance() -> bool {
    unsafe {
        // Описатель остаётся открытым до выхода из программы: на нём держится «вторая копия не нужна»
        let _ = CreateMutexW(None, false, w!("Local\\NerdPcSingleton"));
        GetLastError() != ERROR_ALREADY_EXISTS
    }
}

// Меняет подпись значка; balloon — всплывающее уведомление
pub fn set_status(connected: bool, text: &str, balloon: Option<String>) {
    *STATUS.lock().unwrap() = Status { connected, text: text.to_string(), balloon };
    let h = HWND_RAW.load(Ordering::SeqCst);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_STATUS, WPARAM(0), LPARAM(0));
        }
    }
}

fn status_text() -> String {
    let s = STATUS.lock().unwrap();
    if s.text.is_empty() { "Ждёт телефон".into() } else { s.text.clone() }
}

fn copy(dst: &mut [u16], s: &str) {
    for (i, c) in s.encode_utf16().take(dst.len() - 1).enumerate() {
        dst[i] = c;
    }
}

unsafe fn icon() -> HICON {
    let inst = GetModuleHandleW(None).unwrap();
    let size = GetSystemMetrics(SM_CXSMICON);
    LoadImageW(Some(inst.into()), PCWSTR(1 as *const u16), IMAGE_ICON, size, size, LR_DEFAULTCOLOR)
        .map(|h| HICON(h.0))
        .unwrap_or_default()
}

unsafe fn notify(hwnd: HWND, op: windows::Win32::UI::Shell::NOTIFY_ICON_MESSAGE, balloon: Option<&str>) {
    let mut d = NOTIFYICONDATAW::default();
    d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    d.hWnd = hwnd;
    d.uID = 1;
    d.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    d.uCallbackMessage = WM_TRAY;
    d.hIcon = icon();
    copy(&mut d.szTip, &format!("Nerd — {}", status_text()));
    if let Some(text) = balloon {
        d.uFlags |= NIF_INFO;
        copy(&mut d.szInfoTitle, "Nerd");
        copy(&mut d.szInfo, text);
        d.dwInfoFlags = NIIF_INFO;
    }
    let _ = Shell_NotifyIconW(op, &d);
}

// Что сделать после выбора пункта меню
pub fn run_action(action: Action) {
    match action {
        Action::None => {}
        Action::SelectMonitor(n) => {
            monitors::select(Some(n));
            settings::save(&settings::Settings { monitor: monitors::selected() });
        }
        Action::ToggleAutostart => settings::set_autostart(!settings::autostart()),
        Action::ForgetPhones => {
            auth::forget_all();
            set_status(false, "Ждёт телефон", Some("Все телефоны забыты: при подключении снова спросим разрешение".into()));
        }
        Action::Exit => {
            let h = HWND_RAW.load(Ordering::SeqCst);
            if h != 0 {
                unsafe { notify(HWND(h as *mut _), NIM_DELETE, None) };
            }
            std::process::exit(0);
        }
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            let event = (lp.0 & 0xFFFF) as u32;
            if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
                let connected = STATUS.lock().unwrap().connected;
                menu::show(connected, &status_text());
            }
            LRESULT(0)
        }
        WM_STATUS => {
            let balloon = STATUS.lock().unwrap().balloon.take();
            notify(hwnd, NIM_MODIFY, balloon.as_deref());
            LRESULT(0)
        }
        m if m != 0 && m == *TASKBAR_CREATED.lock().unwrap() => {
            // Проводник перезапустился: значок нужно добавить заново
            notify(hwnd, NIM_ADD, None);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// Создаёт скрытое окно и значок, дальше обрабатывает сообщения до выхода
pub fn run() {
    unsafe {
        *TASKBAR_CREATED.lock().unwrap() = RegisterWindowMessageW(w!("TaskbarCreated"));
        let inst = GetModuleHandleW(None).unwrap();
        let cls = w!("NerdTray");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: inst.into(),
            lpszClassName: cls,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            cls,
            w!("Nerd"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(inst.into()),
            None,
        )
        .unwrap();
        HWND_RAW.store(hwnd.0 as isize, Ordering::SeqCst);
        notify(hwnd, NIM_ADD, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        PostQuitMessage(0);
    }
}
