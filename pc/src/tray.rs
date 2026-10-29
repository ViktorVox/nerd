// Значок в трее: статус, выбор монитора, запуск вместе с Windows, выход
use crate::monitors;
use crate::settings;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW, GetCursorPos,
    GetMessageW, GetSystemMetrics, LoadImageW, PostMessageW, PostQuitMessage, RegisterClassW,
    RegisterWindowMessageW, SetForegroundWindow, TrackPopupMenu, TranslateMessage, HICON, IMAGE_ICON,
    LR_DEFAULTCOLOR, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MSG, SM_CXSMICON,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_LBUTTONUP, WM_NULL,
    WM_RBUTTONUP, WNDCLASSW,
};

const WM_TRAY: u32 = WM_APP + 1;
const WM_STATUS: u32 = WM_APP + 2;
const ID_AUTOSTART: usize = 900;
const ID_EXIT: usize = 902;
const ID_MONITOR: usize = 1000; // + номер монитора

static HWND_RAW: AtomicIsize = AtomicIsize::new(0);
static STATUS: Mutex<(String, Option<String>)> = Mutex::new((String::new(), None));
static TASKBAR_CREATED: Mutex<u32> = Mutex::new(0);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

// Вторую копию программы не запускаем
pub fn single_instance() -> bool {
    unsafe {
        // Описатель остаётся открытым до выхода из программы: на нём держится «вторая копия не нужна»
        let _ = CreateMutexW(None, false, w!("Local\\NerdPcSingleton"));
        GetLastError() != ERROR_ALREADY_EXISTS
    }
}

// Меняет подпись значка; balloon — всплывающее уведомление
pub fn set_status(text: &str, balloon: Option<String>) {
    *STATUS.lock().unwrap() = (text.to_string(), balloon);
    let h = HWND_RAW.load(Ordering::SeqCst);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_STATUS, WPARAM(0), LPARAM(0));
        }
    }
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
    let (status, _) = STATUS.lock().unwrap().clone();
    let mut d = NOTIFYICONDATAW::default();
    d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    d.hWnd = hwnd;
    d.uID = 1;
    d.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    d.uCallbackMessage = WM_TRAY;
    d.hIcon = icon();
    copy(&mut d.szTip, &format!("Nerd — {}", if status.is_empty() { "ждёт телефон" } else { &status }));
    if let Some(text) = balloon {
        d.uFlags |= NIF_INFO;
        copy(&mut d.szInfoTitle, "Nerd");
        copy(&mut d.szInfo, text);
        d.dwInfoFlags = NIIF_INFO;
    }
    let _ = Shell_NotifyIconW(op, &d);
}

unsafe fn show_menu(hwnd: HWND) {
    let menu = CreatePopupMenu().unwrap();
    let (status, _) = STATUS.lock().unwrap().clone();
    let title = format!("Nerd — {}", if status.is_empty() { "ждёт телефон" } else { &status });
    let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, PCWSTR(wide(&title).as_ptr()));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());

    let sub = CreatePopupMenu().unwrap();
    let selected = monitors::selected();
    for m in monitors::list() {
        let label = format!(
            "Монитор {} — {}×{}{}",
            m.num,
            m.rect.2,
            m.rect.3,
            if m.primary { " (основной в Windows)" } else { "" }
        );
        let mut flags = MF_STRING;
        if selected == Some(m.num) {
            flags |= MF_CHECKED;
        }
        let _ = AppendMenuW(sub, flags, ID_MONITOR + m.num as usize, PCWSTR(wide(&label).as_ptr()));
    }
    let _ = AppendMenuW(menu, MF_POPUP | MF_STRING, sub.0 as usize, w!("Монитор для телефона"));

    let mut auto = MF_STRING;
    if settings::autostart() {
        auto |= MF_CHECKED;
    }
    let _ = AppendMenuW(menu, auto, ID_AUTOSTART, w!("Запускать вместе с Windows"));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, ID_EXIT, w!("Выход"));

    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let _ = SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_RETURNCMD, pt.x, pt.y, Some(0), hwnd, None).0 as usize;
    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(menu);

    match cmd {
        0 => {}
        ID_AUTOSTART => settings::set_autostart(!settings::autostart()),
        ID_EXIT => {
            notify(hwnd, NIM_DELETE, None);
            std::process::exit(0);
        }
        c if c > ID_MONITOR => {
            monitors::select(Some((c - ID_MONITOR) as u32));
            settings::save(&settings::Settings { monitor: monitors::selected() });
        }
        _ => {}
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            let event = (lp.0 & 0xFFFF) as u32;
            if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
                show_menu(hwnd);
            }
            LRESULT(0)
        }
        WM_STATUS => {
            let (_, balloon) = STATUS.lock().unwrap().clone();
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
