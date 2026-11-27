//! Native modeless update notice. The network thread never owns a window handle.
use super::*;
use crate::update_check::{self, CheckError, Notice, Update, Version};

pub(super) const CHECK: usize = 108;
const DOWNLOAD: usize = 301;
const LATER: usize = 302;
const NOTES: usize = 303;
const WIDTH: i32 = 540;
const HEIGHT: i32 = 414;
const KEY: &str = "Software\\DeadlockLiveMatchCollector\\Updates";
type ResultMessage = Result<Option<Update>, CheckError>;

struct State {
    notice: Notice,
    next: Instant,
    receiver: Option<mpsc::Receiver<ResultMessage>>,
    popup: HWND,
    update: Option<Update>,
}
thread_local! { static STATE: RefCell<Option<State>> = const { RefCell::new(None) }; }

fn saved() -> Option<Version> {
    let mut data = [0u16; 128];
    let mut bytes = std::mem::size_of_val(&data) as u32;
    let code = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(KEY).as_ptr(),
            wide(ID).as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            data.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if code != ERROR_SUCCESS {
        return None;
    }
    let end = data.iter().position(|&c| c == 0)?;
    Version::parse(&String::from_utf16_lossy(&data[..end]))
}
fn persist(tag: &str) {
    let data = wide(tag);
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            wide(KEY).as_ptr(),
            wide(ID).as_ptr(),
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        );
    }
}
pub(super) fn start() {
    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            notice: Notice::new(saved()),
            next: Instant::now(),
            receiver: None,
            popup: null_mut(),
            update: None,
        })
    });
    request(false);
}
pub(super) fn request(manual: bool) {
    let existing = STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        let Some(s) = s.as_mut() else {
            return null_mut();
        };
        if !s.popup.is_null() {
            return if manual { s.popup } else { null_mut() };
        }
        if !s.notice.begin(manual) {
            return null_mut();
        }
        s.next = Instant::now() + update_check::INTERVAL;
        let (tx, rx) = mpsc::channel();
        s.receiver = Some(rx);
        // A separate bounded HTTP client: no collector credentials or game data.
        let _ = thread::Builder::new()
            .name("release-check".into())
            .spawn(move || {
                let test_channel =
                    std::env::var("DEADLOCK_COLLECTOR_PUBLIC_TEST_CHANNEL").as_deref() == Ok("1");
                let _ = tx.send(update_check::check(test_channel));
            });
        null_mut()
    });
    if !existing.is_null() {
        show(existing);
    }
}
pub(super) fn tick() {
    let (due, result) = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let Some(s) = state.as_mut() else {
            return (false, None);
        };
        let result = match s.receiver.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(result)) => Some(result),
            Some(Err(mpsc::TryRecvError::Disconnected)) => Some(Err(CheckError::Network)),
            _ => None,
        };
        let result = result.map(|r| {
            s.receiver = None;
            let manual = s.notice.finish();
            (r, manual)
        });
        (
            Instant::now() >= s.next && !s.notice.running && s.popup.is_null(),
            result,
        )
    });
    if let Some((result, manual)) = result {
        match result {
            Ok(Some(update)) => {
                let allowed = STATE.with(|cell| cell.borrow().as_ref().is_some_and(|s| s.notice.should_show(&update, manual)));
                if allowed { popup(Some(update), "", manual); }
            }
            Ok(None) if manual => popup(None, "Keine neuere veröffentlichte Version gefunden.", true),
            Err(error) if manual => popup(None, match error {
                CheckError::RateLimited => "GitHub begrenzt momentan die Abfragen. Bitte später erneut versuchen.",
                CheckError::Unavailable => "Der öffentliche Release-Kanal ist nicht verfügbar.",
                _ => "Update-Prüfung derzeit nicht möglich. Der Collector arbeitet normal weiter.",
            }, true),
            _ => {}
        }
    }
    if due {
        request(false);
    }
}
pub(super) fn stop() {
    let hwnd = STATE.with(|s| s.borrow().as_ref().map_or(null_mut(), |s| s.popup));
    if !hwnd.is_null() {
        unsafe {
            DestroyWindow(hwnd);
        }
    }
    STATE.with(|s| {
        s.borrow_mut().take();
    });
}
fn close(hwnd: HWND) {
    let tag = STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let s = state.as_mut()?;
        let update = s.update.take()?;
        if s.notice.should_show(&update, false) {
            s.notice.dismiss(&update);
            Some(update.tag)
        } else {
            None
        }
    });
    if let Some(tag) = tag {
        persist(&tag);
    }
    unsafe {
        DestroyWindow(hwnd);
    }
}
unsafe fn child(
    hwnd: HWND,
    class: &str,
    caption: &str,
    style: u32,
    id: usize,
    b: (i32, i32, i32, i32),
) -> HWND {
    let dpi = window_dpi(hwnd);
    let r = scaled_rect(b, dpi);
    let ch = CreateWindowExW(
        0,
        wide(class).as_ptr(),
        wide(caption).as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        r.left,
        r.top,
        r.right - r.left,
        r.bottom - r.top,
        hwnd,
        id as HMENU,
        GetModuleHandleW(null()),
        null(),
    );
    if !ch.is_null() {
        SendMessageW(ch, WM_SETFONT, font(id, dpi) as usize, 1);
    }
    ch
}
const BOUNDS: [(usize, (i32, i32, i32, i32)); 3] = [
    (NOTES, (24, 132, 492, 208)),
    (DOWNLOAD, (24, 358, 260, 36)),
    (LATER, (302, 358, 214, 36)),
];
fn popup(update: Option<Update>, message: &str, manual: bool) {
    unsafe {
        let instance = GetModuleHandleW(null());
        let class = wide("DeadlockCollectorUpdateNotice");
        let mut wc: WNDCLASSW = zeroed();
        wc.lpfnWndProc = Some(proc);
        wc.hInstance = instance;
        wc.lpszClassName = class.as_ptr();
        wc.hIcon = LoadIconW(instance, wide("COLLECTOR").as_ptr());
        wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            wide("Deadlock Collector – Updates").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            WIDTH,
            HEIGHT,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return;
        }
        let dpi = window_dpi(hwnd);
        let mut r = scaled_rect((0, 0, WIDTH, HEIGHT), dpi);
        AdjustWindowRectExForDpi(
            &mut r,
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
            0,
            WS_EX_TOOLWINDOW,
            dpi,
        );
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            r.right - r.left,
            r.bottom - r.top,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let dark = 1i32;
        windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            hwnd,
            20,
            (&dark as *const i32).cast(),
            4,
        );
        let notes = update.as_ref().map_or_else(
            || message.to_owned(),
            |u| {
                if u.notes.trim().is_empty() {
                    "Keine Release Notes hinterlegt.".into()
                } else {
                    u.notes
                        .replace("\r\n", "\n")
                        .replace('\n', "\r\n")
                        .replace('\0', "�")
                }
            },
        );
        let edit = child(
            hwnd,
            "EDIT",
            &notes,
            WS_TABSTOP
                | WS_VSCROLL
                | ES_MULTILINE as u32
                | ES_READONLY as u32
                | ES_AUTOVSCROLL as u32,
            NOTES,
            BOUNDS[0].1,
        );
        let download = if update.is_some() {
            child(
                hwnd,
                "BUTTON",
                "Update herunterladen",
                WS_TABSTOP | BS_OWNERDRAW as u32,
                DOWNLOAD,
                BOUNDS[1].1,
            )
        } else {
            null_mut()
        };
        let later = child(
            hwnd,
            "BUTTON",
            if update.is_some() {
                "Später"
            } else {
                "Schließen"
            },
            WS_TABSTOP | BS_OWNERDRAW as u32,
            LATER,
            BOUNDS[2].1,
        );
        if edit.is_null() || later.is_null() || (update.is_some() && download.is_null()) {
            DestroyWindow(hwnd);
            return;
        }
        STATE.with(|s| {
            if let Some(s) = s.borrow_mut().as_mut() {
                s.popup = hwnd;
                s.update = update;
            }
        });
        ShowWindow(hwnd, if manual { SW_SHOW } else { SW_SHOWNOACTIVATE });
        if manual {
            SetForegroundWindow(hwnd);
            SetFocus(later);
        }
    }
}
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let target = BeginPaint(hwnd, &mut ps);
            let mut r: RECT = zeroed();
            GetClientRect(hwnd, &mut r);
            let dpi = window_dpi(hwnd);
            let tag = STATE.with(|s| {
                s.borrow()
                    .as_ref()
                    .and_then(|s| s.update.as_ref().map(|u| u.tag.clone()))
            });
            let title = if tag.is_some() {
                "Neue Version verfügbar"
            } else {
                "Nach Updates suchen"
            };
            let version = tag.map_or_else(
                || format!("Installiert: {}", env!("CARGO_PKG_VERSION")),
                |v| {
                    format!(
                        "Installiert: {}    Verfügbar: {}",
                        env!("CARGO_PKG_VERSION"),
                        v
                    )
                },
            );
            let buffer = Buffer::new(target, r);
            let dc = buffer.as_ref().map_or(target, Buffer::dc);
            render_surface(dc, r, |s| {
                s.fill(r, BG);
                s.logo(scaled_rect((24, 18, 48, 48), dpi));
                for (id, label, b, color) in [
                    (SECTION, title, (86, 16, 430, 28), FG),
                    (
                        BRAND,
                        "DEUTSCHE DEADLOCK COMMUNITY",
                        (86, 44, 430, 22),
                        GOLD,
                    ),
                    (SUBTLE, version.as_str(), (24, 78, 492, 24), FG),
                    (SECTION, "Release Notes", (24, 104, 492, 24), MUTED),
                ] {
                    let v: Vec<u16> = label.encode_utf16().collect();
                    s.label(id, dpi, &v, scaled_rect(b, dpi), color, false);
                }
                s.outlined(
                    scaled_rect((20, 128, 500, 216), dpi),
                    FIELD,
                    LINE,
                    scale(8, dpi),
                    1,
                );
            });
            if let Some(b) = buffer {
                b.present(target, r);
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
            let dc = w as HDC;
            SetBkColor(dc, FIELD);
            SetTextColor(dc, FG);
            SetDCBrushColor(dc, FIELD);
            GetStockObject(DC_BRUSH) as LRESULT
        }
        WM_DRAWITEM if l != 0 => {
            let item = &*(l as *const DRAWITEMSTRUCT);
            let dpi = window_dpi(hwnd);
            let mut label = [0u16; 128];
            let n = GetWindowTextW(item.hwndItem, label.as_mut_ptr(), 128).max(0) as usize;
            render_surface(item.hDC, item.rcItem, |s| {
                s.fill(item.rcItem, BG);
                let primary = item.CtlID as usize == DOWNLOAD;
                let color = if primary { GOLD } else { CARD };
                s.rounded(
                    item.rcItem,
                    if item.itemState & 1 != 0 {
                        blend(color, FG, 1)
                    } else {
                        color
                    },
                    scale(8, dpi),
                );
                s.label(
                    SECTION,
                    dpi,
                    &label[..n],
                    item.rcItem,
                    if primary { BG } else { FG },
                    true,
                );
            });
            if item.itemState & 0x10 != 0 {
                let mut r = item.rcItem;
                InflateRect(&mut r, -3, -3);
                DrawFocusRect(item.hDC, &r);
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
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            for (id, b) in BOUNDS {
                let ch = GetDlgItem(hwnd, id as i32);
                if !ch.is_null() {
                    apply_control_layout(ch, b, id, dpi);
                }
            }
            InvalidateRect(hwnd, null(), 1);
            0
        }
        WM_COMMAND if (w >> 16) & 0xffff == 0 => {
            match w & 0xffff {
                DOWNLOAD => {
                    let url = STATE.with(|s| {
                        s.borrow()
                            .as_ref()
                            .and_then(|s| s.update.as_ref().map(|u| u.url.clone()))
                    });
                    if let Some(url) = url {
                        open_link(hwnd, &url);
                        close(hwnd);
                    }
                }
                LATER => close(hwnd),
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            close(hwnd);
            0
        }
        WM_DESTROY => {
            STATE.with(|s| {
                if let Some(s) = s.borrow_mut().as_mut() {
                    if s.popup == hwnd {
                        s.popup = null_mut();
                        s.update = None;
                    }
                }
            });
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}
pub(super) fn dialog_message(msg: &mut MSG) -> bool {
    let popup = STATE.with(|s| s.borrow().as_ref().map_or(null_mut(), |s| s.popup));
    !popup.is_null() && unsafe { IsDialogMessageW(popup, msg) != 0 }
}
