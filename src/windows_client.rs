//! Native per-user desktop application. No service, injection, elevation or installer.
use crate::{
    desktop_runtime::{ConnectionProbe, Runtime, IDLE},
    independent_source::open_source,
    secret,
    window_layout::{centered, ink_baseline, scale},
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
    UI::{
        Controls::{DRAWITEMSTRUCT, EM_SETLIMITTEXT, EM_SETMARGINS},
        HiDpi::*,
        Shell::*,
        WindowsAndMessaging::*,
    },
};
use zeroize::Zeroizing;

const TITLE: &str = "Deadlock Live Collector";
const DASH: usize = 106;
const REPO: usize = 107;
const DASH_URL: &str = "https://deutsche-deadlock-community.de/twitch/live-collector";
const REPO_URL: &str = "https://github.com/EarlySalty/deadlock-live-match-collector";
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
    Key(u64, Option<Zeroizing<String>>),
    Stop,
}
fn worker(
    rx: mpsc::Receiver<Command>,
    status: Arc<Mutex<(u64, String)>>,
    key: Option<Zeroizing<String>>,
) {
    let mut generation = 0;
    let mut probe = ConnectionProbe::new();
    let mut token = key.clone();
    let mut runtime = Runtime::new(key);
    loop {
        let pid = detect();
        let game = runtime.tick(pid, Instant::now(), open_source).to_owned();
        let state = if pid.is_err() {
            "running"
        } else {
            runtime.connection_state()
        };
        let connection = probe.tick(token.as_deref().map(String::as_str), state);
        let backend = match connection {
            "Backend: kein Schlüssel gespeichert." => "Nicht eingerichtet",
            "Backend: verbunden · Schlüssel bestätigt." => "Verbunden · Schlüssel bestätigt",
            "Backend: Schlüssel abgelehnt. Bitte im Dashboard erneuern." => "Ungültiger Schlüssel",
            "Backend: API nicht mehr unterstützt. Bitte App aktualisieren." => {
                "API nicht unterstützt"
            }
            "Backend: Verbindung wird geprüft …" => "Verbindung wird hergestellt …",
            "Backend: nicht verbunden. Erneuter Versuch folgt." => "Server nicht erreichbar",
            _ => "Verbindung fehlgeschlagen",
        };
        let game = match pid {
            Err(()) => "Spielstatus nicht verfügbar",
            Ok(None) => "Wartet auf Deadlock",
            Ok(Some(_))
                if backend == "Verbunden · Schlüssel bestätigt"
                    && game.starts_with("Der Server hat Live-Daten bestätigt.") =>
            {
                "Live-Daten werden übertragen"
            }
            Ok(Some(_)) if state == "waiting_for_source" && token.is_some() => {
                "Wartet auf Spielquelle"
            }
            Ok(Some(_)) => "Deadlock läuft",
        };
        let next = format!("{backend}\r\n{game}");
        if let Ok(mut shared) = status.lock() {
            if shared.0 == generation {
                shared.1 = next;
            }
        }
        match rx.recv_timeout(IDLE) {
            Ok(Command::Key(next_generation, key)) => {
                generation = next_generation;
                token = key.clone();
                probe = ConnectionProbe::new();
                runtime.set_token(key);
            }
            Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        // Drain queued commands before another poll.
        while let Ok(command) = rx.try_recv() {
            match command {
                Command::Key(next_generation, key) => {
                    generation = next_generation;
                    token = key.clone();
                    probe = ConnectionProbe::new();
                    runtime.set_token(key);
                }
                Command::Stop => return,
            }
        }
    }
}

struct Ui {
    edit: HWND,
    status: HWND,
    game: HWND,
    save: HWND,
    notice: HWND,
    saved: HWND,
    auto: HWND,
    tx: mpsc::Sender<Command>,
    shared: Arc<Mutex<(u64, String)>>,
    last: String,
    generation: u64,
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
    unsafe {
        let dpi = GetDpiForWindow(parent).max(96);
        let r = scaled_rect(bounds, dpi);
        let hwnd = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(caption).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            parent,
            id as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        if !hwnd.is_null() {
            LAYOUT.with(|items| items.borrow_mut().push((hwnd, bounds, id)));
            apply_control_layout(hwnd, bounds, id, dpi);
        }
        hwnd
    }
}
const BG: u32 = 0x121010;
const CARD: u32 = 0x1c1c1c;
const GOLD: u32 = 0x5ea1c7;
const FG: u32 = 0xece9e5;
const MUTED: u32 = 0xbcb6ad;
const GREEN: u32 = 0xa9ce78;
const RED: u32 = 0x8b8bf0;
const FIELD: u32 = 0x292525;
const HEADING: usize = 201;
const BACKEND: usize = 202;
const GAME: usize = 203;
const FOOT: usize = 204;
const EDIT: usize = 205;
const EDIT_BOUNDS: (i32, i32, i32, i32) = (40, 305, 552, 29);
// Win32 STATIC style (not exported by windows-sys 0.61).
const SS_CENTERIMAGE: u32 = 0x0200;
const HEADER_TOP: i32 = 20;
const HEADER_BOTTOM: i32 = 76;
type LayoutItem = (HWND, (i32, i32, i32, i32), usize);
thread_local! {
    static LAYOUT: RefCell<Vec<LayoutItem>> = const { RefCell::new(Vec::new()) };
    static FONTS: RefCell<Vec<(usize,u32,HFONT)>> = const { RefCell::new(Vec::new()) };
}
fn font(id: usize, dpi: u32) -> HFONT {
    let kind = if id == HEADING {
        1
    } else if id == BACKEND || id == GAME {
        2
    } else {
        0
    };
    FONTS.with(|fonts| {
        let mut fonts = fonts.borrow_mut();
        if let Some((_, _, h)) = fonts.iter().find(|(k, d, _)| *k == kind && *d == dpi) {
            return *h;
        }
        let h = unsafe {
            CreateFontW(
                -scale([15, 26, 17][kind], dpi),
                0,
                0,
                0,
                if kind == 0 {
                    FW_NORMAL as i32
                } else {
                    FW_SEMIBOLD as i32
                },
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                DEFAULT_PITCH as u32,
                wide("Segoe UI Variable").as_ptr(),
            )
        };
        fonts.push((kind, dpi, h));
        h
    })
}
fn scaled_rect((x, y, w, h): (i32, i32, i32, i32), dpi: u32) -> RECT {
    RECT {
        left: scale(x, dpi),
        top: scale(y, dpi),
        right: scale(x + w, dpi),
        bottom: scale(y + h, dpi),
    }
}

unsafe fn apply_control_layout(child: HWND, bounds: (i32, i32, i32, i32), id: usize, dpi: u32) {
    let hfont = font(id, dpi);
    SendMessageW(child, WM_SETFONT, hfont as usize, 1);
    let mut r = scaled_rect(bounds, dpi);
    if id == EDIT {
        // Single-line EDIT ignores EM_SETRECT. Center its native text line inside
        // the painted field instead; preserve password masking and edit semantics.
        let dc = GetDC(child);
        if !dc.is_null() {
            let old = SelectObject(dc, hfont);
            let mut metrics: TEXTMETRICW = zeroed();
            if GetTextMetricsW(dc, &mut metrics) != 0 {
                let height = (metrics.tmHeight + scale(2, dpi)).min(r.bottom - r.top);
                (r.top, r.bottom) = centered(r.top, r.bottom, height);
            }
            SelectObject(dc, old);
            ReleaseDC(child, dc);
        }
        r.left += scale(4, dpi);
        r.right -= scale(4, dpi);
        let margin = scale(6, dpi) as isize;
        SendMessageW(child, EM_SETMARGINS, 3, margin | (margin << 16));
    }
    MoveWindow(child, r.left, r.top, r.right - r.left, r.bottom - r.top, 1);
}

/// Measure the visible glyph bounds of the actual label with the selected font.
/// A common ink center avoids a text-line-box/track mismatch across DPI settings.
unsafe fn center_button_label(dc: HDC, label: &[u16], r: &mut RECT) -> u32 {
    let flags = DT_CENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS;
    let mut tm: TEXTMETRICW = zeroed();
    if GetTextMetricsW(dc, &mut tm) == 0 {
        return flags | DT_VCENTER;
    }
    let one = FIXED { fract: 0, value: 1 };
    let zero = FIXED { fract: 0, value: 0 };
    let matrix = MAT2 {
        eM11: one,
        eM12: zero,
        eM21: zero,
        eM22: one,
    };
    let mut above = 0;
    let mut below = 0;
    for &ch in label {
        let mut glyph: GLYPHMETRICS = zeroed();
        if GetGlyphOutlineW(
            dc,
            ch as u32,
            GGO_METRICS,
            &mut glyph,
            0,
            null_mut(),
            &matrix,
        ) != GDI_ERROR as u32
            && glyph.gmBlackBoxY != 0
        {
            above = above.max(glyph.gmptGlyphOrigin.y);
            below = below.max(glyph.gmBlackBoxY as i32 - glyph.gmptGlyphOrigin.y);
        }
    }
    if above == 0 {
        above = (tm.tmAscent - tm.tmInternalLeading).max(1);
        below = tm.tmDescent;
    }
    let baseline = ink_baseline(r.top, r.bottom, above, below);
    r.top = (baseline - tm.tmAscent).max(r.top);
    flags
}

unsafe fn rounded(dc: HDC, r: RECT, color: u32, radius: i32) {
    let old_brush = SelectObject(dc, GetStockObject(DC_BRUSH));
    let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
    SetDCBrushColor(dc, color);
    RoundRect(dc, r.left, r.top, r.right, r.bottom, radius, radius);
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
}
unsafe fn draw_button(item: &DRAWITEMSTRUCT) {
    let id = item.CtlID as usize;
    let dpi = GetDpiForWindow(item.hwndItem).max(96);
    let r = item.rcItem;
    let active = item.itemState & 1 != 0;
    let color = if id == SAVE {
        if active {
            0x497e9d
        } else {
            GOLD
        }
    } else if active {
        0x49413a
    } else {
        0x332d29
    };
    let brush = GetStockObject(DC_BRUSH) as HBRUSH;
    SetDCBrushColor(item.hDC, if id == REPO || id == EXIT { BG } else { CARD });
    FillRect(item.hDC, &r, brush);
    rounded(item.hDC, r, color, 12 * dpi as i32 / 96);
    let mut label = [0u16; 128];
    let n = GetWindowTextW(item.hwndItem, label.as_mut_ptr(), 128);
    let old_font = SelectObject(item.hDC, font(id, dpi));
    SetBkMode(item.hDC, TRANSPARENT as i32);
    SetTextColor(item.hDC, if id == SAVE { BG } else { FG });
    let mut rect = r;
    if id == AUTO {
        let on = String::from_utf16_lossy(&label[..n.max(0) as usize]).ends_with("EIN");
        let sc = |v| scale(v, dpi);
        let (top, bottom) = centered(r.top, r.bottom, sc(20));
        let track = RECT {
            left: r.right - sc(48),
            right: r.right - sc(12),
            top,
            bottom,
        };
        rounded(item.hDC, track, if on { GOLD } else { 0x59534d }, sc(20));
        let diameter = sc(14);
        let (thumb_top, thumb_bottom) = centered(r.top, r.bottom, diameter);
        let x = if on {
            track.right - sc(3) - diameter
        } else {
            track.left + sc(3)
        };
        rounded(
            item.hDC,
            RECT {
                left: x,
                right: x + diameter,
                top: thumb_top,
                bottom: thumb_bottom,
            },
            if on { BG } else { FG },
            diameter,
        );
        rect.right -= sc(52);
    }
    let flags = center_button_label(item.hDC, &label[..n.max(0) as usize], &mut rect);
    DrawTextW(item.hDC, label.as_mut_ptr(), n, &mut rect, flags);
    SelectObject(item.hDC, old_font);
    if item.itemState & 0x10 != 0 {
        let mut focus = r;
        InflateRect(&mut focus, -4, -4);
        DrawFocusRect(item.hDC, &focus);
    }
}
fn open_link(hwnd: HWND, url: &str) {
    unsafe {
        if ShellExecuteW(
            hwnd,
            wide("open").as_ptr(),
            wide(url).as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        ) as isize
            <= 32
        {
            alert("Der Browser konnte nicht geöffnet werden.");
        }
    }
}
fn show_status(ui: &Ui, status: &str) {
    let (backend, game) = status
        .split_once("\r\n")
        .unwrap_or((status, "Spielstatus wird geprüft …"));
    text(
        ui.status,
        backend.strip_prefix("Backend: ").unwrap_or(backend),
    );
    text(ui.game, game);
    unsafe {
        InvalidateRect(ui.status, null(), 1);
        InvalidateRect(ui.game, null(), 1);
    }
}

fn refresh_auto(ui: &Ui) {
    text(
        ui.auto,
        match auto_enabled() {
            Ok(true) => "Mit Windows starten · EIN",
            Ok(false) => "Mit Windows starten · AUS",
            Err(()) => "Autostart prüfen / aktivieren",
        },
    );
}
fn change_key(ui: &mut Ui, key: Option<Zeroizing<String>>) -> Result<(), ()> {
    let configured = key.is_some();
    ui.generation = ui.generation.checked_add(1).ok_or(())?;
    let next = if configured {
        "Verbindung wird hergestellt …"
    } else {
        "Nicht eingerichtet"
    };
    let status = format!("{next}\r\nSpielstatus wird geprüft …");
    if let Ok(mut shared) = ui.shared.lock() {
        *shared = (ui.generation, status.clone());
    }
    show_status(ui, &status);
    ui.last = String::new();
    ui.tx.send(Command::Key(ui.generation, key)).map_err(|_| ())
}
fn click(hwnd: HWND, id: usize) {
    if id == DASH || id == REPO {
        open_link(hwnd, if id == DASH { DASH_URL } else { REPO_URL });
        return;
    }
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
                    change_key(ui, Some(key))
                        .map_err(|_| "Hintergrundbetrieb beendet. Bitte neu starten.")?;
                    Ok(())
                });
                if result.is_ok() {
                    text(ui.saved, "Schlüssel: geschützt gespeichert");
                    text(ui.save, "Speichern");
                    text(ui.status, "Verbindung wird hergestellt …");
                }
                text(
                    ui.notice,
                    result
                        .err()
                        .unwrap_or("Verbindungsschlüssel geschützt gespeichert."),
                );
            }
            FORGET => {
                text(ui.edit, "");
                if forget_key().is_ok() && change_key(ui, None).is_ok() {
                    text(ui.saved, "Schlüssel: nicht gespeichert");
                    text(ui.save, "Verbinden");
                    text(ui.status, "Nicht eingerichtet");
                    text(
                        ui.notice,
                        "Verbindungsschlüssel entfernt. Hintergrundbetrieb wird aktualisiert.",
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
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            let dpi = GetDpiForWindow(hwnd).max(96);
            let mut r: RECT = zeroed();
            GetClientRect(hwnd, &mut r);
            SetDCBrushColor(dc, BG);
            FillRect(dc, &r, GetStockObject(DC_BRUSH) as HBRUSH);
            for (x, y, w, h) in [
                (24, 86, 286, 132),
                (322, 86, 286, 132),
                (24, 234, 584, 210),
                (24, 460, 584, 102),
            ] {
                rounded(dc, scaled_rect((x, y, w, h), dpi), CARD, scale(18, dpi));
            }
            rounded(dc, scaled_rect(EDIT_BOUNDS, dpi), FIELD, scale(8, dpi));
            let icon_size = scale(42, dpi);
            let (icon_top, _) =
                centered(scale(HEADER_TOP, dpi), scale(HEADER_BOTTOM, dpi), icon_size);
            let icon = LoadImageW(
                GetModuleHandleW(null()),
                wide("COLLECTOR").as_ptr(),
                IMAGE_ICON,
                icon_size,
                icon_size,
                LR_DEFAULTCOLOR,
            );
            if !icon.is_null() {
                DrawIconEx(
                    dc,
                    scale(24, dpi),
                    icon_top,
                    icon as HICON,
                    icon_size,
                    icon_size,
                    0,
                    null_mut(),
                    DI_NORMAL,
                );
                DestroyIcon(icon as HICON);
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
            let dc = w as HDC;
            let child = l as HWND;
            let id = GetDlgCtrlID(child) as usize;
            let bg = if msg == WM_CTLCOLOREDIT {
                FIELD
            } else if id == HEADING || id == FOOT {
                BG
            } else {
                CARD
            };
            let fg = if id == BACKEND || id == GAME {
                let mut buf = [0u16; 256];
                let len = GetWindowTextW(child, buf.as_mut_ptr(), 256);
                let label = String::from_utf16_lossy(&buf[..len.max(0) as usize]);
                if label.contains("Verbunden ·") || label.contains("Live-Daten werden") {
                    GREEN
                } else if label.contains("Ungültig")
                    || label.contains("nicht erreichbar")
                    || label.contains("fehlgeschlagen")
                {
                    RED
                } else {
                    GOLD
                }
            } else if id == FOOT {
                MUTED
            } else {
                FG
            };
            SetBkColor(dc, bg);
            SetTextColor(dc, fg);
            SetDCBrushColor(dc, bg);
            GetStockObject(DC_BRUSH) as LRESULT
        }
        WM_DRAWITEM if l != 0 => {
            draw_button(&*(l as *const DRAWITEMSTRUCT));
            1
        }
        WM_DPICHANGED => {
            let dpi = (w & 0xffff) as u32;
            let r = &*(l as *const RECT);
            SetWindowPos(
                hwnd,
                null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            LAYOUT.with(|items| {
                for &(child, bounds, id) in items.borrow().iter() {
                    apply_control_layout(child, bounds, id, dpi);
                }
            });
            InvalidateRect(hwnd, null(), 1);
            0
        }
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
                        .map(|s| s.1.clone())
                        .unwrap_or_else(|_| "Hintergrundfehler.".into());
                    if status != ui.last {
                        show_status(ui, &status);
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
                        UI.with(|cell| {
                            if let Some(ui) = cell.borrow().as_ref() {
                                for line in ui.last.lines().take(2) {
                                    AppendMenuW(
                                        menu,
                                        MF_STRING | MF_GRAYED,
                                        0,
                                        wide(line).as_ptr(),
                                    );
                                }
                            }
                        });
                        AppendMenuW(menu, MF_SEPARATOR, 0, null());
                        AppendMenuW(menu, MF_STRING, DASH, wide("Dashboard öffnen").as_ptr());
                        AppendMenuW(
                            menu,
                            MF_STRING
                                | if auto_enabled() == Ok(true) {
                                    MF_CHECKED
                                } else {
                                    0
                                },
                            AUTO,
                            wide("Mit Windows starten").as_ptr(),
                        );
                        AppendMenuW(menu, MF_SEPARATOR, 0, null());
                        AppendMenuW(menu, MF_STRING, EXIT, wide("Collector beenden").as_ptr());
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
    SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    let class = wide(ID);
    let instance = GetModuleHandleW(null());
    let icon = LoadIconW(instance, wide("COLLECTOR").as_ptr());
    let icon = if icon.is_null() {
        LoadIconW(null_mut(), IDI_APPLICATION)
    } else {
        icon
    };
    let mut wc: WNDCLASSW = zeroed();
    wc.lpfnWndProc = Some(wndproc);
    wc.hInstance = instance;
    wc.lpszClassName = class.as_ptr();
    wc.hIcon = icon;
    wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
    wc.hbrBackground = null_mut();
    if RegisterClassW(&wc) == 0 {
        return Err("Fensterklasse konnte nicht eingerichtet werden.");
    }
    let hwnd = CreateWindowExW(
        0,
        class.as_ptr(),
        wide(TITLE).as_ptr(),
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        620,
        505,
        null_mut(),
        null_mut(),
        instance,
        null(),
    );
    if hwnd.is_null() {
        return Err("Statusfenster konnte nicht erstellt werden.");
    }
    let dpi = GetDpiForWindow(hwnd).max(96);
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 632 * dpi as i32 / 96,
        bottom: 622 * dpi as i32 / 96,
    };
    AdjustWindowRectExForDpi(
        &mut rect,
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN,
        0,
        0,
        dpi,
    );
    SetWindowPos(
        hwnd,
        null_mut(),
        0,
        0,
        rect.right - rect.left,
        rect.bottom - rect.top,
        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
    );
    let dark: i32 = 1;
    windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
        hwnd,
        20,
        (&dark as *const i32).cast(),
        4,
    );
    let heading = control(
        hwnd,
        "STATIC",
        "Deadlock Live Collector",
        SS_CENTERIMAGE,
        HEADING,
        (82, HEADER_TOP, 520, 34),
    );
    let _version = control(
        hwnd,
        "STATIC",
        concat!("Version ", env!("CARGO_PKG_VERSION"), " · Windows x64"),
        SS_CENTERIMAGE,
        FOOT,
        (82, 54, 500, 22),
    );
    let _ = control(
        hwnd,
        "STATIC",
        "BACKEND",
        SS_CENTERIMAGE,
        0,
        (40, 100, 254, 22),
    );
    let _ = control(
        hwnd,
        "STATIC",
        "DEADLOCK",
        SS_CENTERIMAGE,
        0,
        (338, 100, 254, 22),
    );
    let status = control(
        hwnd,
        "STATIC",
        "Verbindung wird hergestellt …",
        0,
        BACKEND,
        (40, 129, 254, 78),
    );
    let game = control(
        hwnd,
        "STATIC",
        "Spielstatus wird geprüft …",
        0,
        GAME,
        (338, 129, 254, 78),
    );
    let _ = control(
        hwnd,
        "STATIC",
        "VERBINDUNGSSCHLÜSSEL",
        0,
        0,
        (40, 248, 540, 23),
    );
    let saved = control(
        hwnd,
        "STATIC",
        "Schlüssel wird geprüft …",
        0,
        0,
        (40, 275, 540, 22),
    );
    let edit = control(
        hwnd,
        "EDIT",
        "",
        ES_PASSWORD as u32 | ES_AUTOHSCROLL as u32 | WS_TABSTOP,
        EDIT,
        EDIT_BOUNDS,
    );
    SendMessageW(edit, EM_SETLIMITTEXT, 128, 0);
    let button = |label, id, bounds| {
        control(
            hwnd,
            "BUTTON",
            label,
            WS_TABSTOP | BS_OWNERDRAW as u32,
            id,
            bounds,
        )
    };
    let save = button("Verbinden", SAVE, (40, 347, 170, 36));
    let forget = button("Schlüssel entfernen", FORGET, (220, 347, 174, 36));
    let dashboard = button("Dashboard öffnen", DASH, (404, 347, 188, 36));
    let notice = control(hwnd, "STATIC", "", 0, 0, (40, 394, 552, 44));
    let _ = control(hwnd, "STATIC", "EINSTELLUNGEN", 0, 0, (40, 474, 540, 22));
    let auto = button("Mit Windows starten · AUS", AUTO, (40, 502, 246, 36));
    let _ = control(
        hwnd,
        "STATIC",
        "Schließen (X) minimiert in den Infobereich. Der Collector läuft weiter.",
        0,
        0,
        (302, 503, 290, 52),
    );
    let repo = button("GitHub  ↗", REPO, (24, 577, 145, 30));
    let _ = control(
        hwnd,
        "STATIC",
        concat!("Build ", env!("CARGO_PKG_VERSION"), " · x64"),
        SS_CENTERIMAGE,
        FOOT,
        (184, 577, 240, 30),
    );
    let exit = button("Collector beenden", EXIT, (434, 577, 174, 30));
    if [
        heading, status, game, edit, save, forget, dashboard, auto, exit, notice, saved, repo,
    ]
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
    if configured {
        text(save, "Speichern");
    } else {
        text(status, "Nicht eingerichtet");
    }
    text(notice, notice_text);
    text(
        saved,
        if configured {
            "Schlüssel: geschützt gespeichert"
        } else {
            "Schlüssel: nicht gespeichert"
        },
    );
    let shared = Arc::new(Mutex::new((
        0,
        if configured {
            "Verbindung wird hergestellt …".into()
        } else {
            "Nicht eingerichtet".into()
        },
    )));
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
        game,
        save,
        notice,
        saved,
        auto,
        tx,
        shared,
        last: String::new(),
        generation: 0,
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
                status.1 =
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
        if IsDialogMessageW(hwnd, &msg) == 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    KillTimer(hwnd, 1);
    UI.with(|cell| {
        if let Some(ui) = cell.borrow_mut().take() {
            let _ = ui.tx.send(Command::Stop);
            Shell_NotifyIconW(NIM_DELETE, &ui.tray);
        }
    });
    let _ = handle.join();
    FONTS.with(|fonts| {
        for (_, _, h) in fonts.borrow_mut().drain(..) {
            if !h.is_null() {
                DeleteObject(h);
            }
        }
    });
    LAYOUT.with(|items| items.borrow_mut().clear());
    Ok(())
}
