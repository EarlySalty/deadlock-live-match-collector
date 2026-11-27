//! Native per-user desktop application. No service, injection or elevation.
use crate::{
    desktop_runtime::{ConnectionProbe, Runtime, IDLE},
    independent_source::open_source,
    secret,
    window_layout::{
        centered, ink_baseline, scale, AUTO_BOUNDS, BRAND_BOUNDS, CAPTION_H, CARDS, CLIENT_H,
        CLIENT_W, CLOSE_BOUNDS, EDIT_BOUNDS, EXIT_BOUNDS, FOOT_BOUNDS, HEADER_BOTTOM, HEADER_TOP,
        MINIMIZE_BOUNDS, REPO_BOUNDS, UPDATE_BOUNDS,
    },
    windows_paint::{Buffer, Canvas, Session},
    windows_vector::VectorFrame,
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
        Controls::{DRAWITEMSTRUCT, EM_SETLIMITTEXT, EM_SETMARGINS, ODT_STATIC, WM_MOUSELEAVE},
        HiDpi::*,
        Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT},
        Shell::*,
        WindowsAndMessaging::*,
    },
};
use zeroize::Zeroizing;
#[path = "windows_update.rs"]
mod updates;

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
        if GetDlgCtrlID(hwnd) as usize != EDIT {
            let mut old = [0u16; 256];
            let count = GetWindowTextW(hwnd, old.as_mut_ptr(), old.len() as i32).max(0) as usize;
            if old[..count].iter().copied().eq(s.encode_utf16()) {
                return;
            }
        }
        SetWindowTextW(hwnd, wide(s).as_ptr());
        InvalidateRect(hwnd, null(), 0);
    }
}
pub(super) fn alert(s: &str) {
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
    if enable {
        let exe = std::env::current_exe().map_err(|_| ())?;
        set_auto_path(true, &exe)
    } else {
        set_auto_path(false, std::path::Path::new(""))
    }
}
pub(super) fn set_auto_path(enable: bool, exe: &std::path::Path) -> Result<(), ()> {
    unsafe {
        let rc = if enable {
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
        let dpi = window_dpi(parent);
        let r = scaled_rect(bounds, dpi);
        let hwnd = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(caption).as_ptr(),
            WS_CHILD
                | WS_VISIBLE
                | if class == "STATIC" {
                    SS_OWNERDRAW
                } else {
                    style
                },
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
// COLORREF uses BGR byte order. Distinct neutral layers, with gold reserved
// for primary actions and brand/icon accents.
const BG: u32 = 0x171616; // #161617
const CARD: u32 = 0x282626; // #262628
const STATUS_CARD: u32 = 0x2d2a2a; // #2a2a2d
const GOLD: u32 = 0x5ea1c7; // #c7a15e
const FG: u32 = 0xf5f5f5;
const MUTED: u32 = 0xd5d3d2;
const FIELD: u32 = 0x211f1f;
const LINE: u32 = 0x494646;
const TILE: u32 = 0x3d3938;
const STATUS_TILE: u32 = 0x45413e;
const TOP_BG: u32 = 0x201e1e;
const FOOT_BG: u32 = 0x201e1e;
const CARD_BORDER: u32 = 0x444141;
const FRAME: u32 = 0x565351;
const CAPTION_LABEL: usize = 216;
const STATUS_LABEL: usize = 215;
const MINIMIZE: usize = 211;
const CLOSE_WINDOW: usize = 212;
const AUTO_LABEL: usize = 213;
const AUTO_HINT: usize = 214;
const SS_OWNERDRAW: u32 = 0x000d;
const SECTION: usize = 206;
const BRAND: usize = 207;
const SUBTLE: usize = 208;
const CARD_LABEL: usize = 209;
const BACKEND: usize = 202;
const GAME: usize = 203;
const FOOT: usize = 204;
const EDIT: usize = 205;
type LayoutItem = (HWND, (i32, i32, i32, i32), usize);
thread_local! {
    static LAYOUT: RefCell<Vec<LayoutItem>> = const { RefCell::new(Vec::new()) };
    static HOVER: RefCell<Vec<(HWND,u8)>> = const { RefCell::new(Vec::new()) };
    static RENDER_REPORTED: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    static FONTS: RefCell<Vec<(usize,u32,HFONT)>> = const { RefCell::new(Vec::new()) };
}
fn font(id: usize, dpi: u32) -> HFONT {
    let (size, weight, _) = text_style(id);
    let kind = size as usize * 1000 + weight as usize;
    FONTS.with(|fonts| {
        let mut fonts = fonts.borrow_mut();
        if let Some((_, _, h)) = fonts.iter().find(|(k, d, _)| *k == kind && *d == dpi) {
            return *h;
        }
        let h = unsafe {
            CreateFontW(
                -scale(size, dpi),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                ANTIALIASED_QUALITY as u32,
                DEFAULT_PITCH as u32,
                wide("Segoe UI").as_ptr(),
            )
        };
        fonts.push((kind, dpi, h));
        h
    })
}
fn window_dpi(hwnd: HWND) -> u32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        96
    } else {
        dpi
    }
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
        r.left += scale(39, dpi);
        r.right -= scale(4, dpi);
        let margin = scale(3, dpi) as isize;
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
    if let Some(canvas) = Canvas::new(dc) {
        if canvas.rounded(r, color, radius) {
            return;
        }
    }
    let old_brush = SelectObject(dc, GetStockObject(DC_BRUSH));
    let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
    SetDCBrushColor(dc, color);
    RoundRect(dc, r.left, r.top, r.right, r.bottom, radius, radius);
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
}
unsafe fn outlined(dc: HDC, r: RECT, fill: u32, border: u32, radius: i32, stroke: i32) {
    rounded(dc, r, border, radius);
    let inner = RECT {
        left: r.left + stroke,
        top: r.top + stroke,
        right: r.right - stroke,
        bottom: r.bottom - stroke,
    };
    rounded(dc, inner, fill, (radius - 2 * stroke).max(1));
}

fn blend(a: u32, b: u32, n: u8) -> u32 {
    [0, 8, 16].into_iter().fold(0, |out, shift| {
        let x = (a >> shift) & 255;
        let y = (b >> shift) & 255;
        out | (((x * (4 - n as u32) + y * n as u32) / 4) << shift)
    })
}
fn hover_level(hwnd: HWND) -> u8 {
    HOVER.with(|h| {
        h.borrow()
            .iter()
            .find(|(w, _)| *w == hwnd)
            .map_or(0, |(_, v)| *v)
    })
}
unsafe fn set_hover(child: HWND, value: u8) {
    let changed = HOVER.with(|h| {
        let mut h = h.borrow_mut();
        if let Some((_, v)) = h.iter_mut().find(|(w, _)| *w == child) {
            if *v != value {
                *v = value;
                return true;
            }
        }
        false
    });
    if changed {
        InvalidateRect(child, null(), 0);
    }
}
unsafe fn clear_hover() {
    let children = HOVER.with(|h| h.borrow().iter().map(|(w, _)| *w).collect::<Vec<_>>());
    for child in children {
        set_hover(child, 0);
    }
}
unsafe extern "system" fn hover_subclass(
    child: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_MOUSEMOVE if hover_level(child) == 0 => {
            let mut track = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: child,
                dwHoverTime: 0,
            };
            if TrackMouseEvent(&mut track) != 0 {
                set_hover(child, 4);
            }
        }
        WM_MOUSELEAVE => set_hover(child, 0),
        WM_ERASEBKGND => return 1,
        WM_NCDESTROY => {
            HOVER.with(|h| h.borrow_mut().retain(|(w, _)| *w != child));
            RemoveWindowSubclass(child, Some(hover_subclass), id);
        }
        _ => {}
    }
    DefSubclassProc(child, msg, w, l)
}
fn text_style(id: usize) -> (i32, i32, bool) {
    match id {
        BACKEND | GAME => (20, FW_SEMIBOLD as i32, true),
        SECTION | AUTO_LABEL => (17, FW_SEMIBOLD as i32, false),
        SAVE => (16, FW_SEMIBOLD as i32, false),
        BRAND => (18, FW_SEMIBOLD as i32, false),
        FOOT | updates::CHECK => (14, FW_NORMAL as i32, false),
        CAPTION_LABEL => (13, FW_NORMAL as i32, false),
        STATUS_LABEL | CARD_LABEL | AUTO_HINT => (15, FW_NORMAL as i32, false),
        SUBTLE => (15, FW_NORMAL as i32, true),
        EDIT => (15, FW_NORMAL as i32, false),
        _ => (15, FW_NORMAL as i32, false),
    }
}
fn label_color(id: usize) -> u32 {
    match id {
        BACKEND | GAME => FG,
        BRAND => GOLD,
        FOOT | SUBTLE | CARD_LABEL | STATUS_LABEL | AUTO_HINT | CAPTION_LABEL => MUTED,
        _ => FG,
    }
}
unsafe fn fill(dc: HDC, r: RECT, color: u32) {
    SetDCBrushColor(dc, color);
    FillRect(dc, &r, GetStockObject(DC_BRUSH) as HBRUSH);
}
unsafe fn paint_label(
    dc: HDC,
    id: usize,
    dpi: u32,
    label: &[u16],
    mut r: RECT,
    fg: u32,
    center: bool,
) {
    let (size, weight, wrap) = text_style(id);
    if let Some(canvas) = Canvas::new(dc) {
        if canvas.text(
            label,
            r,
            fg,
            scale(size, dpi),
            (weight >= FW_SEMIBOLD as i32, center, wrap),
        ) {
            return;
        }
    }
    let old = SelectObject(dc, font(id, dpi));
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, fg);
    let flags = if center {
        center_button_label(dc, label, &mut r)
    } else if wrap {
        let mut extent: SIZE = zeroed();
        if GetTextExtentPoint32W(dc, label.as_ptr(), label.len() as i32, &mut extent) != 0
            && extent.cx <= r.right - r.left
        {
            center_button_label(dc, label, &mut r) & !DT_CENTER
        } else {
            DT_WORDBREAK | DT_NOPREFIX
        }
    } else {
        center_button_label(dc, label, &mut r) & !DT_CENTER
    };
    DrawTextW(dc, label.as_ptr(), label.len() as i32, &mut r, flags);
    SelectObject(dc, old);
}
struct Surface<'a> {
    dc: HDC,
    vector: Option<&'a VectorFrame>,
}
impl Surface<'_> {
    unsafe fn fill(&self, r: RECT, c: u32) {
        if let Some(v) = self.vector {
            v.fill(r, c);
        } else {
            fill(self.dc, r, c);
        }
    }
    unsafe fn rounded(&self, r: RECT, c: u32, d: i32) {
        if let Some(v) = self.vector {
            v.rounded(r, c, d);
        } else {
            rounded(self.dc, r, c, d);
        }
    }
    unsafe fn outlined(&self, r: RECT, c: u32, b: u32, d: i32, stroke: i32) {
        if let Some(v) = self.vector {
            v.rounded(r, b, d);
            let inner = RECT {
                left: r.left + stroke,
                top: r.top + stroke,
                right: r.right - stroke,
                bottom: r.bottom - stroke,
            };
            v.rounded(inner, c, (d - 2 * stroke).max(1));
        } else {
            outlined(self.dc, r, c, b, d, stroke);
        }
    }
    unsafe fn icon(&self, name: &str, r: RECT, c: u32) -> bool {
        if let Some(v) = self.vector {
            v.icon(name, r, c);
            true
        } else if let Some(canvas) = Canvas::new(self.dc) {
            canvas.icon(name, r, c);
            true
        } else {
            false
        }
    }
    unsafe fn label(&self, id: usize, dpi: u32, label: &[u16], r: RECT, c: u32, center: bool) {
        if let Some(v) = self.vector {
            let (size, weight, wrap) = text_style(id);
            v.text(label, r, c, scale(size, dpi), weight, (center, wrap));
        } else {
            paint_label(self.dc, id, dpi, label, r, c, center);
        }
    }
    unsafe fn measure(&self, id: usize, dpi: u32, label: &[u16]) -> i32 {
        let (size, weight, _) = text_style(id);
        if let Some(v) = self.vector {
            if let Some(width) = v.text_width(label, scale(size, dpi), weight) {
                return width;
            }
            // Do not issue GDI calls into an active Direct2D surface.
            return scale(size, dpi) * (label.len() as i32);
        }
        if let Some(width) = Canvas::new(self.dc).and_then(|canvas| {
            canvas.text_width(label, scale(size, dpi), weight >= FW_SEMIBOLD as i32)
        }) {
            return width;
        }
        let old = SelectObject(self.dc, font(id, dpi));
        let mut measured: SIZE = zeroed();
        GetTextExtentPoint32W(self.dc, label.as_ptr(), label.len() as i32, &mut measured);
        SelectObject(self.dc, old);
        measured.cx + scale(2, dpi)
    }
    unsafe fn logo(&self, r: RECT) {
        if let Some(v) = self.vector {
            v.logo(r);
            return;
        }
        let size = r.right - r.left;
        let icon = LoadImageW(
            GetModuleHandleW(null()),
            wide("COLLECTOR").as_ptr(),
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        );
        if !icon.is_null() {
            DrawIconEx(
                self.dc,
                r.left,
                r.top,
                icon as HICON,
                size,
                size,
                0,
                null_mut(),
                DI_NORMAL,
            );
            DestroyIcon(icon as HICON);
        }
    }
}
// Debugger-only, no files, network, user data or key contents.
fn report_renderer(flag: u8, message: &str) {
    RENDER_REPORTED.with(|reported| {
        if reported.get() & flag == 0 {
            reported.set(reported.get() | flag);
            unsafe {
                windows_sys::Win32::System::Diagnostics::Debug::OutputDebugStringW(
                    wide(message).as_ptr(),
                );
            }
        }
    });
}
fn render_surface(dc: HDC, r: RECT, mut draw: impl FnMut(&Surface<'_>)) {
    if let Some(v) = VectorFrame::begin(dc, r) {
        draw(&Surface {
            dc,
            vector: Some(&v),
        });
        if v.finish() {
            report_renderer(1, "Direct2D/DirectWrite frame completed (native pixels)");
            return;
        }
    }
    // Device lost or unsupported target: redraw the entire frame with the
    // proven 0.2.5 grayscale fallback, never leave a partially drawn control.
    report_renderer(
        2,
        "Direct2D unavailable: complete frame uses GDI+/GDI fallback",
    );
    draw(&Surface { dc, vector: None });
}

unsafe fn draw_static(item: &DRAWITEMSTRUCT) {
    let id = item.CtlID as usize;
    let dpi = window_dpi(item.hwndItem);
    let bg = if id == FOOT {
        FOOT_BG
    } else if id == BRAND {
        BG
    } else if matches!(id, BACKEND | GAME | STATUS_LABEL) {
        STATUS_CARD
    } else {
        CARD
    };
    let mut label = [0u16; 256];
    let n = GetWindowTextW(item.hwndItem, label.as_mut_ptr(), 256).max(0) as usize;
    let fg = label_color(id);
    render_surface(item.hDC, item.rcItem, |surface| {
        surface.fill(item.rcItem, bg);
        surface.label(id, dpi, &label[..n], item.rcItem, fg, false);
    });
}
unsafe fn draw_button(item: &DRAWITEMSTRUCT) {
    let id = item.CtlID as usize;
    let dpi = window_dpi(item.hwndItem);
    let sc = |v| scale(v, dpi);
    let r = item.rcItem;
    let active = item.itemState & 1 != 0;
    let hover = hover_level(item.hwndItem);
    let bg = if matches!(id, REPO | EXIT | updates::CHECK) {
        FOOT_BG
    } else if matches!(id, MINIMIZE | CLOSE_WINDOW) {
        TOP_BG
    } else {
        CARD
    };
    let mut label = [0u16; 128];
    let n = GetWindowTextW(item.hwndItem, label.as_mut_ptr(), 128).max(0) as usize;
    let fg = if id == SAVE {
        BG
    } else if matches!(id, REPO | EXIT | MINIMIZE | CLOSE_WINDOW | FORGET) {
        MUTED
    } else {
        FG
    };
    render_surface(item.hDC, r, |surface| {
        surface.fill(r, bg);
        if id == AUTO {
            let state = String::from_utf16_lossy(&label[..n]);
            let on = state.ends_with("EIN");
            let off = state.ends_with("AUS");
            let (top, bottom) = centered(r.top, r.bottom, sc(24));
            let (left, right) = centered(r.left, r.right, sc(44));
            surface.rounded(
                RECT {
                    left,
                    top,
                    right,
                    bottom,
                },
                blend(
                    if on { GOLD } else { 0x514941 },
                    if on { 0x78b8de } else { 0x74665b },
                    hover,
                ),
                sc(24),
            );
            let diameter = sc(18);
            let (t, b) = centered(r.top, r.bottom, diameter);
            let x = if on {
                right - sc(3) - diameter
            } else {
                left + sc(3)
            };
            surface.rounded(
                RECT {
                    left: x,
                    top: t,
                    right: x + diameter,
                    bottom: b,
                },
                if on { BG } else { FG },
                diameter,
            );
            if !on && !off {
                let q = wide("?");
                surface.label(
                    id,
                    dpi,
                    &q[..1],
                    RECT {
                        left: x,
                        top: t,
                        right: x + diameter,
                        bottom: b,
                    },
                    BG,
                    true,
                );
            }
        } else {
            let base = if id == SAVE {
                GOLD
            } else if id == DASH {
                0x454242
            } else {
                bg
            };
            let hot = if id == SAVE {
                0x6aafd5
            } else if id == CLOSE_WINDOW {
                0x494078
            } else {
                0x504c49
            };
            let color = blend(base, hot, if active { 4 } else { hover });
            surface.rounded(r, color, sc(10));
            let icon = match id {
                SAVE => "link",
                DASH => "external",
                FORGET => "trash",
                REPO => "repo",
                updates::CHECK => "external",
                EXIT => "power",
                MINIMIZE => "minus",
                CLOSE_WINDOW => "close",
                _ => "",
            };
            let icon_size = sc(18);
            let (top, bottom) = centered(r.top, r.bottom, icon_size);
            let icon_only = matches!(id, MINIMIZE | CLOSE_WINDOW);
            let (left, text_rect) = if icon_only {
                (centered(r.left, r.right, icon_size).0, r)
            } else {
                let measured_width = surface.measure(id, dpi, &label[..n]);
                let (icon_left, text_left, text_right) = crate::window_layout::icon_label_group(
                    r.left + sc(8),
                    r.right - sc(8),
                    icon_size,
                    sc(8),
                    measured_width,
                );
                // Keep the version/update group aligned with the card and logo edge.
                let shift = if id == updates::CHECK {
                    r.left + sc(8) - icon_left
                } else {
                    0
                };
                (
                    icon_left + shift,
                    RECT {
                        left: text_left + shift,
                        right: text_right + shift,
                        ..r
                    },
                )
            };
            if !surface.icon(
                icon,
                RECT {
                    left,
                    top,
                    right: left + icon_size,
                    bottom,
                },
                fg,
            ) && icon_only
            {
                let glyph = wide(if id == MINIMIZE { "−" } else { "×" });
                surface.label(id, dpi, &glyph[..1], r, fg, true);
            }
            if !icon_only {
                surface.label(id, dpi, &label[..n], text_rect, fg, true);
            }
        }
    });
    if item.itemState & 0x10 != 0 {
        let mut focus = r;
        InflateRect(&mut focus, -sc(3), -sc(3));
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
    if id == updates::CHECK {
        updates::request(true);
        return;
    }
    if id == MINIMIZE || id == CLOSE_WINDOW {
        unsafe {
            if id == MINIMIZE {
                ShowWindow(hwnd, SW_MINIMIZE);
            } else {
                SendMessageW(hwnd, WM_CLOSE, 0, 0);
            }
        }
        return;
    }
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
                            "Autostart aktiviert."
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
        WM_NCCALCSIZE => 0,
        WM_NCHITTEST => {
            let mut pt = POINT {
                x: (l as u16 as i16) as i32,
                y: ((l >> 16) as u16 as i16) as i32,
            };
            ScreenToClient(hwnd, &mut pt);
            let dpi = window_dpi(hwnd);
            if pt.y >= 0
                && pt.y < scale(CAPTION_H, dpi)
                && pt.x >= 0
                && pt.x < scale(MINIMIZE_BOUNDS.0, dpi)
            {
                HTCAPTION as LRESULT
            } else {
                HTCLIENT as LRESULT
            }
        }
        WM_NCLBUTTONDBLCLK if w == HTCAPTION as usize => 0,
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let target = BeginPaint(hwnd, &mut ps);
            let dpi = window_dpi(hwnd);
            let mut r: RECT = zeroed();
            GetClientRect(hwnd, &mut r);
            let buffer = Buffer::new(target, r);
            let dc = buffer.as_ref().map_or(target, Buffer::dc);
            render_surface(dc, r, |surface| {
                surface.fill(r, BG);
                // Native-style caption and full-width status bar. Draw separators
                // at integer device-pixel edges, not scaled one-DIP strokes.
                let cap = scale(CAPTION_H, dpi);
                let foot = scale(FOOT_BOUNDS.1, dpi);
                surface.fill(
                    RECT {
                        left: 1,
                        top: 1,
                        right: r.right - 1,
                        bottom: cap,
                    },
                    TOP_BG,
                );
                surface.fill(
                    RECT {
                        left: 1,
                        top: cap - 1,
                        right: r.right - 1,
                        bottom: cap,
                    },
                    LINE,
                );
                surface.fill(
                    RECT {
                        left: 1,
                        top: foot,
                        right: r.right - 1,
                        bottom: r.bottom - 1,
                    },
                    FOOT_BG,
                );
                surface.fill(
                    RECT {
                        left: 1,
                        top: foot,
                        right: r.right - 1,
                        bottom: foot + 1,
                    },
                    LINE,
                );
                surface.logo(scaled_rect((12, 9, 16, 16), dpi));
                let caption = wide(TITLE);
                surface.label(
                    CAPTION_LABEL,
                    dpi,
                    &caption[..caption.len() - 1],
                    scaled_rect((36, 2, 490, 30), dpi),
                    MUTED,
                    false,
                );
                for (index, bounds) in CARDS.into_iter().enumerate() {
                    let card = scaled_rect(bounds, dpi);
                    let color = if index < 2 { STATUS_CARD } else { CARD };
                    if let Some(v) = surface.vector {
                        v.bordered(card, color, CARD_BORDER, scale(20, dpi));
                    } else {
                        surface.outlined(card, color, CARD_BORDER, scale(20, dpi), 1);
                    }
                }
                // Visible single physical pixel frame, independent of DWM support.
                for edge in [
                    RECT {
                        left: 0,
                        top: 0,
                        right: r.right,
                        bottom: 1,
                    },
                    RECT {
                        left: 0,
                        top: r.bottom - 1,
                        right: r.right,
                        bottom: r.bottom,
                    },
                    RECT {
                        left: 0,
                        top: 1,
                        right: 1,
                        bottom: r.bottom - 1,
                    },
                    RECT {
                        left: r.right - 1,
                        top: 1,
                        right: r.right,
                        bottom: r.bottom - 1,
                    },
                ] {
                    surface.fill(edge, FRAME);
                }
                surface.outlined(
                    scaled_rect(EDIT_BOUNDS, dpi),
                    FIELD,
                    LINE,
                    scale(12, dpi),
                    1,
                );
                for (x, y, name) in [
                    (40, 146, "server"),
                    (338, 146, "game"),
                    (40, 278, "shield"),
                    (40, 508, "power"),
                ] {
                    surface.rounded(
                        scaled_rect((x, y, 32, 32), dpi),
                        if y == 146 { STATUS_TILE } else { TILE },
                        scale(10, dpi),
                    );
                    surface.icon(name, scaled_rect((x + 5, y + 5, 22, 22), dpi), GOLD);
                }
                surface.icon("key", scaled_rect((52, 342, 18, 18), dpi), MUTED);
                let size = scale(64, dpi);
                let (top, bottom) =
                    centered(scale(HEADER_TOP, dpi), scale(HEADER_BOTTOM, dpi), size);
                let left = scale(24, dpi);
                surface.logo(RECT {
                    left,
                    top,
                    right: left + size,
                    bottom,
                });
            });
            if let Some(b) = buffer {
                b.present(target, r);
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
            let dc = w as HDC;
            let id = GetDlgCtrlID(l as HWND) as usize;
            let bg = if id == EDIT {
                FIELD
            } else if id == FOOT {
                FOOT_BG
            } else if id == BRAND {
                BG
            } else if matches!(id, BACKEND | GAME | STATUS_LABEL) {
                STATUS_CARD
            } else {
                CARD
            };
            SetBkColor(dc, bg);
            SetTextColor(dc, FG);
            SetDCBrushColor(dc, bg);
            GetStockObject(DC_BRUSH) as LRESULT
        }
        WM_DRAWITEM if l != 0 => {
            let item = &*(l as *const DRAWITEMSTRUCT);
            let buffer = Buffer::new(item.hDC, item.rcItem);
            let mut painted = *item;
            if let Some(b) = &buffer {
                painted.hDC = b.dc();
            }
            if item.CtlType == ODT_STATIC {
                draw_static(&painted);
            } else {
                draw_button(&painted);
            }
            if let Some(b) = buffer {
                b.present(item.hDC, item.rcItem);
            }
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
                scale(CLIENT_W, dpi),
                scale(CLIENT_H, dpi),
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
            updates::stop();
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
        WM_SHOWWINDOW if w == 0 => {
            clear_hover();
            DefWindowProcW(hwnd, msg, w, l)
        }
        WM_SIZE if w == SIZE_MINIMIZED as usize => {
            clear_hover();
            DefWindowProcW(hwnd, msg, w, l)
        }
        WM_TIMER => {
            updates::tick();
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
                            MF_STRING,
                            updates::CHECK,
                            wide("Nach Updates suchen").as_ptr(),
                        );
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
    #[cfg(not(feature = "internal-test"))]
    let gate = match crate::windows_install::InstallGate::acquire(ID) {
        Ok(gate) => gate,
        Err(reason) => {
            alert(reason);
            return;
        }
    };
    #[cfg(not(feature = "internal-test"))]
    match crate::windows_install::prepare(ID, EXIT, startup, auto_enabled()) {
        Ok(true) => {}
        Ok(false) => return,
        Err(reason) => {
            drop(gate);
            alert(reason);
            return;
        }
    }
    unsafe {
        let mutex = CreateMutexW(null(), 0, wide(&format!("Local\\{ID}")).as_ptr());
        if mutex.is_null() {
            alert("Instanzschutz konnte nicht eingerichtet werden.");
            return;
        }
        let exists = GetLastError() == ERROR_ALREADY_EXISTS;
        #[cfg(not(feature = "internal-test"))]
        drop(gate);
        if exists {
            CloseHandle(mutex);
            if !startup {
                let window = FindWindowW(wide(ID).as_ptr(), null());
                if !window.is_null() {
                    PostMessageW(window, WM_COMMAND, SHOW, 0);
                } else {
                    alert("Collector läuft bereits. Öffne das Symbol im Windows-Infobereich.");
                }
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
    let _graphics = Session::start();
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
        CLIENT_W,
        CLIENT_H,
        null_mut(),
        null_mut(),
        instance,
        null(),
    );
    if hwnd.is_null() {
        return Err("Statusfenster konnte nicht erstellt werden.");
    }
    let dpi = window_dpi(hwnd);
    SetWindowPos(
        hwnd,
        null_mut(),
        0,
        0,
        scale(CLIENT_W, dpi),
        scale(CLIENT_H, dpi),
        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
    );
    let dark: i32 = 1;
    windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
        hwnd,
        20,
        (&dark as *const i32).cast(),
        4,
    );
    windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
        hwnd,
        34,
        (&FRAME as *const u32).cast(),
        4, // DWMWA_BORDER_COLOR
    );
    let corners: i32 = 3; // DWMWCP_ROUNDSMALL
    windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
        hwnd,
        33,
        (&corners as *const i32).cast(),
        4,
    );
    let label = |caption, id, bounds| control(hwnd, "STATIC", caption, 0, id, bounds);
    let button = |caption, id, bounds| {
        let child = control(
            hwnd,
            "BUTTON",
            caption,
            WS_TABSTOP | BS_OWNERDRAW as u32,
            id,
            bounds,
        );
        if !child.is_null() && SetWindowSubclass(child, Some(hover_subclass), 1, 0) != 0 {
            HOVER.with(|h| h.borrow_mut().push((child, 0)));
        }
        child
    };
    // Caption text is painted by the parent (so this area remains draggable).
    let minimize = button("Minimieren", MINIMIZE, MINIMIZE_BOUNDS);
    let close = button("Schließen (Infobereich)", CLOSE_WINDOW, CLOSE_BOUNDS);
    let brand = label("DEUTSCHE DEADLOCK COMMUNITY", BRAND, BRAND_BOUNDS);
    let backend_label = label("Backend", STATUS_LABEL, (80, 150, 190, 24));
    let game_label = label("Deadlock", STATUS_LABEL, (378, 150, 190, 24));
    let status = label("Verbindung wird hergestellt …", BACKEND, (40, 184, 254, 46));
    let game = label("Spielstatus wird geprüft …", GAME, (338, 184, 254, 46));
    let connection = label("Verbindung", SECTION, (80, 278, 512, 32));
    let saved = label("Schlüssel wird geprüft …", SUBTLE, (40, 310, 552, 20));
    let edit = control(
        hwnd,
        "EDIT",
        "",
        ES_PASSWORD as u32 | ES_AUTOHSCROLL as u32 | WS_TABSTOP,
        EDIT,
        EDIT_BOUNDS,
    );
    SendMessageW(edit, EM_SETLIMITTEXT, 128, 0);
    let save = button("Verbinden", SAVE, (40, 382, 270, 38));
    let dashboard = button("Dashboard öffnen", DASH, (322, 382, 270, 38));
    let forget = button("Schlüssel entfernen", FORGET, (40, 430, 186, 32));
    let notice = label("", SUBTLE, (238, 430, 354, 32));
    let auto_label = label("Mit Windows starten", AUTO_LABEL, (80, 503, 420, 25));
    let auto_hint = label(
        "Beim Anmelden im Hintergrund starten",
        AUTO_HINT,
        (80, 528, 420, 21),
    );
    let auto = button("Mit Windows starten · AUS", AUTO, AUTO_BOUNDS);
    let version = button(
        concat!(
            "Version ",
            env!("CARGO_PKG_VERSION"),
            " · Nach Updates suchen"
        ),
        updates::CHECK,
        UPDATE_BOUNDS,
    );
    let repo = button("GitHub", REPO, REPO_BOUNDS);
    let exit = button("Beenden", EXIT, EXIT_BOUNDS);
    if [
        minimize,
        close,
        brand,
        backend_label,
        game_label,
        status,
        game,
        connection,
        saved,
        edit,
        save,
        dashboard,
        forget,
        notice,
        auto_label,
        auto_hint,
        auto,
        version,
        repo,
        exit,
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
        Ok(None) => (None, "Bitte Verbindungsschlüssel eingeben."),
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
    updates::start();
    if !startup || !configured {
        show(hwnd);
    }
    let mut msg: MSG = zeroed();
    while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
        if !updates::dialog_message(&mut msg) && IsDialogMessageW(hwnd, &msg) == 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    KillTimer(hwnd, 1);
    HOVER.with(|h| h.borrow_mut().clear());
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
