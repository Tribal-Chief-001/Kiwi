use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

static TERMINAL_RESTORED: AtomicBool = AtomicBool::new(false);

/// Restore terminal attributes, ensure cursor is visible, and clear synchronized output state.
pub fn restore_terminal() {
    if TERMINAL_RESTORED.swap(true, Ordering::SeqCst) {
        return;
    }
    let mut stdout = std::io::stdout().lock();
    // \x1b[?2025l : disable synchronized output
    // \x1b[?25h   : show cursor
    // \x1b[0m     : reset formatting
    let _ = stdout.write_all(b"\x1b[?2025l\x1b[?25h\x1b[0m\n");
    let _ = stdout.flush();
}

/// Reset cursor visibility specifically when drawing starts.
pub fn hide_cursor() {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(b"\x1b[?25l");
    let _ = stdout.flush();
    TERMINAL_RESTORED.store(false, Ordering::SeqCst);
}

/// Install synchronous panic hook to ensure terminal is 100% restored even under `panic = "abort"`.
pub fn setup_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        restore_terminal();
        default_hook(panic_info);
    }));
}

/// Windows timer precision helper: locks NT timer quantum to 1.0ms.
#[cfg(target_os = "windows")]
pub struct WindowsTimerGuard;

#[cfg(target_os = "windows")]
impl WindowsTimerGuard {
    pub fn new() -> Self {
        unsafe {
            windows_sys::Win32::Media::timeBeginPeriod(1);
        }
        WindowsTimerGuard
    }
}

#[cfg(target_os = "windows")]
impl Default for WindowsTimerGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "windows")]
impl Drop for WindowsTimerGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Media::timeEndPeriod(1);
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub struct WindowsTimerGuard;

#[cfg(not(target_os = "windows"))]
impl WindowsTimerGuard {
    pub fn new() -> Self {
        WindowsTimerGuard
    }
}

#[cfg(not(target_os = "windows"))]
impl Default for WindowsTimerGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn background signal listener for SIGINT (Ctrl-C), SIGTERM, and SIGHUP.
pub fn setup_signal_handlers() {
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let mut sigterm = signal(SignalKind::terminate()).ok();
            let mut sighup = signal(SignalKind::hangup()).ok();

            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    restore_terminal();
                    let _ = std::io::stderr().write_all(b"\n[!] Kiwi aborted by user signal (SIGINT).\n");
                    let _ = std::io::stderr().flush();
                    std::process::exit(130);
                }
                _ = async {
                    if let Some(ref mut st) = sigterm {
                        st.recv().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    restore_terminal();
                    let _ = std::io::stderr().write_all(b"\n[!] Kiwi terminated by signal (SIGTERM).\n");
                    let _ = std::io::stderr().flush();
                    std::process::exit(143);
                }
                _ = async {
                    if let Some(ref mut sh) = sighup {
                        sh.recv().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    restore_terminal();
                    std::process::exit(129);
                }
            }
        }

        #[cfg(not(unix))]
        {
            if tokio::signal::ctrl_c().await.is_ok() {
                restore_terminal();
                let _ = std::io::stderr().write_all(b"\n[!] Kiwi aborted by user signal.\n");
                let _ = std::io::stderr().flush();
                std::process::exit(130);
            }
        }
    });
}
