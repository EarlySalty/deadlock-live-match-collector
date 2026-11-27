#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    #[cfg(windows)]
    live_match_collector::run_windows();
    #[cfg(not(windows))]
    live_match_collector::run_cli();
}
