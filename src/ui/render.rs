use crate::transcript::ConversationTurn;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::Line,
    widgets::ListState,
};
use std::time::SystemTime;

use super::session::ClaudeSession;

#[path = "render/command_select.rs"]
mod command_select;
#[path = "render/footer.rs"]
mod footer;
#[path = "render/history.rs"]
mod history;
#[path = "render/live.rs"]
mod live;
#[path = "render/summary.rs"]
mod summary;

/// Cache entry for history detail view: ((text_hash, width), rendered_lines).
pub type HistoryLinesCache = Option<((u64, usize), Vec<Line<'static>>)>;
/// Cache entry for details preview: ((text_hash, width, max_lines), rendered_lines).
pub type PreviewLinesCache = Option<((u64, usize, usize), Vec<Line<'static>>)>;
/// Cache entry for live pane view: ((content_hash, width), rendered_lines).
pub type LivePaneLinesCache = Option<((u64, usize), Vec<Line<'static>>)>;

/// Detail panel display mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailMode {
    /// Normal detail view (session info + last prompt/output preview).
    Summary,
    /// Showing the history turn list.
    HistoryList,
    /// Showing a single turn's detail.
    HistoryDetail,
    /// Live pane view: raw terminal output from `wezterm cli get-text`.
    LivePane,
}

/// Rendering context for the details panel.
pub struct DetailsRenderCtx<'a> {
    pub sessions: &'a [ClaudeSession],
    pub selected: Option<usize>,
    pub input_mode: bool,
    pub input_buffer: &'a str,
    pub cursor_position: usize,
    pub detail_mode: DetailMode,
    pub history_turns: &'a [ConversationTurn],
    pub history_index: usize,
    pub history_scroll_offset: &'a mut usize,
    pub history_list_state: &'a mut ListState,
    pub history_timestamps: &'a [Option<SystemTime>],
    pub cached_history_lines: &'a mut HistoryLinesCache,
    pub cached_preview_lines: &'a mut PreviewLinesCache,
    pub live_pane_bytes: Option<&'a [u8]>,
    pub live_pane_scroll_offset: &'a mut usize,
    pub cached_live_pane_lines: &'a mut LivePaneLinesCache,
    pub live_pane_error: bool,
}

/// Render the session list.
pub fn render_list(
    f: &mut ratatui::Frame,
    area: Rect,
    sessions: &[ClaudeSession],
    list_state: &mut ListState,
    refreshing: bool,
    animation_frame: u8,
    current_workspace: &str,
) -> Option<Rect> {
    summary::render_list(
        f,
        area,
        sessions,
        list_state,
        refreshing,
        animation_frame,
        current_workspace,
    )
}

/// Render the details panel.
pub fn render_details(f: &mut ratatui::Frame, area: Rect, ctx: &mut DetailsRenderCtx<'_>) {
    if ctx.detail_mode == DetailMode::LivePane {
        let content_area = if let Some(session) = ctx.selected.and_then(|i| ctx.sessions.get(i)) {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(2), Constraint::Min(0)])
                .split(area);
            summary::render_session_info_header(f, chunks[0], session);
            chunks[1]
        } else {
            area
        };

        live::render_live_pane(
            f,
            content_area,
            ctx.live_pane_bytes,
            ctx.live_pane_scroll_offset,
            ctx.cached_live_pane_lines,
            ctx.live_pane_error,
        );
        return;
    }

    if matches!(
        ctx.detail_mode,
        DetailMode::HistoryList | DetailMode::HistoryDetail
    ) && !ctx.history_turns.is_empty()
    {
        let content_area = if let Some(session) = ctx.selected.and_then(|i| ctx.sessions.get(i)) {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(2), Constraint::Min(0)])
                .split(area);
            summary::render_session_info_header(f, chunks[0], session);
            chunks[1]
        } else {
            area
        };

        match ctx.detail_mode {
            DetailMode::HistoryList => {
                history::render_history_list(
                    f,
                    content_area,
                    ctx.history_turns,
                    ctx.history_list_state,
                    ctx.history_timestamps,
                );
            }
            DetailMode::HistoryDetail => {
                history::render_history_details(
                    f,
                    content_area,
                    ctx.history_turns,
                    ctx.history_index,
                    ctx.history_scroll_offset,
                    ctx.cached_history_lines,
                );
            }
            _ => unreachable!(),
        }
        return;
    }

    summary::render_summary_details(f, area, ctx);
}

/// Render the footer with keybindings help.
#[allow(clippy::too_many_arguments)]
pub fn render_footer(
    f: &mut ratatui::Frame,
    area: Rect,
    input_mode: bool,
    detail_mode: DetailMode,
    toast: Option<&super::toast::Toast>,
    kill_confirm: Option<&(u32, String)>,
    add_pane_pending: Option<&(u32, String, u32)>,
    command_select_active: bool,
) {
    footer::render_footer(
        f,
        area,
        input_mode,
        detail_mode,
        toast,
        kill_confirm,
        add_pane_pending,
        command_select_active,
    );
}

/// Render the command selection popup overlay.
pub fn render_command_select(
    f: &mut ratatui::Frame,
    area: Rect,
    commands: &[crate::config::SpawnCommand],
    list_state: &mut ListState,
) {
    command_select::render_command_select(f, area, commands, list_state);
}
