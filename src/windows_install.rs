//! Per-user installation from a manually downloaded executable. No updater
//! downloads code, elevates privileges, or forcibly terminates a process.
use crate::install_policy::{self, Action, Existing};
use std::{
    ffi::{OsStr, OsString},
    fs,
    mem::size_of,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::MetadataExt,
    },
    path::{Path, PathBuf},
    process::Command,
    ptr::{null, null_mut},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::*,
    Storage::FileSystem::*,
    System::{Com::CoTaskMemFree, Threading::*},
    UI::{
        Shell::{FOLDERID_LocalAppData, SHGetKnownFolderPath},
        WindowsAndMessaging::*,
    },
};

const FILE_NAME: &str = "live-match-collector.exe";
const PRODUCT: &str = "DeadlockLiveMatchCollector";
const REPARSE: u32 = FILE_ATTRIBUTE_REPARSE_POINT;
type Result<T> = std::result::Result<T, &'static str>;

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub(super) struct InstallGate(Handle);
impl InstallGate {
    pub(super) fn acquire(id: &str) -> Result<Self> {
        unsafe {
            let handle = CreateMutexW(
                null(),
                0,
                wide(OsStr::new(&format!("Local\\{id}-Install"))).as_ptr(),
            );
            if handle.is_null() {
                return Err("Update-Sperre konnte nicht erstellt werden.");
            }
            let handle = Handle(handle);
            match WaitForSingleObject(handle.0, 30_000) {
                WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self(handle)),
                _ => Err("Ein anderer Collector-Start oder ein Update läuft bereits."),
            }
        }
    }
}
impl Drop for InstallGate {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0 .0);
        }
    }
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

fn install_path() -> Result<PathBuf> {
    unsafe {
        let mut ptr = null_mut();
        if SHGetKnownFolderPath(&FOLDERID_LocalAppData, 0, null_mut(), &mut ptr) < 0
            || ptr.is_null()
        {
            return Err("Der Windows-Benutzerordner konnte nicht ermittelt werden.");
        }
        let len = (0..32768).find(|&i| *ptr.add(i) == 0);
        let path =
            len.map(|n| PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(ptr, n))));
        CoTaskMemFree(ptr.cast());
        let dir = path
            .ok_or("Ungültiger Windows-Benutzerordner.")?
            .join(PRODUCT);
        match fs::create_dir(&dir) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(_) => {
                return Err("Der Collector-Installationsordner konnte nicht erstellt werden.")
            }
        }
        let metadata =
            fs::symlink_metadata(&dir).map_err(|_| "Installationsordner nicht lesbar.")?;
        if !metadata.is_dir() || metadata.file_attributes() & REPARSE != 0 {
            return Err("Der Collector-Installationsordner ist kein regulärer Ordner.");
        }
        Ok(dir.join(FILE_NAME))
    }
}

fn version(path: &Path) -> Option<(u32, u32)> {
    unsafe {
        let path = wide(path.as_os_str());
        let mut ignored = 0;
        let size = GetFileVersionInfoSizeW(path.as_ptr(), &mut ignored);
        if !(1..=65_536).contains(&size) {
            return None;
        }
        // aligned storage for the Win32 version structure
        let mut data = vec![0u32; (size as usize).div_ceil(4)];
        if GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
            return None;
        }
        let mut ptr = null_mut();
        let mut len = 0;
        if VerQueryValueW(
            data.as_ptr().cast(),
            wide(OsStr::new("\\StringFileInfo\\040904B0\\ProductName")).as_ptr(),
            &mut ptr,
            &mut len,
        ) == 0
            || ptr.is_null()
            || len > 128
            || len == 0
        {
            return None;
        }
        let product = std::slice::from_raw_parts(ptr.cast::<u16>(), len as usize);
        if String::from_utf16_lossy(product).trim_end_matches('\0') != PRODUCT {
            return None;
        }
        if VerQueryValueW(
            data.as_ptr().cast(),
            wide(OsStr::new("\\")).as_ptr(),
            &mut ptr,
            &mut len,
        ) == 0
            || ptr.is_null()
            || len < size_of::<VS_FIXEDFILEINFO>() as u32
        {
            return None;
        }
        let info = ptr.cast::<VS_FIXEDFILEINFO>().read_unaligned();
        (info.dwSignature == 0xFEEF04BD).then_some((info.dwFileVersionMS, info.dwFileVersionLS))
    }
}

