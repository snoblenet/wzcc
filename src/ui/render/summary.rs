use crate::transcript::SessionStatus;
use crate::transcript::WaitingPrompt;
use crate::ui::markdown;
use crate::ui::session::{status_display, ClaudeSession};
use ratatui::{
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use std::hash::{Hash, Hasher};
use std::time::SystemTime;
use unicode_width::UnicodeWidthChar;

use super::DetailsRenderCtx;

/// Animation frames for Processing status (rotating dots)
const PROCESSING_FRAMES: [&str; 4] = ["◐", "◓", "◑", "◒"];

/// Compute a fast hash of a string for cache key comparison.
fn hash_str(s: &str) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Format a duration as a relative time string (e.g., "5s", "2m", "1h", "3d").
fn format_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

/// Format relative time (e.g., "5s", "2m", "1h")
pub(super) fn format_relative_time(time: &SystemTime) -> String {
    let now = SystemTime::now();
    match now.duration_since(*time) {
        Ok(d) => format_duration(d),
        Err(_) => "now".to_string(),
    }
}

/// Get color for a given elapsed duration.
/// - < 5 minutes: Green (fresh/active)
/// - 5-30 minutes: Yellow (slightly stale)
/// - > 30 minutes: Red (inactive/stale)
fn color_for_elapsed(duration: std::time::Duration) -> Color {
    let secs = duration.as_secs();
    if secs < 300 {
        Color::Green
    } else if secs < 1800 {
        Color::Yellow
    } else {
        Color::Red
    }
}

/// Get color for elapsed time display based on a SystemTime.
pub(super) fn elapsed_time_color(time: &SystemTime) -> Color {
    let now = SystemTime::now();
    match now.duration_since(*time) {
        Ok(d) => color_for_elapsed(d),
        Err(_) => Color::Green,
    }
}

/// Render the session list.
pub(super) fn render_list(
    f: &mut ratatui::Frame,
    area: Rect,
    sessions: &[ClaudeSession],
    list_state: &mut ListState,
    refreshing: bool,
    animation_frame: u8,
    current_workspace: &str,
) -> Option<Rect> {
    // Count sessions per (workspace, cwd)
    let mut cwd_info: std::collections::HashMap<(String, String), usize> =
        std::collections::HashMap::new();
    for session in sessions {
        let ws = session.pane.workspace.clone();
        if let Some(cwd) = session.pane.cwd_path() {
            *cwd_info.entry((ws, cwd)).or_insert(0) += 1;
        }
    }

    // Build list items (workspace header + cwd header + sessions)
    let mut items: Vec<ListItem> = Vec::new();
    let mut session_indices: Vec<usize> = Vec::new(); // ListItem index -> session index mapping
    let mut current_ws: Option<String> = None;
    let mut current_cwd: Option<String> = None;

    for (session_idx, session) in sessions.iter().enumerate() {
        let pane = &session.pane;
        let ws = &pane.workspace;
        let cwd = pane.cwd_path().unwrap_or_default();

        // Add workspace header for new workspace
        if current_ws.as_ref() != Some(ws) {
            current_ws = Some(ws.clone());
            current_cwd = None; // Reset cwd tracking for new workspace

            // Visual distinction for current vs other workspace (subtle colors)
            let (ws_icon, ws_style) = if ws == current_workspace {
                (
                    "🏠",
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                ("📍", Style::default().fg(Color::DarkGray))
            };

            let ws_header = Line::from(vec![Span::styled(
                format!("{} Workspace: {}", ws_icon, ws),
                ws_style,
            )]);
            items.push(ListItem::new(ws_header));
            session_indices.push(usize::MAX); // Header is not a session
        }

        // Get group info
        let count = cwd_info
            .get(&(ws.clone(), cwd.clone()))
            .copied()
            .unwrap_or(1);

        // Add header for new CWD (within the same workspace)
        if current_cwd.as_ref() != Some(&cwd) {
            current_cwd = Some(cwd.clone());

            // Get directory name from cwd
            let dir_name = std::path::Path::new(&cwd)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&cwd)
                .to_string();

            // Show session count if multiple sessions
            let header_text = if count > 1 {
                format!("  📂 {} ({} sessions)", dir_name, count)
            } else {
                format!("  📂 {}", dir_name)
            };

            let header_line = Line::from(vec![Span::raw(header_text)]);
            items.push(ListItem::new(header_line));
            session_indices.push(usize::MAX); // Header is not a session
        }

        // Status icon and color (Processing uses animated spinner)
        let (status_icon, status_color) = match &session.status {
            SessionStatus::Ready => ("◇", Color::Cyan),
            SessionStatus::Processing => (
                PROCESSING_FRAMES[animation_frame as usize % 4],
                Color::Yellow,
            ),
            SessionStatus::Idle => ("○", Color::Green),
            SessionStatus::WaitingForUser { .. } => ("◐", Color::Magenta),
            SessionStatus::Unknown => ("?", Color::DarkGray),
        };

        // Title (max 35 chars)
        let title = if pane.title.chars().count() > 35 {
            let truncated: String = pane.title.chars().take(32).collect();
            format!("{}...", truncated)
        } else {
            pane.title.clone()
        };

        // Quick select number (1-9, or space if > 9)
        let quick_num = if session_idx < 9 {
            format!("[{}]", session_idx + 1)
        } else {
            "   ".to_string()
        };

        // Relative time display with color based on elapsed time
        let (time_display, time_color) = session
            .updated_at
            .as_ref()
            .map(|t| {
                (
                    format!(" {}", format_relative_time(t)),
                    elapsed_time_color(t),
                )
            })
            .unwrap_or((String::new(), Color::DarkGray));

        // Indent (all sessions are indented under workspace + cwd headers)
        let line = Line::from(vec![
            Span::raw("    "), // Extra indent for hierarchy
            Span::styled(format!("{} ", quick_num), Style::default().fg(Color::White)),
            Span::styled(
                format!("{} ", status_icon),
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(title),
            Span::styled(
                format!(" [{}]", session.status.as_str()),
                Style::default().fg(status_color),
            ),
            Span::styled(time_display, Style::default().fg(time_color)),
        ]);

        items.push(ListItem::new(line));
        session_indices.push(session_idx);
    }

    // Convert list_state index to ListItem index
    let list_index = list_state
        .selected()
        .and_then(|session_idx| session_indices.iter().position(|&idx| idx == session_idx));

    let mut render_state = ListState::default();
    render_state.select(list_index);

    // Title (show indicator while refreshing)
    let title = if refreshing {
        " ⌛ Claude Code Sessions - Refreshing... ".to_string()
    } else {
        format!(" Claude Code Sessions ({}) ", sessions.len())
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    f.render_stateful_widget(list, area, &mut render_state);

    Some(area)
}

pub(super) fn render_summary_details(
    f: &mut ratatui::Frame,
    area: Rect,
    ctx: &mut DetailsRenderCtx<'_>,
) {
    let text = if let Some(i) = ctx.selected {
        if let Some(session) = ctx.sessions.get(i) {
            let pane = &session.pane;

            // Quick select number display (1-9 or none)
            let quick_num_display = if i < 9 {
                format!(" [{}]", i + 1)
            } else {
                String::new()
            };

            // -- Compact badge-style metadata header --

            // Line 1: Pane ID [quick] │ ● Status │ ⎇ Branch
            let (status_color, status_text) = status_display(&session.status);
            let mut header_spans: Vec<Span<'_>> = vec![
                Span::styled(
                    format!("#{}", pane.pane_id),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(quick_num_display, Style::default().fg(Color::DarkGray)),
                Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                Span::styled("● ", Style::default().fg(status_color)),
                Span::styled(status_text, Style::default().fg(status_color)),
            ];
            if let Some(branch) = &session.git_branch {
                header_spans.push(Span::styled("  │ ", Style::default().fg(Color::DarkGray)));
                header_spans.push(Span::styled("⎇ ", Style::default().fg(Color::Cyan)));
                header_spans.push(Span::styled(
                    branch.as_str(),
                    Style::default().fg(Color::Cyan),
                ));
            }
            if let Some(worktree) = &session.git_worktree {
                header_spans.push(Span::styled("  │ ", Style::default().fg(Color::DarkGray)));
                header_spans.push(Span::styled("🌳 ", Style::default().fg(Color::Green)));
                header_spans.push(Span::styled(
                    worktree.as_str(),
                    Style::default().fg(Color::Green),
                ));
            }
            let mut lines = vec![Line::from(header_spans)];

            // Line 2: Workspace │ TTY
            let mut info_spans: Vec<Span<'_>> = vec![Span::styled(
                &pane.workspace,
                Style::default().fg(Color::Yellow),
            )];
            if let Some(tty) = &pane.tty_name {
                info_spans.push(Span::styled("  │ ", Style::default().fg(Color::DarkGray)));
                info_spans.push(Span::styled(tty, Style::default().fg(Color::DarkGray)));
            }
            lines.push(Line::from(info_spans));

            // Line 3: CWD (if present)
            if let Some(cwd) = pane.cwd_path() {
                lines.push(Line::from(Span::styled(
                    cwd,
                    Style::default().fg(Color::DarkGray),
                )));
            }

            // Show hint when session is waiting for user input
            if session.waiting_prompt.is_some() {
                lines.push(Line::from(vec![
                    Span::styled("⚡ ", Style::default().fg(Color::Magenta)),
                    Span::styled("Press ", Style::default().fg(Color::Magenta)),
                    Span::styled(
                        "o",
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(" to respond", Style::default().fg(Color::Magenta)),
                ]));
            }

            // Display last prompt and last output preview
            // Dynamically compute header height: actual lines + 2 for block border
            let header_lines = lines.len() as u16 + 2;
            let available_for_preview = area.height.saturating_sub(header_lines) as usize;
            let inner_width = (area.width.saturating_sub(2)) as usize;

            if available_for_preview >= 1 {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![Span::styled(
                    "─".repeat(inner_width),
                    Style::default().fg(Color::DarkGray),
                )]));

                // When PlanApproval with plan content, show plan instead of last prompt/output
                let plan_text = match &session.waiting_prompt {
                    Some(WaitingPrompt::PlanApproval { plan }) if !plan.is_empty() => {
                        Some(plan.as_str())
                    }
                    _ => None,
                };

                if let Some(plan) = plan_text {
                    lines.push(Line::from(vec![Span::styled(
                        "📋 Claude's plan:",
                        Style::default().add_modifier(Modifier::BOLD),
                    )]));

                    let text_hash = hash_str(plan);
                    let cache_key = (text_hash, inner_width);
                    let output_lines =
                        if let Some((cached_key, cached)) = ctx.cached_preview_lines.as_ref() {
                            if *cached_key == cache_key {
                                cached.clone()
                            } else {
                                let rendered = markdown::markdown_to_lines(plan, inner_width);
                                *ctx.cached_preview_lines = Some((cache_key, rendered.clone()));
                                rendered
                            }
                        } else {
                            let rendered = markdown::markdown_to_lines(plan, inner_width);
                            *ctx.cached_preview_lines = Some((cache_key, rendered.clone()));
                            rendered
                        };
                    lines.extend(output_lines);
                } else {
                    if let Some(prompt) = &session.last_prompt {
                        lines.push(Line::from(vec![Span::styled(
                            "💬 Last prompt:",
                            Style::default().add_modifier(Modifier::BOLD),
                        )]));
                        let prompt_lines = crate::ui::session::wrap_text_lines(
                            prompt,
                            inner_width,
                            usize::MAX,
                            Color::Cyan,
                        );
                        lines.extend(prompt_lines);
                    }

                    if let Some(output) = &session.last_output {
                        if session.last_prompt.is_some() {
                            lines.push(Line::from(""));
                            lines.push(Line::from(vec![Span::styled(
                                "─".repeat(inner_width),
                                Style::default().fg(Color::DarkGray),
                            )]));
                        }

                        lines.push(Line::from(vec![Span::styled(
                            "🤖 Last output:",
                            Style::default().add_modifier(Modifier::BOLD),
                        )]));

                        let text_hash = hash_str(output);
                        let cache_key = (text_hash, inner_width);
                        let output_lines =
                            if let Some((cached_key, cached)) = ctx.cached_preview_lines.as_ref() {
                                if *cached_key == cache_key {
                                    cached.clone()
                                } else {
                                    let rendered = markdown::markdown_to_lines(output, inner_width);
                                    *ctx.cached_preview_lines = Some((cache_key, rendered.clone()));
                                    rendered
                                }
                            } else {
                                let rendered = markdown::markdown_to_lines(output, inner_width);
                                *ctx.cached_preview_lines = Some((cache_key, rendered.clone()));
                                rendered
                            };
                        lines.extend(output_lines);
                    }
                }
            }

            lines
        } else {
            vec![Line::from("No selection")]
        }
    } else {
        vec![Line::from("No sessions")]
    };

    if ctx.input_mode {
        let inner_width = area.width.saturating_sub(2) as usize;
        let prefix_width: usize = 2;
        let text_width = inner_width.saturating_sub(prefix_width);

        let mut visual_lines: Vec<Line<'static>> = Vec::new();
        let mut cursor_visual_row: u16 = 0;
        let mut cursor_visual_col: u16 = prefix_width as u16;
        let mut global_byte = 0usize;

        let logical_lines: Vec<&str> = ctx.input_buffer.split('\n').collect();
        for (li, logical_line) in logical_lines.iter().enumerate() {
            let prefix_str = if li == 0 { "> " } else { "  " };

            if logical_line.is_empty() {
                if ctx.cursor_position == global_byte {
                    cursor_visual_row = visual_lines.len() as u16;
                    cursor_visual_col = prefix_width as u16;
                }
                visual_lines.push(Line::from(Span::styled(
                    prefix_str.to_string(),
                    Style::default().fg(Color::Cyan),
                )));
            } else if text_width == 0 {
                visual_lines.push(Line::from(Span::styled(
                    prefix_str.to_string(),
                    Style::default().fg(Color::Cyan),
                )));
            } else {
                let mut col_w = 0usize;
                let mut chunk_start = 0usize;
                let mut is_first_visual = true;

                for (ci, ch) in logical_line.char_indices() {
                    let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);

                    if col_w + ch_w > text_width && col_w > 0 {
                        let chunk = &logical_line[chunk_start..ci];
                        let pfx = if is_first_visual { prefix_str } else { "  " };
                        visual_lines.push(Line::from(vec![
                            Span::styled(pfx.to_string(), Style::default().fg(Color::Cyan)),
                            Span::raw(chunk.to_string()),
                        ]));
                        chunk_start = ci;
                        col_w = 0;
                        is_first_visual = false;
                    }

                    let byte_in_buf = global_byte + ci;
                    if byte_in_buf == ctx.cursor_position {
                        cursor_visual_row = visual_lines.len() as u16;
                        cursor_visual_col = (prefix_width + col_w) as u16;
                    }

                    col_w += ch_w;
                }

                let chunk = &logical_line[chunk_start..];
                let pfx = if is_first_visual { prefix_str } else { "  " };
                visual_lines.push(Line::from(vec![
                    Span::styled(pfx.to_string(), Style::default().fg(Color::Cyan)),
                    Span::raw(chunk.to_string()),
                ]));

                let end_byte = global_byte + logical_line.len();
                if ctx.cursor_position == end_byte {
                    cursor_visual_row = (visual_lines.len() - 1) as u16;
                    cursor_visual_col = (prefix_width + col_w) as u16;
                }
            }

            global_byte += logical_line.len() + 1;
        }

        let input_height = (visual_lines.len() as u16 + 2).min(7);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(input_height)])
            .split(area);

        let content_height = text.len();
        let viewport_height = chunks[0].height.saturating_sub(2) as usize;
        let max_scroll = content_height.saturating_sub(viewport_height);
        *ctx.summary_scroll_offset = (*ctx.summary_scroll_offset).min(max_scroll);
        let clamped_offset = (*ctx.summary_scroll_offset).min(u16::MAX as usize) as u16;

        let paragraph = Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(" Details "))
            .wrap(Wrap { trim: false })
            .scroll((clamped_offset, 0));
        f.render_widget(paragraph, chunks[0]);

        let pane_id = ctx
            .selected
            .and_then(|i| ctx.sessions.get(i))
            .map(|s| s.pane.pane_id)
            .unwrap_or(0);

        let max_visible_lines = input_height.saturating_sub(2);
        let scroll_offset = if cursor_visual_row >= max_visible_lines {
            cursor_visual_row - max_visible_lines + 1
        } else {
            0
        };

        let input_paragraph = Paragraph::new(visual_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Send prompt to Pane {} ", pane_id))
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .scroll((scroll_offset, 0));
        f.render_widget(input_paragraph, chunks[1]);

        let cursor_x = chunks[1].x + 1 + cursor_visual_col;
        let cursor_y = chunks[1].y + 1 + cursor_visual_row - scroll_offset;
        f.set_cursor_position(Position::new(cursor_x, cursor_y));
    } else {
        let content_height = text.len();
        let viewport_height = area.height.saturating_sub(2) as usize;
        let max_scroll = content_height.saturating_sub(viewport_height);
        *ctx.summary_scroll_offset = (*ctx.summary_scroll_offset).min(max_scroll);
        let clamped_offset = (*ctx.summary_scroll_offset).min(u16::MAX as usize) as u16;

        let paragraph = Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(" Details "))
            .wrap(Wrap { trim: false })
            .scroll((clamped_offset, 0));

        f.render_widget(paragraph, area);
    }
}

