use clap::Parser;
use kiwi::cli::Cli;
use kiwi::engine;
use kiwi::safety::{restore_terminal, setup_panic_hook, setup_signal_handlers, WindowsTimerGuard};

fn main() {
    // Install panic hook early to ensure terminal cursor restoration on any crash
    setup_panic_hook();

    // Lock NT timer quantum to 1.0ms on Windows (no-op on Linux/macOS)
    let _timer_guard = WindowsTimerGuard::new();

    let cli = Cli::parse();

    // Single-thread Tokio runtime for sub-4MB RSS and deterministic zero-overhead event loop
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[!] Failed to build Tokio single-thread runtime: {}", e);
            std::process::exit(1);
        }
    };

    runtime.block_on(async {
        setup_signal_handlers();

        if let Err(e) = engine::run_benchmark(&cli).await {
            restore_terminal();
            eprintln!("[!] Kiwi benchmark failed: {}", e);
            std::process::exit(1);
        }
    });
}
