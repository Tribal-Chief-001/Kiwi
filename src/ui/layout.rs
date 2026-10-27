use crossterm::terminal::size;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    Full,
    Compact,
    UltraDense,
}

pub struct TerminalLayout {
    pub mode: LayoutMode,
    pub cols: u16,
    #[allow(dead_code)]
    pub rows: u16,
    pub is_ssh: bool,
    pub fps: u64,
}

impl TerminalLayout {
    pub fn current(low_bandwidth_flag: bool) -> Self {
        let (cols, rows) = size().unwrap_or((80, 24));
        let is_ssh = low_bandwidth_flag
            || std::env::var("SSH_CLIENT").is_ok()
            || std::env::var("SSH_TTY").is_ok()
            || std::env::var("SSH_CONNECTION").is_ok();

        let mode = if cols < 50 || rows < 8 {
            LayoutMode::UltraDense
        } else if cols < 80 {
            LayoutMode::Compact
        } else {
            LayoutMode::Full
        };

        let fps = if is_ssh { 10 } else { 20 };

        TerminalLayout {
            mode,
            cols,
            rows,
            is_ssh,
            fps,
        }
    }
}
