//! Native per-user desktop application. No service, injection, elevation or installer.
use crate::{
    desktop_runtime::{Runtime, IDLE},
    independent_source::open_source,
    secret,
};
use std::{
    cell::RefCell,
    mem::{size_of, zeroed},
    os::windows::ffi::OsStrExt,
    ptr::{null, null_mut},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    Security::Credentials::*,
    System::{Diagnostics::ToolHelp::*, LibraryLoader::*, Registry::*, Threading::*},
    UI::{Controls::EM_SETLIMITTEXT, Shell::*, WindowsAndMessaging::*},
};
use zeroize::Zeroizing;

const TITLE: &str = "Deadlock Live Match Collector";
#[cfg(not(feature = "internal-test"))]
const ID: &str = "DeadlockLiveMatchCollector";
#[cfg(feature = "internal-test")]
const ID: &str = "DeadlockLiveMatchCollectorInternalTest";
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const TRAY: u32 = WM_APP + 1;
const SAVE: usize = 101;
const AUTO: usize = 102;
const EXIT: usize = 103;
const SHOW: usize = 104;
const FORGET: usize = 105;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn text(hwnd: HWND, s: &str) {
    unsafe {
        SetWindowTextW(hwnd, wide(s).as_ptr());
    }
}
fn alert(s: &str) {
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(s).as_ptr(),
            wide(TITLE).as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

fn detect() -> Result<Option<u32>, ()> {
    unsafe {
        let handle = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if handle == INVALID_HANDLE_VALUE {
            return Err(());
        }
        let mut entry: PROCESSENTRY32W = zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut found = None;
        let mut ok = Process32FirstW(handle, &mut entry);
        while ok != 0 {
            let n = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..n]).eq_ignore_ascii_case("deadlock.exe")
            {
                found = Some(entry.th32ProcessID);
                break;
            }
            ok = Process32NextW(handle, &mut entry);
        }
        let error = GetLastError();
        CloseHandle(handle);
        if found.is_some() || error == ERROR_NO_MORE_FILES {
            Ok(found)
        } else {
            Err(())
        }
    }
}
fn load_key() -> Result<Option<Zeroizing<String>>, ()> {
    unsafe {
        let mut ptr: *mut CREDENTIALW = null_mut();
        if CredReadW(wide(ID).as_ptr(), CRED_TYPE_GENERIC, 0, &mut ptr) == 0 {
            return if GetLastError() == ERROR_NOT_FOUND {
                Ok(None)
            } else {
                Err(())
            };
        }
        let c = &*ptr;
        let result = if c.CredentialBlobSize == 68 && !c.CredentialBlob.is_null() {
            let bytes = std::slice::from_raw_parts(c.CredentialBlob, 68);
            let result = secret::validate(bytes).map(Some).map_err(|_| ());
            // Credential Manager allocated this writable buffer.
            std::slice::from_raw_parts_mut(c.CredentialBlob, 68).fill(0);
            result
        } else {
            Err(())
        };
        CredFree(ptr.cast());
        result
    }
}
fn save_key(key: &str) -> Result<(), ()> {
    let target = wide(ID);
    let user = wide(TITLE);
    let mut bytes = Zeroizing::new(key.as_bytes().to_vec());
    unsafe {
        let mut c: CREDENTIALW = zeroed();
        c.Type = CRED_TYPE_GENERIC;
        c.TargetName = target.as_ptr() as *mut _;
        c.UserName = user.as_ptr() as *mut _;
        c.CredentialBlobSize = bytes.len() as u32;
        c.CredentialBlob = bytes.as_mut_ptr();
        c.Persist = CRED_PERSIST_LOCAL_MACHINE;
        if CredWriteW(&c, 0) == 0 {
            Err(())
        } else {
            Ok(())
        }
    }
}
fn forget_key() -> Result<(), ()> {
    unsafe {
        if CredDeleteW(wide(ID).as_ptr(), CRED_TYPE_GENERIC, 0) != 0
            || GetLastError() == ERROR_NOT_FOUND
        {
            Ok(())
        } else {
            Err(())
        }
    }
}
fn auto_enabled() -> Result<bool, ()> {
    unsafe {
        let mut len = 0;
        match RegGetValueW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            wide(ID).as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            null_mut(),
            &mut len,
        ) {
            ERROR_SUCCESS => Ok(true),
            ERROR_FILE_NOT_FOUND => Ok(false),
            _ => Err(()),
        }
    }
}
fn set_auto(enable: bool) -> Result<(), ()> {
    unsafe {
        let rc = if enable {
            let exe = std::env::current_exe().map_err(|_| ())?;
            let mut command = vec![34u16];
            command.extend(exe.as_os_str().encode_wide());
            command.extend(wide("\" --autostart"));
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                wide(ID).as_ptr(),
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * 2) as u32,
            )
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(ID).as_ptr())
        };
        if rc == ERROR_SUCCESS || (!enable && rc == ERROR_FILE_NOT_FOUND) {
            Ok(())
        } else {
            Err(())
        }
    }
}
enum Command {
    Key(Option<Zeroizing<String>>),
    Stop,
}
fn worker(rx: mpsc::Receiver<Command>, status: Arc<Mutex<String>>, key: Option<Zeroizing<String>>) {
    let mut runtime = Runtime::new(key);
    loop {
        let next = runtime
            .tick(detect(), Instant::now(), open_source)
            .to_owned();
        if let Ok(mut shared) = status.lock() {
            *shared = next;
        }
        match rx.recv_timeout(IDLE) {
            Ok(Command::Key(key)) => runtime.set_token(key),
            Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        // Drain queued commands before another poll.
        while let Ok(command) = rx.try_recv() {
            match command {
                Command::Key(key) => runtime.set_token(key),
                Command::Stop => return,
            }
        }
    }
}

struct Ui {
    edit: HWND,
    status: HWND,
    notice: HWND,
    auto: HWND,
    tx: mpsc::Sender<Command>,
    shared: Arc<Mutex<String>>,
    last: String,
    tray: NOTIFYICONDATAW,
    taskbar: u32,
    tray_available: bool,
}
thread_local! { static UI: RefCell<Option<Ui>> = const { RefCell::new(None) }; }
fn copy_tip(tray: &mut NOTIFYICONDATAW, s: &str) {
    tray.szTip.fill(0);
    for (d, c) in tray.szTip.iter_mut().take(127).zip(s.encode_utf16()) {
        *d = c;
    }
}
fn add_tray(tray: &mut NOTIFYICONDATAW) -> bool {
    unsafe { Shell_NotifyIconW(NIM_ADD, tray) != 0 }
}
fn show(hwnd: HWND) {
    unsafe {
        ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
    }
}
fn control(
    parent: HWND,
    class: &str,
    caption: &str,
    style: u32,
    id: usize,
    bounds: (i32, i32, i32, i32),
) -> HWND {
    let (x, y, w, h) = bounds;
    unsafe {
        let hwnd = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(caption).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            x,
            y,
            w,
            h,
            parent,
            id as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        if !hwnd.is_null() {
            SendMessageW(
                hwnd,
                WM_SETFONT,
                GetStockObject(DEFAULT_GUI_FONT) as usize,
                1,
            );
        }
        hwnd
    }
}
fn refresh_auto(ui: &Ui) {
    text(
        ui.auto,
        match auto_enabled() {
            Ok(true) => "Autostart deaktivieren",
            Ok(false) => "Autostart aktivieren",
            Err(()) => "Autostart prüfen / aktivieren",
        },
    );
}
fn click(hwnd: HWND, id: usize) {
    if id == EXIT {
        unsafe {
            DestroyWindow(hwnd);
        }
        return;
    }
    if id == SHOW {
        show(hwnd);
        return;
    }
    UI.with(|cell| {
        let Ok(mut state) = cell.try_borrow_mut() else {
            return;
        };
        let Some(ui) = state.as_mut() else {
            return;
        };
        match id {
            SAVE => {
                let mut buffer = Zeroizing::new([0u16; 129]);
                let count = unsafe { GetWindowTextW(ui.edit, buffer.as_mut_ptr(), 129) } as usize;
                let raw = Zeroizing::new(String::from_utf16_lossy(&buffer[..count]));
                text(ui.edit, "");
                let result = (if count >= 128 {
                    Err("Der Verbindungsschlüssel ist zu lang.")
                } else {
                    secret::validate(raw.as_bytes())
                })
                .and_then(|key| {
                    save_key(&key).map_err(|_| {
                        "Speichern im Windows-Anmeldeinformationsmanager fehlgeschlagen."
                    })?;
                    ui.tx
                        .send(Command::Key(Some(key)))
                        .map_err(|_| "Hintergrundbetrieb beendet. Bitte neu starten.")?;
                    Ok(())
                });
                text(
                    ui.notice,
                    result
                        .err()
                        .unwrap_or("Verbindungsschlüssel geschützt gespeichert."),
                );
            }
            FORGET => {
                text(ui.edit, "");
                if forget_key().is_ok() && ui.tx.send(Command::Key(None)).is_ok() {
                    text(
                        ui.notice,
                        "Verbindungsschlüssel entfernt. Übertragung gestoppt.",
                    );
                } else {
                    text(
                        ui.notice,
                        "Schlüssel konnte nicht entfernt werden. Bitte erneut versuchen.",
                    );
                }
            }
            AUTO => {
                match auto_enabled().and_then(|enabled| set_auto(!enabled)) {
                    Ok(()) => text(
                        ui.notice,
                        if auto_enabled() == Ok(true) {
                            "Autostart aktiviert. EXE am aktuellen Ort belassen."
                        } else {
                            "Autostart deaktiviert."
                        },
                    ),
                    Err(()) => text(ui.notice, "Autostart konnte nicht geändert werden."),
                }
                refresh_auto(ui);
            }
            _ => {}
        }
    });
}
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND if (w >> 16) & 0xffff == 0 => {
            click(hwnd, w & 0xffff);
            0
        }
        WM_CLOSE => {
            let available = UI.with(|cell| {
                cell.try_borrow()
                    .ok()
                    .and_then(|state| state.as_ref().map(|ui| ui.tray_available))
                    .unwrap_or(false)
            });
            if available {
                ShowWindow(hwnd, SW_HIDE);
            } else {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_DESTROY => {
            UI.with(|cell| {
                if let Ok(mut state) = cell.try_borrow_mut() {
                    if let Some(ui) = state.as_mut() {
                        Shell_NotifyIconW(NIM_DELETE, &ui.tray);
                        let _ = ui.tx.send(Command::Stop);
                    }
                }
            });
            PostQuitMessage(0);
            0
        }
        WM_QUERYENDSESSION => 1,
        WM_ENDSESSION if w != 0 => {
            DestroyWindow(hwnd);
            0
        }
        WM_TIMER => {
            UI.with(|cell| {
                let Ok(mut state) = cell.try_borrow_mut() else {
                    return;
                };
                if let Some(ui) = state.as_mut() {
                    let status = ui
                        .shared
                        .lock()
                        .map(|s| s.clone())
                        .unwrap_or_else(|_| "Hintergrundfehler.".into());
                    if status != ui.last {
                        text(ui.status, &status);
                        copy_tip(&mut ui.tray, &status);
                        ui.tray.uFlags = NIF_TIP;
                        if Shell_NotifyIconW(NIM_MODIFY, &ui.tray) == 0 {
                            ui.tray_available = false;
                        }
                        ui.last = status;
                    }
                    if !ui.tray_available {
                        ui.tray.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
                        ui.tray_available = add_tray(&mut ui.tray);
                        if !ui.tray_available {
                            ShowWindow(hwnd, SW_SHOW);
                        }
                    }
                }
            });
            0
        }
        TRAY => {
            match l as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => show(hwnd),
                WM_RBUTTONUP => {
                    let menu = CreatePopupMenu();
                    if !menu.is_null() {
                        AppendMenuW(
                            menu,
                            MF_STRING,
                            SHOW,
                            wide("Status und Einstellungen").as_ptr(),
                        );
                        AppendMenuW(menu, MF_SEPARATOR, 0, null());
                        AppendMenuW(menu, MF_STRING, EXIT, wide("Beenden").as_ptr());
                        let mut pos: POINT = zeroed();
                        GetCursorPos(&mut pos);
                        SetForegroundWindow(hwnd);
                        let cmd = TrackPopupMenu(
                            menu,
                            TPM_RETURNCMD | TPM_RIGHTBUTTON,
                            pos.x,
                            pos.y,
                            0,
                            hwnd,
                            null(),
                        );
                        DestroyMenu(menu);
                        PostMessageW(hwnd, WM_NULL, 0, 0);
                        if cmd != 0 {
                            click(hwnd, cmd as usize);
                        }
                    }
                }
                _ => {}
            };
            0
        }
        _ => {
            let recovered = UI.with(|cell| {
                let Ok(mut state) = cell.try_borrow_mut() else {
                    return false;
                };
                if let Some(ui) = state.as_mut() {
                    if msg == ui.taskbar && msg != 0 {
                        ui.tray.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
                        ui.tray_available = add_tray(&mut ui.tray);
                        if !ui.tray_available {
                            ShowWindow(hwnd, SW_SHOW);
                        }
                        return true;
                    }
                }
                false
            });
            if recovered {
                0
            } else {
                DefWindowProcW(hwnd, msg, w, l)
            }
        }
    }
}
pub fn run_windows() {
    std::panic::set_hook(Box::new(|_| {
        eprintln!("Unerwarteter Programmfehler im Collector.")
    }));
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let startup = args.len() == 1 && args[0] == "--autostart";
    if !args.is_empty() && !startup {
        alert("Dieses Programm akzeptiert keine weiteren Befehlszeilenargumente.");
        return;
    }
    unsafe {
        let mutex = CreateMutexW(null(), 0, wide(&format!("Local\\{ID}")).as_ptr());
        if mutex.is_null() {
            alert("Instanzschutz konnte nicht eingerichtet werden.");
            return;
        }
        let exists = GetLastError() == ERROR_ALREADY_EXISTS;
        if exists {
            CloseHandle(mutex);
            if !startup {
                alert("Collector läuft bereits. Öffne das Symbol im Windows-Infobereich.");
            }
            return;
        }
        let result = desktop(startup);
        CloseHandle(mutex);
        if let Err(reason) = result {
            alert(reason);
        }
    }
}
unsafe fn desktop(startup: bool) -> Result<(), &'static str> {
    let class = wide(ID);
    let instance = GetModuleHandleW(null());
    let icon = LoadIconW(null_mut(), IDI_APPLICATION);
    let mut wc: WNDCLASSW = zeroed();
    wc.lpfnWndProc = Some(wndproc);
    wc.hInstance = instance;
    wc.lpszClassName = class.as_ptr();
    wc.hIcon = icon;
    wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
    wc.hbrBackground = (COLOR_WINDOW + 1) as HBRUSH;
    if RegisterClassW(&wc) == 0 {
        return Err("Fensterklasse konnte nicht eingerichtet werden.");
    }
    let hwnd = CreateWindowExW(
        0,
        class.as_ptr(),
        wide(TITLE).as_ptr(),
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        570,
        320,
        null_mut(),
        null_mut(),
        instance,
        null(),
    );
    if hwnd.is_null() {
        return Err("Statusfenster konnte nicht erstellt werden.");
    }
    let status = control(
        hwnd,
        "STATIC",
        "Collector wird gestartet…",
        0,
        0,
        (20, 18, 515, 55),
    );
    let _ = control(
        hwnd,
        "STATIC",
        "Verbindungsschlüssel (aus dem Overlay-Baukasten):",
        0,
        0,
        (20, 80, 515, 20),
    );
    let edit = control(
        hwnd,
        "EDIT",
        "",
        ES_PASSWORD as u32 | ES_AUTOHSCROLL as u32 | WS_BORDER | WS_TABSTOP,
        0,
        (20, 105, 515, 25),
    );
    SendMessageW(edit, EM_SETLIMITTEXT, 128, 0);
    let save = control(
        hwnd,
        "BUTTON",
        "Schlüssel speichern",
        WS_TABSTOP,
        SAVE,
        (20, 140, 200, 30),
    );
    let forget = control(
        hwnd,
        "BUTTON",
        "Schlüssel entfernen",
        WS_TABSTOP,
        FORGET,
        (230, 140, 200, 30),
    );
    let auto = control(
        hwnd,
        "BUTTON",
        "Autostart",
        WS_TABSTOP,
        AUTO,
        (20, 180, 230, 30),
    );
    let exit = control(
        hwnd,
        "BUTTON",
        "Beenden",
        WS_TABSTOP,
        EXIT,
        (260, 180, 170, 30),
    );
    let notice = control(hwnd, "STATIC", "", 0, 0, (20, 220, 515, 48));
    if [status, edit, save, forget, auto, exit, notice]
        .iter()
        .any(|p| p.is_null())
    {
        DestroyWindow(hwnd);
        return Err("Einstellungen konnten nicht erstellt werden.");
    }
    let (key, notice_text) = match load_key() {
        Ok(Some(key)) => (
            Some(key),
            "Schlüssel geschützt gespeichert. Schließen minimiert in den Infobereich.",
        ),
        Ok(None) => (None, "Bitte Schlüssel eingeben. Autostart ist optional."),
        Err(()) => (
            None,
            "Gespeicherter Schlüssel nicht lesbar. Bitte erneut eingeben.",
        ),
    };
    let configured = key.is_some();
    text(notice, notice_text);
    let shared = Arc::new(Mutex::new("Warte auf Deadlock.".into()));
    let (tx, rx) = mpsc::channel();
    let worker_status = shared.clone();
    let mut tray: NOTIFYICONDATAW = zeroed();
    tray.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    tray.hWnd = hwnd;
    tray.uID = 1;
    tray.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    tray.uCallbackMessage = TRAY;
    tray.hIcon = icon;
    copy_tip(&mut tray, TITLE);
    if !add_tray(&mut tray) {
        DestroyWindow(hwnd);
        return Err("Infobereich-Symbol konnte nicht erstellt werden. Bitte erneut starten.");
    }
    let ui = Ui {
        edit,
        status,
        notice,
        auto,
        tx,
        shared,
        last: String::new(),
        tray,
        taskbar: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
        tray_available: true,
    };
    refresh_auto(&ui);
    UI.with(|cell| *cell.borrow_mut() = Some(ui));
    let handle = thread::spawn(move || {
        let failure_status = worker_status.clone();
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            worker(rx, worker_status, key)
        }))
        .is_err()
        {
            if let Ok(mut status) = failure_status.lock() {
                *status =
                    "Hintergrundbetrieb unerwartet beendet. Bitte Collector neu starten.".into();
            }
        }
    });
    SetTimer(hwnd, 1, 1000, None);
    if !startup || !configured {
        show(hwnd);
    }
    let mut msg: MSG = zeroed();
    while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    KillTimer(hwnd, 1);
    UI.with(|cell| {
        if let Some(ui) = cell.borrow_mut().take() {
            let _ = ui.tx.send(Command::Stop);
            Shell_NotifyIconW(NIM_DELETE, &ui.tray);
        }
    });
    let _ = handle.join();
    Ok(())
}
