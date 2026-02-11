use crate::cli::{switch_workspace, WeztermCli};
use crate::config::Config;
use crate::datasource::git::GitBranchCache;
use crate::datasource::{
    PaneDataSource, ProcessDataSource, SystemProcessDataSource, WeztermDataSource,
};
use crate::detector::ClaudeCodeDetector;
use crate::session_mapping::SessionMapping;
use crate::transcript::{ConversationTurn, TranscriptWatcher};
use anyhow::Result;
use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        KeyCode, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    widgets::ListState,
    Terminal,
};
use std::collections::HashMap;
use std::io;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use super::event::{
    is_down_key, is_enter_key, is_quit_key, is_refresh_key, is_up_key, Event, EventHandler,
};
use super::input_buffer::InputBuffer;
use super::render::{
    render_details, render_footer, render_list, DetailMode, DetailsRenderCtx, LivePaneLinesCache,
};
use super::session::ClaudeSession;
use super::toast::Toast;

#[path = "app/actions.rs"]
mod actions;
#[path = "app/navigation.rs"]
mod navigation;
#[path = "app/runtime.rs"]
mod runtime;

/// Debounce interval (ms) for transcript file refreshes.
/// 200ms keeps the status responsive while coalescing burst writes during streaming.
const TRANSCRIPT_DEBOUNCE_MS: u64 = 200;

/// Polling interval (ms) for live pane content refresh.
const LIVE_PANE_POLL_MS: u64 = 300;

/// Circuit-breaker: seconds to skip polling after a failure.
const LIVE_PANE_COOLDOWN_SECS: u64 = 5;

/// Number of consecutive poll failures before auto-exiting live pane view.
const LIVE_PANE_MAX_FAILURES: u32 = 3;

/// TUI application
pub struct App {
    /// Claude Code session list
    sessions: Vec<ClaudeSession>,
    /// List selection state
    list_state: ListState,
    /// Data sources
    pane_ds: WeztermDataSource,
    process_ds: SystemProcessDataSource,
    detector: ClaudeCodeDetector,
    /// Dirty flag (needs redraw)
    dirty: bool,
    /// Refreshing flag
    refreshing: bool,
    /// Needs full redraw (to prevent artifacts on selection change)
    needs_full_redraw: bool,
    /// 'g' key pressed state (for gg sequence)
    pending_g: bool,
    /// Previous last_output snapshot (for change detection)
    prev_last_outputs: Vec<Option<String>>,
    /// Last click time and index (for double click detection)
    last_click: Option<(std::time::Instant, usize)>,
    /// List area Rect (for click position calculation)
    list_area: Option<Rect>,
    /// File watcher for transcript changes
    transcript_watcher: Option<TranscriptWatcher>,
    /// Animation frame counter for Processing status indicator (0-3)
    animation_frame: u8,
    /// Current workspace name (for detecting cross-workspace jumps)
    current_workspace: String,
    /// Details panel width percentage (default: 65, range: 20-80)
    details_width_percent: u16,
    /// Input mode (for sending prompts to sessions)
    input_mode: bool,
    /// Input buffer with cursor management
    input_buffer: InputBuffer,
    /// Toast notification
    toast: Option<Toast>,
    /// Kill confirmation mode (stores pane_id and display label)
    kill_confirm: Option<(u32, String)>,
    /// Add pane mode: stores (pane_id, cwd, window_id) for split direction selection
    add_pane_pending: Option<(u32, String, u32)>,
    /// Detail panel display mode
    detail_mode: DetailMode,
    /// Conversation turns for history browsing (newest first)
    history_turns: Vec<ConversationTurn>,
    /// Selection state for history list view
    history_list_state: ListState,
    /// Current history index for detail view (0 = newest)
    history_index: usize,
    /// Scroll offset within the current history turn detail (line-level)
    history_scroll_offset: usize,
    /// Pre-parsed timestamps for history turns (avoids per-frame parsing)
    history_timestamps: Vec<Option<SystemTime>>,
    /// Cached rendered lines for history detail view: ((text_hash, width), lines)
    cached_history_lines: Option<((u64, usize), Vec<ratatui::text::Line<'static>>)>,
    /// Cached rendered lines for details preview: ((text_hash, width, max_lines), lines)
    cached_preview_lines: Option<((u64, usize, usize), Vec<ratatui::text::Line<'static>>)>,
    /// Cached raw ANSI bytes from wezterm cli get-text --escapes
    live_pane_bytes: Option<Vec<u8>>,
    /// Scroll offset within live pane view (line-level)
    live_pane_scroll_offset: usize,
    /// Cached rendered lines for live pane view
    cached_live_pane_lines: LivePaneLinesCache,
    /// Timestamp of the last live pane content fetch
    last_live_pane_fetch: Instant,
    /// Consecutive poll failure count (circuit-breaker)
    live_pane_poll_failures: u32,
    /// User configuration loaded from ~/.config/wzcc/config.toml
    config: Config,
    /// Git branch cache (30s TTL)
    git_branch_cache: GitBranchCache,
    /// Last time a transcript-only refresh was performed (for debouncing)
    last_transcript_refresh: Instant,
    /// Whether a transcript refresh is pending (trailing-edge debounce)
    pending_transcript_refresh: bool,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));

