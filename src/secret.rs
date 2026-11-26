//! Bounded, non-echoing ASCII service-key input. No argv/env/file key configuration.
use std::io::{self, IsTerminal, Read, Write};
use zeroize::{Zeroize, Zeroizing};

const MAX_INPUT: usize = 128;
const UNREADABLE: &str = "Der Verbindungsschlüssel konnte nicht gelesen werden.";
const INVALID: &str = "Der Verbindungsschlüssel hat ein ungültiges Format.";

fn read_key(
    mut next: impl FnMut() -> io::Result<Option<u8>>,
    terminal: bool,
) -> Result<Zeroizing<String>, &'static str> {
    let mut input = Zeroizing::new([0u8; MAX_INPUT]);
    let mut len = 0;
    // Bound total input work as well as retained bytes, including editing keys.
    for _ in 0..MAX_INPUT * 4 {
        let Some(byte) = next().map_err(|_| UNREADABLE)? else {
            if terminal || len == 0 {
                return Err(UNREADABLE);
            }
            return validate(&input[..len]);
        };
        match byte {
            b'\n' => return validate(&input[..len]),
            b'\r' if terminal => return validate(&input[..len]),
            3 | 4 if terminal => return Err(UNREADABLE),
            8 | 127 if terminal => {
                if len > 0 {
                    len -= 1;
                    input[len].zeroize();
                }
            }
            _ if len == MAX_INPUT => return Err(INVALID),
            _ => {
                input[len] = byte;
                len += 1;
            }
        }
    }
    Err(INVALID)
}

fn validate(input: &[u8]) -> Result<Zeroizing<String>, &'static str> {
    let token = std::str::from_utf8(input).map_err(|_| INVALID)?.trim();
    if !token
        .strip_prefix("dlm_")
        .is_some_and(|suffix| suffix.len() == 64 && suffix.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(INVALID);
    }
    Ok(Zeroizing::new(token.to_owned()))
}

fn prompt() -> Result<(), &'static str> {
    let mut output = io::stderr().lock();
    write!(output, "Verbindungsschlüssel: ")
        .and_then(|_| output.flush())
        .map_err(|_| UNREADABLE)
}

fn next_byte(reader: &mut impl Read) -> io::Result<Option<u8>> {
    let mut byte = Zeroizing::new([0u8; 1]);
    match reader.read(&mut *byte)? {
        0 => Ok(None),
        _ => Ok(Some(byte[0])),
    }
}

pub fn read_token() -> Result<Zeroizing<String>, &'static str> {
    if !io::stdin().is_terminal() {
        return read_key(|| next_byte(&mut io::stdin().lock()), false);
    }
    let result = console_key();
    let _ = writeln!(io::stderr());
    result
}

#[cfg(unix)]
fn console_key() -> Result<Zeroizing<String>, &'static str> {
    use std::os::fd::{AsRawFd, RawFd};
    struct Restore(RawFd, libc::termios);
    impl Drop for Restore {
        fn drop(&mut self) {
            // SAFETY: descriptor remains open until after this guard drops.
            unsafe {
                libc::tcsetattr(self.0, libc::TCSAFLUSH, &self.1);
            }
        }
    }
    let mut tty = std::fs::File::open("/dev/tty").map_err(|_| UNREADABLE)?;
    let fd = tty.as_raw_fd();
    let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
    // SAFETY: tcgetattr initializes the structure on success.
    if unsafe { libc::tcgetattr(fd, original.as_mut_ptr()) } != 0 {
        return Err(UNREADABLE);
    }
    let original = unsafe { original.assume_init() };
    let _restore = Restore(fd, original);
    let mut mode = original;
    mode.c_lflag &= !(libc::ECHO | libc::ECHONL | libc::ICANON | libc::ISIG);
    mode.c_cc[libc::VMIN] = 1;
    mode.c_cc[libc::VTIME] = 0;
    // SAFETY: valid terminal descriptor and initialized termios.
    if unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &mode) } != 0 {
        return Err(UNREADABLE);
    }
    prompt()?;
    read_key(|| next_byte(&mut tty), true)
}

#[cfg(windows)]
fn console_key() -> Result<Zeroizing<String>, &'static str> {
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Console::*;
    struct Restore(HANDLE, u32);
    impl Drop for Restore {
        fn drop(&mut self) {
            // SAFETY: borrowed process console handle, never closed by this guard.
            unsafe {
                FlushConsoleInputBuffer(self.0);
                SetConsoleMode(self.0, self.1);
            }
        }
    }
    // SAFETY: only queries/modifies the current process's own console.
    let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(UNREADABLE);
    }
    let mut original = 0;
    if unsafe { GetConsoleMode(handle, &mut original) } == 0 {
        return Err(UNREADABLE);
    }
    let _restore = Restore(handle, original);
    let mode = original & !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT);
    if unsafe { SetConsoleMode(handle, mode) } == 0 {
        return Err(UNREADABLE);
    }
    prompt()?;
    read_key(
        || {
            let mut unit = Zeroizing::new([0u16; 1]);
            let mut count = 0;
            // SAFETY: one UTF-16 code unit, valid buffer and count pointer.
            if unsafe {
                ReadConsoleW(
                    handle,
                    unit.as_mut_ptr().cast(),
                    1,
                    &mut count,
                    std::ptr::null(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            if count == 0 {
                return Ok(None);
            }
            // Keys are ASCII; never silently remove or replace non-ASCII input.
            if unit[0] > 127 {
                return Err(io::ErrorKind::InvalidData.into());
            }
            Ok(Some(unit[0] as u8))
        },
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(bytes: &[u8], terminal: bool) -> Result<Zeroizing<String>, &'static str> {
        let mut reader = bytes;
        read_key(|| next_byte(&mut reader), terminal)
    }
    #[test]
    fn valid_key_and_crlf_or_eof() {
        let key = format!("dlm_{}", "aB09".repeat(16));
        for suffix in ["", "\n", "\r\n"] {
            assert_eq!(
                &*parse(format!(" {key}{suffix}").as_bytes(), false).unwrap(),
                &key
            );
        }
        assert_eq!(&*parse(format!("{key}\r").as_bytes(), true).unwrap(), &key);
        assert!(parse(key.as_bytes(), true).is_err());
    }
    #[test]
    fn rejects_invalid_and_overlong_without_accepting_prefix() {
        let key = format!("dlm_{}", "a".repeat(64));
        for bytes in [
            b"".as_slice(),
            b"dlm_bad",
            b"\xff",
            b"dlm_\0",
            format!("{key}g\n").as_bytes(),
            format!("{key}{}\n", " ".repeat(MAX_INPUT)).as_bytes(),
        ] {
            assert!(parse(bytes, false).is_err());
        }
        let mut calls = 0;
        assert!(read_key(
            || {
                calls += 1;
                Ok(Some(b'a'))
            },
            false
        )
        .is_err());
        assert_eq!(calls, MAX_INPUT + 1);
        assert!(parse(b"\x03", true).is_err());
    }
    #[test]
    fn terminal_editing_is_bounded() {
        let key = format!("dlm_{}", "f".repeat(64));
        assert_eq!(
            &*parse(format!("x\x08{key}\n").as_bytes(), true).unwrap(),
            &key
        );
        let mut calls = 0;
        assert!(read_key(
            || {
                calls += 1;
                Ok(Some(8))
            },
            true
        )
        .is_err());
        assert_eq!(calls, MAX_INPUT * 4);
    }
}