unsafe fn token_user(process: HANDLE) -> Option<Vec<usize>> {
    let mut token = null_mut();
    if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
        return None;
    }
    let token = Handle(token);
    let mut needed = 0;
    GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut needed);
    if !(1..=65536).contains(&needed) {
        return None;
    }
    let mut storage = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
    if GetTokenInformation(
        token.0,
        TokenUser,
        storage.as_mut_ptr().cast(),
        needed,
        &mut needed,
    ) == 0
    {
        return None;
    }
    Some(storage)
}

unsafe fn same_user(process: HANDLE) -> bool {
    let Some(own) = token_user(GetCurrentProcess()) else {
        return false;
    };
    let Some(other) = token_user(process) else {
        return false;
    };
    EqualSid(
        (*own.as_ptr().cast::<TOKEN_USER>()).User.Sid,
        (*other.as_ptr().cast::<TOKEN_USER>()).User.Sid,
    ) != 0
}

unsafe fn collector_process(hwnd: HWND) -> Result<(Handle, PathBuf)> {
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    let process = OpenProcess(
        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
        0,
        pid,
    );
    if process.is_null() {
        return Err("Der laufende Collector konnte nicht geprüft werden.");
    }
    let process = Handle(process);
    if !same_user(process.0) {
        return Err("Der laufende Collector gehört einem anderen Benutzer.");
    }
    let mut path = vec![0u16; 32768];
    let mut len = path.len() as u32;
    if QueryFullProcessImageNameW(process.0, 0, path.as_mut_ptr(), &mut len) == 0 {
        return Err("Der laufende Collector konnte nicht eindeutig erkannt werden.");
    }
    let path = PathBuf::from(OsString::from_wide(&path[..len as usize]));
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    // Legacy portable releases (before version resources) used this filename.
    // A matching class, same user and collector filename are all required.
    if version(&path).is_none()
        && !(name.starts_with("live-match-collector") && name.ends_with(".exe"))
    {
        return Err("Das laufende Programm konnte nicht als Collector erkannt werden.");
    }
    let mut actual = 0;
    GetWindowThreadProcessId(hwnd, &mut actual);
    if actual != pid {
        return Err("Der Collector wurde während des Updates neu gestartet.");
    }
    Ok((process, path))
}