        let (config, config_warning) = match Config::load() {
            Ok(c) => (c, None),
            Err(e) => (Config::default(), Some(format!("Config warning: {}", e))),
        };

        let toast = config_warning.map(Toast::error);

        Self {
            sessions: Vec::new(),
            list_state,
            pane_ds: WeztermDataSource::new(),
            process_ds: SystemProcessDataSource::new(),
            detector: ClaudeCodeDetector::new(),
            dirty: true,
            refreshing: false,
            needs_full_redraw: true,
            pending_g: false,
            prev_last_outputs: Vec::new(),
            last_click: None,
            list_area: None,
            transcript_watcher: None,
            animation_frame: 0,
            current_workspace: String::new(),
            details_width_percent: 65,
            input_mode: false,
            input_buffer: InputBuffer::new(),
            toast,
            kill_confirm: None,
            add_pane_pending: None,
            detail_mode: DetailMode::Summary,
            history_turns: Vec::new(),
            history_list_state: ListState::default(),
            history_index: 0,
            history_scroll_offset: 0,
            history_timestamps: Vec::new(),
            cached_history_lines: None,
            cached_preview_lines: None,
            live_pane_bytes: None,
            live_pane_scroll_offset: 0,
            cached_live_pane_lines: None,
            last_live_pane_fetch: Instant::now(),
            live_pane_poll_failures: 0,
            config,
            git_branch_cache: GitBranchCache::new(30),
            last_transcript_refresh: Instant::now(),
            pending_transcript_refresh: false,
        }
    }

    /// Clean up session mapping files for TTYs that no longer exist.
    ///
    /// This is called at startup to remove stale mappings from previous
    /// WezTerm sessions. Only removes mappings for TTYs that are definitely
    /// not in use by any current pane.
    fn cleanup_inactive_session_mappings(&self) {
        // Get list of all current TTYs from WezTerm
        let active_ttys: Vec<String> = match self.pane_ds.list_panes() {
            Ok(panes) => panes.iter().filter_map(|p| p.tty_short()).collect(),
            Err(_) => return, // If we can't list panes, don't clean up anything
        };

        // Clean up mappings for inactive TTYs
        SessionMapping::cleanup_inactive_ttys(&active_ttys);
    }

    /// Update watched directories based on current sessions.
    fn update_watched_dirs(&mut self) -> Result<()> {
        let cwds: Vec<String> = self
            .sessions
            .iter()
            .filter_map(|s| s.pane.cwd_path())
            .collect();

        if let Some(watcher) = &mut self.transcript_watcher {
            watcher.update_dirs(&cwds)?;
        }

        Ok(())
    }

    /// Drain file change events and return true if any were received.
    fn drain_file_changes(&self) -> bool {
        self.transcript_watcher
            .as_ref()
            .is_some_and(|w| w.drain_changes())
    }

    /// Extract current workspace from pane list (avoids redundant wezterm CLI call)
    fn extract_current_workspace(panes: &[crate::models::Pane]) -> Option<String> {
        let current_pane_id = std::env::var("WEZTERM_PANE").ok()?.parse::<u32>().ok()?;
        panes
            .iter()
            .find(|p| p.pane_id == current_pane_id)
            .map(|p| p.workspace.clone())
    }

    /// Apply duplicate CWD guard: clear last_prompt/last_output for sessions
    /// that share the same CWD without statusLine bridge mapping.
    fn apply_duplicate_cwd_guard(&mut self) {
        apply_duplicate_cwd_guard(&mut self.sessions);
    }

    /// Lightweight refresh: only re-read transcript data for known sessions.
    /// Does NOT call wezterm CLI, ps, or git. Only re-reads transcript files.
    fn refresh_transcripts(&mut self) {
        for session in &mut self.sessions {
            let info = crate::transcript::detect_session_info(&session.pane);
            session.status = info.status;
            session.last_prompt = info.last_prompt;
            session.last_output = info.last_output;
            session.updated_at = info.updated_at;
            session.warning = info.warning;
            session.session_id = info.session_id;
            session.transcript_path = info.transcript_path;
        }
        self.apply_duplicate_cwd_guard();
        self.dirty = true;
    }

    /// Check if enough time has passed for a debounced transcript refresh.
    /// Uses trailing-edge debounce: if not enough time passed, sets pending flag.
    fn should_refresh_transcripts(&mut self) -> bool {
        let debounce = Duration::from_millis(TRANSCRIPT_DEBOUNCE_MS);
        if self.last_transcript_refresh.elapsed() >= debounce {
            self.pending_transcript_refresh = false;
            self.last_transcript_refresh = Instant::now();
            true
        } else {
            self.pending_transcript_refresh = true;
            false
        }
    }

    /// Refresh session list
    pub fn refresh(&mut self) -> Result<()> {
        // Preserve currently selected pane_id
        let selected_pane_id = self
            .list_state
            .selected()
            .and_then(|i| self.sessions.get(i))
            .map(|s| s.pane.pane_id);

        // Get all panes (single call, also used to extract workspace)
        let panes = self.pane_ds.list_panes()?;

        // Extract workspace from pane list (avoids redundant wezterm CLI call)
        self.current_workspace = Self::extract_current_workspace(&panes)
            .unwrap_or_else(|| self.current_workspace.clone());

        // Build process tree once (optimization)
        let process_tree = self.process_ds.build_tree()?;

        self.sessions = panes
            .into_iter()
            .filter_map(|pane| {
                // Try to detect Claude Code (reusing process tree)
                let reason = self
                    .detector
                    .detect_by_tty_with_tree(&pane, &process_tree)
                    .ok()??;

                // Get session info (uses statusLine bridge if available, falls back to CWD-based)
                let session_info = crate::transcript::detect_session_info(&pane);

                // Keep only detected sessions (git_branch filled below)
                Some(ClaudeSession {
                    pane,
                    detected: true,
                    reason,
                    status: session_info.status,
                    git_branch: None,
                    last_prompt: session_info.last_prompt,
                    last_output: session_info.last_output,
                    session_id: session_info.session_id,
                    transcript_path: session_info.transcript_path,
                    updated_at: session_info.updated_at,
                    warning: session_info.warning,
                })
            })
            .collect();

        // Fill in git branches with caching (separate loop to avoid borrow issues)
        for session in &mut self.sessions {
            if let Some(cwd) = session.pane.cwd_path() {
                session.git_branch = self.git_branch_cache.get(&cwd);
            }
        }

        // Apply duplicate CWD guard
        self.apply_duplicate_cwd_guard();

        // Sort by workspace → cwd → pane_id (current workspace first)
        sort_sessions(&mut self.sessions, &self.current_workspace);

        // Maintain selection position (reselect if same pane_id exists)
        if !self.sessions.is_empty() {
            let new_index = selected_pane_id
                .and_then(|id| self.sessions.iter().position(|s| s.pane.pane_id == id))
                .unwrap_or(0);
            self.list_state.select(Some(new_index));
        } else {
            self.list_state.select(None);
        }

        self.dirty = true;

        // If in live pane view, check if the selected session's pane_id changed
        if self.detail_mode == DetailMode::LivePane {
            let new_pane_id = self
                .list_state
                .selected()
                .and_then(|i| self.sessions.get(i))
                .map(|s| s.pane.pane_id);
            if new_pane_id != selected_pane_id {
                self.exit_live_pane_view();
            }
        }

        Ok(())
    }
}

