use crate::pty::PtyHandle;
use ratatui::layout::Rect;
use std::path::PathBuf;

/// An embedded terminal session, independent from sidebar selection.
pub struct TerminalSession {
    /// Working directory where the session was started.
    pub cwd: PathBuf,
    /// Display title for the terminal.
    pub title: String,
    /// PTY handle for the child process.
    pub pty_handle: PtyHandle,
    /// Terminal state machine (screen buffer + parser).
    pub vt100_parser: vt100::Parser,
    /// Last rendered viewport area (for resize detection).
    pub viewport_rect: Option<Rect>,
}