fn stop_old(id: &str, exit_id: usize) -> Result<Option<PathBuf>> {
    let mut previous = None;
    let start = Instant::now();
    let mutex = wide(OsStr::new(&format!("Local\\{id}")));
    let class = wide(OsStr::new(id));
    loop {
        unsafe {
            let running = OpenMutexW(PROCESS_SYNCHRONIZE, 0, mutex.as_ptr());
            if running.is_null() {
                if GetLastError() == ERROR_FILE_NOT_FOUND {
                    return Ok(previous);
                }
                return Err("Instanzstatus konnte nicht geprüft werden.");
            }
            CloseHandle(running);
            let hwnd = FindWindowW(class.as_ptr(), null());
            if !hwnd.is_null() {
                let (process, path) = collector_process(hwnd)?;
                // The existing Tray exit handler joins the worker and releases
                // the instance guard. Never use TerminateProcess.
                if PostMessageW(hwnd, WM_COMMAND, exit_id, 0) == 0 {
                    return Err("Der laufende Collector konnte nicht beendet werden.");
                }
                let remaining = Duration::from_secs(30)
                    .saturating_sub(start.elapsed())
                    .as_millis()
                    .min(25_000) as u32;
                if WaitForSingleObject(process.0, remaining) != WAIT_OBJECT_0 {
                    return Err(
                        "Der alte Collector reagiert nicht. Er wurde nicht gewaltsam beendet.",
                    );
                }
                previous = Some(path);
            }
        }
        if start.elapsed() >= Duration::from_secs(30) {
            return Err("Der alte Collector ist noch beschäftigt.");
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn copy_new(source: &Path, dest: &Path) -> Result<()> {
    // CopyFileW retains alternate data streams, including Zone.Identifier.
    if unsafe {
        CopyFileW(
            wide(source.as_os_str()).as_ptr(),
            wide(dest.as_os_str()).as_ptr(),
            1,
        )
    } == 0
    {
        return Err("Die neue Collector-Datei konnte nicht kopiert werden.");
    }
    Ok(())
}
fn move_file(source: &Path, dest: &Path, replace: bool) -> Result<()> {
    let flags = MOVEFILE_WRITE_THROUGH
        | if replace {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    if unsafe {
        MoveFileExW(
            wide(source.as_os_str()).as_ptr(),
            wide(dest.as_os_str()).as_ptr(),
            flags,
        )
    } == 0
    {
        return Err("Die installierte EXE konnte nicht ersetzt werden.");
    }
    Ok(())
}
struct TempFile(PathBuf);
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

// Return true if the caller should run the ordinary desktop instance. A
// downloaded executable installs/starts the stable copy and then exits.
pub(super) fn prepare(
    id: &str,
    exit_id: usize,
    autostart: bool,
    auto_enabled: std::result::Result<bool, ()>,
) -> Result<bool> {
    let source =
        std::env::current_exe().map_err(|_| "Die eigene EXE konnte nicht ermittelt werden.")?;
    let target = install_path()?;
    let existing = match fs::symlink_metadata(&target) {
        Ok(m) if m.is_file() && m.file_attributes() & REPARSE == 0 => true,
        Ok(_) => return Err("Die vorhandene Installationsdatei ist keine reguläre EXE."),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => return Err("Die installierte EXE konnte nicht geprüft werden."),
    };
    if existing {
        let installed_path = fs::canonicalize(&target)
            .map_err(|_| "Installationspfad konnte nicht geprüft werden.")?;
        let source_path =
            fs::canonicalize(&source).map_err(|_| "Quellpfad konnte nicht geprüft werden.")?;
        if source_path == installed_path {
            return Ok(true);
        }
    }
    // A legacy HKCU autostart launch must never initiate a silent installation.
    if autostart {
        if existing && version(&target).is_some() {
            Command::new(&target)
                .arg("--autostart")
                .spawn()
                .map_err(|_| "Die installierte Version konnte nicht gestartet werden.")?;
            return Ok(false);
        }
        return Ok(true);
    }
    let incoming = version(&source).ok_or("Die Versionsinformation der neuen EXE fehlt.")?;
    let installed = if existing {
        version(&target).map_or(Existing::Unknown, Existing::Collector)
    } else {
        Existing::Missing
    };
    match install_policy::action(incoming, installed) {
        Action::Refuse => {
            return Err(
                "Die vorhandene Datei ist kein erkannter Collector. Sie wird nicht überschrieben.",
            )
        }
        Action::LaunchInstalled => {
            Command::new(&target)
                .spawn()
                .map_err(|_| "Die installierte Version konnte nicht gestartet werden.")?;
            return Ok(false);
        }
        Action::Install | Action::Replace => (),
    }
    let auto_enabled = auto_enabled.map_err(|_| {
        "Autostart-Status konnte nicht gelesen werden. Das Update wurde nicht begonnen."
    })?;
    let dir = target.parent().ok_or("Installationsordner fehlt.")?;
    let nonce = uuid::Uuid::new_v4();
    let stage = dir.join(format!("update-{nonce}.tmp"));
    copy_new(&source, &stage)?;
    let stage = TempFile(stage);
    if version(&stage.0) != Some(incoming)
        || fs::read(&source)
            .ok()
            .zip(fs::read(&stage.0).ok())
            .is_none_or(|(a, b)| a != b)
    {
        return Err("Die kopierte EXE konnte nicht geprüft werden.");
    }
    let backup = if existing {
        let path = dir.join(format!("backup-{nonce}.tmp"));
        copy_new(&target, &path)?;
        Some(TempFile(path))
    } else {
        None
    };
    let previous = stop_old(id, exit_id)?;
    if let Err(e) = move_file(&stage.0, &target, existing) {
        if let Some(previous) = previous.as_ref() {
            let _ = Command::new(previous).spawn();
        }
        return Err(e);
    }
    if Command::new(&target).spawn().is_err() {
        if let Some(backup) = backup {
            if move_file(&backup.0, &target, true).is_ok() {
                let _ = Command::new(previous.as_deref().unwrap_or(&target)).spawn();
            } else {
                // Retain recovery file if rollback itself fails.
                std::mem::forget(backup);
            }
        } else {
            // First migration: retain the old portable file and restore it on launch failure.
            let _ = fs::remove_file(&target);
            if let Some(previous) = previous.as_ref() {
                let _ = Command::new(previous).spawn();
            }
        }
        return Err("Die neue Version konnte nicht gestartet werden. Bitte erneut versuchen.");
    }
    if auto_enabled && super::windows_client::set_auto_path(true, &target).is_err() {
        return Err("Update installiert. Der Autostart konnte nicht auf den neuen Pfad umgestellt werden. Bitte in den Einstellungen neu aktivieren.");
    }
    Ok(false)
}