/// Apply duplicate CWD guard: clear last_prompt/last_output for sessions
/// that share the same CWD without statusLine bridge mapping.
fn apply_duplicate_cwd_guard(sessions: &mut [ClaudeSession]) {
    let mut cwd_counts: HashMap<String, usize> = HashMap::new();
    for session in sessions.iter() {
        if session.session_id.is_none() && session.warning.is_none() {
            if let Some(cwd) = session.pane.cwd_path() {
                *cwd_counts.entry(cwd).or_insert(0) += 1;
            }
        }
    }

    for session in sessions.iter_mut() {
        if session.session_id.is_some() || session.warning.is_some() {
            continue;
        }
        if let Some(cwd) = session.pane.cwd_path() {
            if cwd_counts.get(&cwd).copied().unwrap_or(0) > 1 {
                session.last_prompt = None;
                session.last_output =
                    Some("Run `wzcc install-bridge` for multi-session support".to_string());
            }
        }
    }
}

/// Calculate session index from list display row.
/// Returns the session corresponding to the clicked row, considering group headers.
fn row_to_session_index(sessions: &[ClaudeSession], row: usize) -> Option<usize> {
    let mut current_row = 0;
    let mut current_ws: Option<String> = None;
    let mut current_cwd: Option<String> = None;

    for (session_idx, session) in sessions.iter().enumerate() {
        let ws = &session.pane.workspace;
        let cwd = session.pane.cwd_path().unwrap_or_default();

        // Workspace header row
        if current_ws.as_ref() != Some(ws) {
            current_ws = Some(ws.clone());
            current_cwd = None;
            if current_row == row {
                return None; // header click
            }
            current_row += 1;
        }

        // CWD header row
        if current_cwd.as_ref() != Some(&cwd) {
            current_cwd = Some(cwd.clone());
            if current_row == row {
                return None; // header click
            }
            current_row += 1;
        }

        // Session row
        if current_row == row {
            return Some(session_idx);
        }
        current_row += 1;
    }

    None
}