/// Render compact session info header above history content.
pub(super) fn render_session_info_header(
    f: &mut ratatui::Frame,
    area: Rect,
    session: &ClaudeSession,
) {
    let pane = &session.pane;
    let (status_color, status_text) = status_display(&session.status);

    let mut spans: Vec<Span<'_>> = vec![
        Span::raw(" "),
        Span::styled(&pane.workspace, Style::default().fg(Color::Yellow)),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(status_text, Style::default().fg(status_color)),
    ];

    if let Some(branch) = &session.git_branch {
        spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            branch.as_str(),
            Style::default().fg(Color::Cyan),
        ));
    }
    if let Some(worktree) = &session.git_worktree {
        spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled("🌳 ", Style::default().fg(Color::Green)));
        spans.push(Span::styled(
            worktree.as_str(),
            Style::default().fg(Color::Green),
        ));
    }

    let mut lines = vec![Line::from(spans)];

    if let Some(cwd) = pane.cwd_path() {
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(cwd, Style::default().fg(Color::DarkGray)),
        ]));
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_format_duration_seconds() {
        assert_eq!(format_duration(Duration::from_secs(0)), "0s");
        assert_eq!(format_duration(Duration::from_secs(1)), "1s");
        assert_eq!(format_duration(Duration::from_secs(30)), "30s");
        assert_eq!(format_duration(Duration::from_secs(59)), "59s");
    }

    #[test]
    fn test_format_duration_minutes() {
        assert_eq!(format_duration(Duration::from_secs(60)), "1m");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m");
        assert_eq!(format_duration(Duration::from_secs(300)), "5m");
        assert_eq!(format_duration(Duration::from_secs(3599)), "59m");
    }

    #[test]
    fn test_format_duration_hours() {
        assert_eq!(format_duration(Duration::from_secs(3600)), "1h");
        assert_eq!(format_duration(Duration::from_secs(7200)), "2h");
        assert_eq!(format_duration(Duration::from_secs(86399)), "23h");
    }

    #[test]
    fn test_format_duration_days() {
        assert_eq!(format_duration(Duration::from_secs(86400)), "1d");
        assert_eq!(format_duration(Duration::from_secs(172800)), "2d");
    }

    #[test]
    fn test_format_relative_time_recent() {
        let time = SystemTime::now() - Duration::from_secs(5);
        let result = format_relative_time(&time);
        assert!(result.ends_with('s'));
    }

    #[test]
    fn test_format_relative_time_future() {
        let time = SystemTime::now() + Duration::from_secs(100);
        assert_eq!(format_relative_time(&time), "now");
    }

    #[test]
    fn test_color_for_elapsed_green() {
        assert_eq!(color_for_elapsed(Duration::from_secs(0)), Color::Green);
        assert_eq!(color_for_elapsed(Duration::from_secs(60)), Color::Green);
        assert_eq!(color_for_elapsed(Duration::from_secs(299)), Color::Green);
    }

    #[test]
    fn test_color_for_elapsed_yellow() {
        assert_eq!(color_for_elapsed(Duration::from_secs(300)), Color::Yellow);
        assert_eq!(color_for_elapsed(Duration::from_secs(900)), Color::Yellow);
        assert_eq!(color_for_elapsed(Duration::from_secs(1799)), Color::Yellow);
    }

    #[test]
    fn test_color_for_elapsed_red() {
        assert_eq!(color_for_elapsed(Duration::from_secs(1800)), Color::Red);
        assert_eq!(color_for_elapsed(Duration::from_secs(3600)), Color::Red);
        assert_eq!(color_for_elapsed(Duration::from_secs(86400)), Color::Red);
    }

    #[test]
    fn test_elapsed_time_color_recent() {
        let time = SystemTime::now() - Duration::from_secs(10);
        assert_eq!(elapsed_time_color(&time), Color::Green);
    }

    #[test]
    fn test_elapsed_time_color_stale() {
        let time = SystemTime::now() - Duration::from_secs(600);
        assert_eq!(elapsed_time_color(&time), Color::Yellow);
    }

    #[test]
    fn test_elapsed_time_color_very_stale() {
        let time = SystemTime::now() - Duration::from_secs(3600);
        assert_eq!(elapsed_time_color(&time), Color::Red);
    }

    #[test]
    fn test_elapsed_time_color_future() {
        let time = SystemTime::now() + Duration::from_secs(100);
        assert_eq!(elapsed_time_color(&time), Color::Green);
    }
}