/// Sort sessions: current workspace first, then by workspace name, CWD, pane_id.
fn sort_sessions(sessions: &mut [ClaudeSession], current_workspace: &str) {
    sessions.sort_by(|a, b| {
        let ws_a_is_current = a.pane.workspace == current_workspace;
        let ws_b_is_current = b.pane.workspace == current_workspace;
        match (ws_a_is_current, ws_b_is_current) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => {
                let ws_a = &a.pane.workspace;
                let ws_b = &b.pane.workspace;
                let cwd_a = a.pane.cwd_path().unwrap_or_default();
                let cwd_b = b.pane.cwd_path().unwrap_or_default();
                ws_a.cmp(ws_b)
                    .then(cwd_a.cmp(&cwd_b))
                    .then(a.pane.pane_id.cmp(&b.pane.pane_id))
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detector::DetectionReason;
    use crate::models::Pane;
    use crate::transcript::SessionStatus;

    fn make_pane(pane_id: u32, workspace: &str, cwd: &str) -> Pane {
        Pane {
            pane_id,
            tab_id: 0,
            window_id: 0,
            workspace: workspace.to_string(),
            title: format!("pane-{}", pane_id),
            cwd: Some(format!("file://{}", cwd)),
            tty_name: None,
            is_active: false,
            tab_title: None,
            window_title: None,
        }
    }

    fn make_session(pane_id: u32, workspace: &str, cwd: &str) -> ClaudeSession {
        ClaudeSession {
            pane: make_pane(pane_id, workspace, cwd),
            detected: true,
            reason: DetectionReason::DirectTtyMatch {
                process_name: "claude".to_string(),
            },
            status: SessionStatus::Idle,
            git_branch: None,
            last_prompt: Some("test prompt".to_string()),
            last_output: Some("test output".to_string()),
            session_id: None,
            transcript_path: None,
            updated_at: None,
            warning: None,
        }
    }

    fn make_session_with_mapping(
        pane_id: u32,
        workspace: &str,
        cwd: &str,
        session_id: &str,
    ) -> ClaudeSession {
        let mut s = make_session(pane_id, workspace, cwd);
        s.session_id = Some(session_id.to_string());
        s
    }

    fn make_session_with_warning(pane_id: u32, workspace: &str, cwd: &str) -> ClaudeSession {
        let mut s = make_session(pane_id, workspace, cwd);
        s.warning = Some("stale".to_string());
        s
    }

    // --- row_to_session_index tests ---

    #[test]
    fn test_row_to_session_single_session() {
        // Layout: row 0 = workspace header, row 1 = cwd header, row 2 = session
        let sessions = vec![make_session(1, "default", "/home/user/project")];
        assert_eq!(row_to_session_index(&sessions, 0), None); // workspace header
        assert_eq!(row_to_session_index(&sessions, 1), None); // cwd header
        assert_eq!(row_to_session_index(&sessions, 2), Some(0)); // session
        assert_eq!(row_to_session_index(&sessions, 3), None); // out of bounds
    }

    #[test]
    fn test_row_to_session_same_workspace_same_cwd() {
        // Two sessions, same workspace, same cwd
        // row 0 = ws header, row 1 = cwd header, row 2 = session 0, row 3 = session 1
        let sessions = vec![
            make_session(1, "default", "/home/user/project"),
            make_session(2, "default", "/home/user/project"),
        ];
        assert_eq!(row_to_session_index(&sessions, 0), None);
        assert_eq!(row_to_session_index(&sessions, 1), None);
        assert_eq!(row_to_session_index(&sessions, 2), Some(0));
        assert_eq!(row_to_session_index(&sessions, 3), Some(1));
    }

    #[test]
    fn test_row_to_session_same_workspace_different_cwd() {
        // row 0 = ws header, row 1 = cwd1 header, row 2 = session 0,
        // row 3 = cwd2 header, row 4 = session 1
        let sessions = vec![
            make_session(1, "default", "/home/user/project-a"),
            make_session(2, "default", "/home/user/project-b"),
        ];
        assert_eq!(row_to_session_index(&sessions, 0), None); // ws header
        assert_eq!(row_to_session_index(&sessions, 1), None); // cwd1 header
        assert_eq!(row_to_session_index(&sessions, 2), Some(0));
        assert_eq!(row_to_session_index(&sessions, 3), None); // cwd2 header
        assert_eq!(row_to_session_index(&sessions, 4), Some(1));
    }

    #[test]
    fn test_row_to_session_different_workspaces() {
        // row 0 = ws1 header, row 1 = cwd header, row 2 = session 0,
        // row 3 = ws2 header, row 4 = cwd header, row 5 = session 1
        let sessions = vec![
            make_session(1, "work", "/home/user/project"),
            make_session(2, "personal", "/home/user/hobby"),
        ];
        assert_eq!(row_to_session_index(&sessions, 0), None); // ws1 header
        assert_eq!(row_to_session_index(&sessions, 1), None); // cwd header
        assert_eq!(row_to_session_index(&sessions, 2), Some(0));
        assert_eq!(row_to_session_index(&sessions, 3), None); // ws2 header
        assert_eq!(row_to_session_index(&sessions, 4), None); // cwd header
        assert_eq!(row_to_session_index(&sessions, 5), Some(1));
    }

    #[test]
    fn test_row_to_session_empty() {
        let sessions: Vec<ClaudeSession> = vec![];
        assert_eq!(row_to_session_index(&sessions, 0), None);
    }

    // --- apply_duplicate_cwd_guard tests ---

    #[test]
    fn test_duplicate_cwd_guard_clears_output() {
        let mut sessions = vec![
            make_session(1, "default", "/home/user/project"),
            make_session(2, "default", "/home/user/project"),
        ];
        apply_duplicate_cwd_guard(&mut sessions);
        assert_eq!(sessions[0].last_prompt, None);
        assert_eq!(
            sessions[0].last_output.as_deref(),
            Some("Run `wzcc install-bridge` for multi-session support")
        );
        assert_eq!(sessions[1].last_prompt, None);
    }

    #[test]
    fn test_duplicate_cwd_guard_different_cwd_untouched() {
        let mut sessions = vec![
            make_session(1, "default", "/home/user/project-a"),
            make_session(2, "default", "/home/user/project-b"),
        ];
        apply_duplicate_cwd_guard(&mut sessions);
        // Different CWDs -> no guard applied
        assert_eq!(sessions[0].last_prompt.as_deref(), Some("test prompt"));
        assert_eq!(sessions[1].last_prompt.as_deref(), Some("test prompt"));
    }

    #[test]
    fn test_duplicate_cwd_guard_with_mapping_skipped() {
        let mut sessions = vec![
            make_session_with_mapping(1, "default", "/home/user/project", "sess-1"),
            make_session(2, "default", "/home/user/project"),
        ];
        apply_duplicate_cwd_guard(&mut sessions);
        // Session with mapping is excluded from counting -> only 1 unmapped session
        // so no guard applied to either
        assert_eq!(sessions[0].last_prompt.as_deref(), Some("test prompt"));
        assert_eq!(sessions[1].last_prompt.as_deref(), Some("test prompt"));
    }

    #[test]
    fn test_duplicate_cwd_guard_with_warning_skipped() {
        let mut sessions = vec![
            make_session_with_warning(1, "default", "/home/user/project"),
            make_session(2, "default", "/home/user/project"),
        ];
        apply_duplicate_cwd_guard(&mut sessions);
        // Session with warning is excluded from counting
        assert_eq!(sessions[0].last_prompt.as_deref(), Some("test prompt"));
        assert_eq!(sessions[1].last_prompt.as_deref(), Some("test prompt"));
    }

    #[test]
    fn test_duplicate_cwd_guard_three_sessions_same_cwd() {
        let mut sessions = vec![
            make_session(1, "default", "/home/user/project"),
            make_session(2, "default", "/home/user/project"),
            make_session(3, "default", "/home/user/project"),
        ];
        apply_duplicate_cwd_guard(&mut sessions);
        for s in &sessions {
            assert_eq!(s.last_prompt, None);
        }
    }

    // --- sort_sessions tests ---

    #[test]
    fn test_sort_current_workspace_first() {
        let mut sessions = vec![
            make_session(1, "other", "/tmp"),
            make_session(2, "current", "/tmp"),
        ];
        sort_sessions(&mut sessions, "current");
        assert_eq!(sessions[0].pane.workspace, "current");
        assert_eq!(sessions[1].pane.workspace, "other");
    }

    #[test]
    fn test_sort_by_workspace_then_cwd_then_pane_id() {
        let mut sessions = vec![
            make_session(3, "alpha", "/home/b"),
            make_session(1, "alpha", "/home/a"),
            make_session(2, "alpha", "/home/a"),
        ];
        sort_sessions(&mut sessions, "none");
        assert_eq!(sessions[0].pane.pane_id, 1);
        assert_eq!(sessions[1].pane.pane_id, 2);
        assert_eq!(sessions[2].pane.pane_id, 3);
    }

    #[test]
    fn test_sort_multiple_workspaces() {
        let mut sessions = vec![
            make_session(1, "beta", "/tmp"),
            make_session(2, "alpha", "/tmp"),
            make_session(3, "current", "/tmp"),
        ];
        sort_sessions(&mut sessions, "current");
        assert_eq!(sessions[0].pane.workspace, "current"); // current first
        assert_eq!(sessions[1].pane.workspace, "alpha"); // then alphabetical
        assert_eq!(sessions[2].pane.workspace, "beta");
    }

    #[test]
    fn test_sort_stable_for_same_workspace_and_cwd() {
        let mut sessions = vec![
            make_session(5, "ws", "/home/project"),
            make_session(2, "ws", "/home/project"),
            make_session(8, "ws", "/home/project"),
        ];
        sort_sessions(&mut sessions, "ws");
        assert_eq!(sessions[0].pane.pane_id, 2);
        assert_eq!(sessions[1].pane.pane_id, 5);
        assert_eq!(sessions[2].pane.pane_id, 8);
    }
}
