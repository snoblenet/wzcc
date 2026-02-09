use crate::transcript::SessionStatus;
use ratatui::{
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use std::hash::{Hash, Hasher};
use std::time::SystemTime;
use unicode_width::UnicodeWidthChar;

use crate::transcript::ConversationTurn;

use super::markdown;
use super::session::{status_display, wrap_text_lines, ClaudeSession};

/// Cache entry for history detail view: ((text_hash, width), rendered_lines).
type HistoryLinesCache = Option<((u64, usize), Vec<Line<'static>>)>;
/// Cache entry for details preview: ((text_hash, width, max_lines), rendered_lines).
type PreviewLinesCache = Option<((u64, usize, usize), Vec<Line<'static>>)>;

/// Compute a fast hash of a string for cache key comparison.
fn hash_str(s: &str) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Sub-mode for history browsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryViewMode {
    /// History mode is not active.
    Off,
    /// Showing the history turn list.
    List,
    /// Showing a single turn's detail.
    Detail,
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
fn format_relative_time(time: &SystemTime) -> String {
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
fn elapsed_time_color(time: &SystemTime) -> Color {
    let now = SystemTime::now();
    match now.duration_since(*time) {
        Ok(d) => color_for_elapsed(d),
        Err(_) => Color::Green,
    }
}

/// Animation frames for Processing status (rotating dots)
const PROCESSING_FRAMES: [&str; 4] = ["◐", "◓", "◑", "◒"];

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
            Span::styled(
                format!("Pane {}: ", pane.pane_id),
                Style::default().fg(Color::White),
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

/// Render the details panel.
#[allow(clippy::too_many_arguments)]
pub fn render_details(
    f: &mut ratatui::Frame,
    area: Rect,
    sessions: &[ClaudeSession],
    selected: Option<usize>,
    input_mode: bool,
    input_buffer: &str,
    cursor_position: usize,
    history_view: HistoryViewMode,
    history_turns: &[ConversationTurn],
    history_index: usize,
    history_scroll_offset: &mut usize,
    history_list_state: &mut ListState,
    history_timestamps: &[Option<SystemTime>],
    cached_history_lines: &mut HistoryLinesCache,
    cached_preview_lines: &mut PreviewLinesCache,
) {
    // History browsing mode dispatch
    if matches!(
        history_view,
        HistoryViewMode::List | HistoryViewMode::Detail
    ) && !history_turns.is_empty()
    {
        // Split area: compact session info header + history content
        let content_area = if let Some(session) = selected.and_then(|i| sessions.get(i)) {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(2), Constraint::Min(0)])
                .split(area);
            render_session_info_header(f, chunks[0], session);
            chunks[1]
        } else {
            area
        };

        match history_view {
            HistoryViewMode::List => {
                render_history_list(
                    f,
                    content_area,
                    history_turns,
                    history_list_state,
                    history_timestamps,
                );
            }
            HistoryViewMode::Detail => {
                render_history_details(
                    f,
                    content_area,
                    history_turns,
                    history_index,
                    history_scroll_offset,
                    cached_history_lines,
                );
            }
            _ => unreachable!(),
        }
        return;
    }

    let text = if let Some(i) = selected {
        if let Some(session) = sessions.get(i) {
            let pane = &session.pane;

            // Quick select number display (1-9 or none)
            let quick_num_display = if i < 9 {
                format!(" [{}]", i + 1)
            } else {
                String::new()
            };

            let mut lines = vec![Line::from(vec![
                Span::styled("Pane: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(pane.pane_id.to_string()),
                Span::styled(quick_num_display, Style::default().fg(Color::DarkGray)),
            ])];

            // Display workspace
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("Workspace: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(&pane.workspace, Style::default().fg(Color::Yellow)),
            ]));

            if let Some(cwd) = pane.cwd_path() {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![Span::styled(
                    "CWD:",
                    Style::default().add_modifier(Modifier::BOLD),
                )]));
                lines.push(Line::from(cwd));
            }

            if let Some(tty) = &pane.tty_name {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled("TTY: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::raw(tty),
                ]));
            }

            // Display session status
            lines.push(Line::from(""));
            let (status_color, status_text) = status_display(&session.status);
            lines.push(Line::from(vec![
                Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(status_text, Style::default().fg(status_color)),
            ]));

            // Display warning message if present
            if let Some(warning) = &session.warning {
                lines.push(Line::from(vec![Span::styled(
                    format!("⚠️  {}", warning),
                    Style::default().fg(Color::Red),
                )]));
            }

            // Display git branch
            if let Some(branch) = &session.git_branch {
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled("Branch: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(branch, Style::default().fg(Color::Cyan)),
                ]));
            }

            // Display last prompt and last output preview
            // Fixed lines: Pane(2) + Workspace(2) + CWD(3) + TTY(2) + Status(2) + Branch(2) + border(2) = ~15 lines
            let fixed_lines: u16 = 15;
            let available_for_preview = area.height.saturating_sub(fixed_lines) as usize;
            let inner_width = (area.width.saturating_sub(2)) as usize;

            // Display if at least 1 line available (previously 3 lines was too strict)
            if available_for_preview >= 1 {
                // Separator line
                lines.push(Line::from(""));
                lines.push(Line::from(vec![Span::styled(
                    "─".repeat(inner_width),
                    Style::default().fg(Color::DarkGray),
                )]));

                // Display last prompt (1-2 lines)
                if let Some(prompt) = &session.last_prompt {
                    lines.push(Line::from(vec![Span::styled(
                        "💬 Last prompt:",
                        Style::default().add_modifier(Modifier::BOLD),
                    )]));
                    // Truncate prompt to 1-2 lines
                    let prompt_chars: Vec<char> = prompt.chars().collect();
                    let max_prompt_len = inner_width * 2;
                    let truncated: String = if prompt_chars.len() > max_prompt_len {
                        prompt_chars[..max_prompt_len].iter().collect::<String>() + "..."
                    } else {
                        prompt_chars.iter().collect()
                    };
                    for line in truncated.lines().take(2) {
                        lines.push(Line::from(Span::styled(
                            line.to_string(),
                            Style::default().fg(Color::Cyan),
                        )));
                    }
                }

                // Display last output
                if let Some(output) = &session.last_output {
                    // Separator between prompt and output
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

                    // Separator + prompt + output label uses ~8 lines
                    let preview_lines = available_for_preview.saturating_sub(8);
                    let text_hash = hash_str(output);
                    let cache_key = (text_hash, inner_width, preview_lines);
                    let output_lines =
                        if let Some((cached_key, cached)) = cached_preview_lines.as_ref() {
                            if *cached_key == cache_key {
                                cached.clone()
                            } else {
                                let rendered = markdown::markdown_to_lines_truncated(
                                    output,
                                    inner_width,
                                    preview_lines,
                                );
                                *cached_preview_lines = Some((cache_key, rendered.clone()));
                                rendered
                            }
                        } else {
                            let rendered = markdown::markdown_to_lines_truncated(
                                output,
                                inner_width,
                                preview_lines,
                            );
                            *cached_preview_lines = Some((cache_key, rendered.clone()));
                            rendered
                        };
                    lines.extend(output_lines);
                }
            }

            lines
        } else {
            vec![Line::from("No selection")]
        }
    } else {
        vec![Line::from("No sessions")]
    };

    if input_mode {
        // Inner width of the input box (inside borders)
        let inner_width = area.width.saturating_sub(2) as usize;
        let prefix_width: usize = 2; // "> " or "  "
        let text_width = inner_width.saturating_sub(prefix_width);

        // Build visual lines with manual wrapping + track cursor position
        let mut visual_lines: Vec<Line<'static>> = Vec::new();
        let mut cursor_visual_row: u16 = 0;
        let mut cursor_visual_col: u16 = prefix_width as u16;
        let mut global_byte = 0usize;

        let logical_lines: Vec<&str> = input_buffer.split('\n').collect();
        for (li, logical_line) in logical_lines.iter().enumerate() {
            let prefix_str = if li == 0 { "> " } else { "  " };

            if logical_line.is_empty() {
                // Cursor on empty line
                if cursor_position == global_byte {
                    cursor_visual_row = visual_lines.len() as u16;
                    cursor_visual_col = prefix_width as u16;
                }
                visual_lines.push(Line::from(Span::styled(
                    prefix_str.to_string(),
                    Style::default().fg(Color::Cyan),
                )));
            } else if text_width == 0 {
                // Degenerate: no space for text
                visual_lines.push(Line::from(Span::styled(
                    prefix_str.to_string(),
                    Style::default().fg(Color::Cyan),
                )));
            } else {
                let mut col_w = 0usize;
                let mut chunk_start = 0usize; // byte offset within logical_line
                let mut is_first_visual = true;

                for (ci, ch) in logical_line.char_indices() {
                    let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);

                    // Wrap if adding this char would exceed available width
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

                    // Check if cursor is at this character
                    let byte_in_buf = global_byte + ci;
                    if byte_in_buf == cursor_position {
                        cursor_visual_row = visual_lines.len() as u16;
                        cursor_visual_col = (prefix_width + col_w) as u16;
                    }

                    col_w += ch_w;
                }

                // Emit last chunk
                let chunk = &logical_line[chunk_start..];
                let pfx = if is_first_visual { prefix_str } else { "  " };
                visual_lines.push(Line::from(vec![
                    Span::styled(pfx.to_string(), Style::default().fg(Color::Cyan)),
                    Span::raw(chunk.to_string()),
                ]));

                // Cursor at end of this logical line
                let end_byte = global_byte + logical_line.len();
                if cursor_position == end_byte {
                    cursor_visual_row = (visual_lines.len() - 1) as u16;
                    cursor_visual_col = (prefix_width + col_w) as u16;
                }
            }

            global_byte += logical_line.len() + 1; // +1 for '\n'
        }

        // Input field height: visual lines + 2 (borders), max 7
        let input_height = (visual_lines.len() as u16 + 2).min(7);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(input_height)])
            .split(area);

        // Render details in top area
        let paragraph = Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(" Details "))
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, chunks[0]);

        // Render input field in bottom area
        let pane_id = selected
            .and_then(|i| sessions.get(i))
            .map(|s| s.pane.pane_id)
            .unwrap_or(0);

        // Calculate scroll offset to keep cursor visible
        let max_visible_lines = input_height.saturating_sub(2); // inside borders
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

        // Set cursor position (+1 for border, adjusted for scroll)
        let cursor_x = chunks[1].x + 1 + cursor_visual_col;
        let cursor_y = chunks[1].y + 1 + cursor_visual_row - scroll_offset;
        f.set_cursor_position(Position::new(cursor_x, cursor_y));
    } else {
        let paragraph = Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(" Details "))
            .wrap(Wrap { trim: false });

        f.render_widget(paragraph, area);
    }
}

/// Render compact session info header above history content.
fn render_session_info_header(f: &mut ratatui::Frame, area: Rect, session: &ClaudeSession) {
    let pane = &session.pane;
    let (status_color, status_text) = status_display(&session.status);

    // Line 1: Workspace + Status + Branch
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

    let mut lines = vec![Line::from(spans)];

    // Line 2: CWD
    if let Some(cwd) = pane.cwd_path() {
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(cwd, Style::default().fg(Color::DarkGray)),
        ]));
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

/// Render the history turn list in the details panel area.
fn render_history_list(
    f: &mut ratatui::Frame,
    area: Rect,
    turns: &[ConversationTurn],
    list_state: &mut ListState,
    timestamps: &[Option<SystemTime>],
) {
    let inner_width = area.width.saturating_sub(5) as usize; // borders(2) + highlight symbol ">> "(3)

    let items: Vec<ListItem> = turns
        .iter()
        .enumerate()
        .map(|(i, turn)| {
            let turn_num = turns.len() - i; // chronological: oldest=#1, newest=#N

            // Format relative time from pre-parsed timestamp
            let time_display = timestamps
                .get(i)
                .and_then(|ts| ts.as_ref())
                .map(format_relative_time)
                .unwrap_or_else(|| "---".to_string());

            let time_color = timestamps
                .get(i)
                .and_then(|ts| ts.as_ref())
                .map(elapsed_time_color)
                .unwrap_or(Color::DarkGray);

            // First line of user prompt, truncated to fit
            let first_line = turn.user_prompt.lines().next().unwrap_or("");

            // Calculate space: "#NN " + "XXs " + prompt
            let num_str = format!("#{}", turn_num);
            let prefix_len = num_str.len() + 1 + time_display.len() + 1;
            let prompt_max = inner_width.saturating_sub(prefix_len);
            let truncated_prompt: String = if first_line.chars().count() > prompt_max {
                let s: String = first_line
                    .chars()
                    .take(prompt_max.saturating_sub(3))
                    .collect();
                format!("{}...", s)
            } else {
                first_line.to_string()
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("{} ", num_str),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("{} ", time_display),
                    Style::default().fg(time_color),
                ),
                Span::raw(truncated_prompt),
            ]);
            ListItem::new(line)
        })
        .collect();

    let title = format!(" History ({} turns) ", turns.len());
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    f.render_stateful_widget(list, area, list_state);
}

/// Render details panel in history browsing mode.
fn render_history_details(
    f: &mut ratatui::Frame,
    area: Rect,
    turns: &[ConversationTurn],
    index: usize,
    scroll_offset: &mut usize,
    cached_history_lines: &mut HistoryLinesCache,
) {
    let turn = &turns[index];
    let total = turns.len();
    let turn_num = total - index; // Display as 1-based chronological number
    let inner_width = (area.width.saturating_sub(2)) as usize;

    // No max_lines limit - content is scrollable via Ctrl+D/Ctrl+U
    let max_lines = usize::MAX;

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Prompt section
    lines.push(Line::from(vec![Span::styled(
        "💬 Prompt:",
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(Color::Cyan),
    )]));

    let prompt_lines = wrap_text_lines(&turn.user_prompt, inner_width, max_lines, Color::White);
    lines.extend(prompt_lines);

    // Separator
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "─".repeat(inner_width),
        Style::default().fg(Color::DarkGray),
    )]));

    // Response section
    lines.push(Line::from(vec![Span::styled(
        "🤖 Response:",
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(Color::Green),
    )]));

    if turn.assistant_response.is_empty() {
        lines.push(Line::from(Span::styled(
            "(no response yet)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        // Use cached markdown lines if the content hasn't changed.
        // Cache key includes width so a resize regenerates lines.
        let text_hash = hash_str(&turn.assistant_response);
        let cache_key = (text_hash, inner_width);
        let response_lines = if let Some((cached_key, cached)) = cached_history_lines.as_ref() {
            if *cached_key == cache_key {
                cached.clone()
            } else {
                let rendered = markdown::markdown_to_lines(&turn.assistant_response, inner_width);
                *cached_history_lines = Some((cache_key, rendered.clone()));
                rendered
            }
        } else {
            let rendered = markdown::markdown_to_lines(&turn.assistant_response, inner_width);
            *cached_history_lines = Some((cache_key, rendered.clone()));
            rendered
        };
        lines.extend(response_lines);
    }

    // Clamp scroll offset to prevent overscroll beyond content.
    // Lines are pre-wrapped so lines.len() == actual visual row count.
    let content_height = lines.len();
    let viewport_height = area.height.saturating_sub(2) as usize; // minus borders
    let max_scroll = content_height.saturating_sub(viewport_height);
    *scroll_offset = (*scroll_offset).min(max_scroll);
    let clamped_offset = (*scroll_offset).min(u16::MAX as usize) as u16;

    let title = format!(" History ({}/{}) ", turn_num, total);
    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .scroll((clamped_offset, 0));

    f.render_widget(paragraph, area);
}

/// Render the footer with keybindings help.
pub fn render_footer(
    f: &mut ratatui::Frame,
    area: Rect,
    input_mode: bool,
    history_view: HistoryViewMode,
    toast: Option<&super::toast::Toast>,
    kill_confirm: Option<&(u32, String)>,
    add_pane_pending: Option<&(u32, String)>,
) {
    // Show toast if active (overrides footer)
    if let Some(toast) = toast {
        let (color, prefix) = match toast.toast_type {
            super::toast::ToastType::Success => (Color::Green, "✓"),
            super::toast::ToastType::Error => (Color::Red, "✗"),
        };
        let toast_text = Line::from(vec![
            Span::styled(format!("{} ", prefix), Style::default().fg(color)),
            Span::styled(&toast.message, Style::default().fg(color)),
        ]);
        let paragraph = Paragraph::new(toast_text);
        f.render_widget(paragraph, area);
        return;
    }

    // Show kill confirmation prompt if active (overrides normal footer)
    if let Some((_pane_id, label)) = kill_confirm {
        let confirm_text = Line::from(vec![
            Span::styled("Kill ", Style::default().fg(Color::Red)),
            Span::styled(
                label.as_str(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("? ", Style::default().fg(Color::Red)),
            Span::styled(
                "[y]",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw("es / "),
            Span::styled("[any]", Style::default().fg(Color::Cyan)),
            Span::raw("cancel"),
        ]);
        let paragraph = Paragraph::new(confirm_text);
        f.render_widget(paragraph, area);
        return;
    }

    // Show add-pane direction selection prompt if active
    if let Some((_pane_id, _cwd)) = add_pane_pending {
        let prompt_text = Line::from(vec![
            Span::styled("Add pane: ", Style::default().fg(Color::Green)),
            Span::styled(
                "[r]",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("ight / "),
            Span::styled(
                "[d]",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("own / "),
            Span::styled(
                "[t]",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("ab / "),
            Span::styled("[any]", Style::default().fg(Color::Cyan)),
            Span::raw("cancel"),
        ]);
        let paragraph = Paragraph::new(prompt_text);
        f.render_widget(paragraph, area);
        return;
    }

    let help_text = if history_view == HistoryViewMode::List {
        Line::from(vec![
            Span::styled("[jk]", Style::default().fg(Color::Yellow)),
            Span::raw("Select "),
            Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
            Span::raw("Open "),
            Span::styled("[gg]", Style::default().fg(Color::Yellow)),
            Span::raw("Newest "),
            Span::styled("[G]", Style::default().fg(Color::Yellow)),
            Span::raw("Oldest "),
            Span::styled("[Esc/q]", Style::default().fg(Color::Yellow)),
            Span::raw("Back"),
        ])
    } else if history_view == HistoryViewMode::Detail {
        Line::from(vec![
            Span::styled("[jk]", Style::default().fg(Color::Yellow)),
            Span::raw("Scroll "),
            Span::styled("[^D/^U]", Style::default().fg(Color::Yellow)),
            Span::raw("HalfPage "),
            Span::styled("[gg]", Style::default().fg(Color::Yellow)),
            Span::raw("Top "),
            Span::styled("[G]", Style::default().fg(Color::Yellow)),
            Span::raw("Bottom "),
            Span::styled("[Esc/q]", Style::default().fg(Color::Yellow)),
            Span::raw("Back"),
        ])
    } else if input_mode {
        Line::from(vec![
            Span::styled("[Enter]", Style::default().fg(Color::Cyan)),
            Span::raw("Send "),
            Span::styled("[^O]", Style::default().fg(Color::Cyan)),
            Span::raw("Newline "),
            Span::styled("[^hjkl]", Style::default().fg(Color::Cyan)),
            Span::raw("Move "),
            Span::styled("[Esc]", Style::default().fg(Color::Cyan)),
            Span::raw("Cancel "),
            Span::styled("[^U]", Style::default().fg(Color::Cyan)),
            Span::raw("Clear"),
        ])
    } else {
        Line::from(vec![
            Span::styled("[↑↓/jk]", Style::default().fg(Color::Cyan)),
            Span::raw("Select "),
            Span::styled("[Enter]", Style::default().fg(Color::Cyan)),
            Span::raw("Focus "),
            Span::styled("[i]", Style::default().fg(Color::Cyan)),
            Span::raw("Prompt "),
            Span::styled("[1-9]", Style::default().fg(Color::Cyan)),
            Span::raw("Quick "),
            Span::styled("[h/l]", Style::default().fg(Color::Cyan)),
            Span::raw("Resize "),
            Span::styled("[H]", Style::default().fg(Color::Cyan)),
            Span::raw("History "),
            Span::styled("[r]", Style::default().fg(Color::Cyan)),
            Span::raw("Refresh "),
            Span::styled("[x]", Style::default().fg(Color::Cyan)),
            Span::raw("Kill "),
            Span::styled("[a]", Style::default().fg(Color::Cyan)),
            Span::raw("Add "),
            Span::styled("[q]", Style::default().fg(Color::Cyan)),
            Span::raw("Quit"),
        ])
    };

    let paragraph = Paragraph::new(help_text).style(Style::default().fg(Color::DarkGray));

    f.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // --- format_duration tests ---

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
        assert_eq!(format_duration(Duration::from_secs(90)), "1m"); // truncates
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

    // --- format_relative_time tests ---

    #[test]
    fn test_format_relative_time_recent() {
        let time = SystemTime::now() - Duration::from_secs(5);
        let result = format_relative_time(&time);
        // Should be "5s" or close (depending on timing)
        assert!(result.ends_with('s'));
    }

    #[test]
    fn test_format_relative_time_future() {
        // Time in the future should return "now"
        let time = SystemTime::now() + Duration::from_secs(100);
        assert_eq!(format_relative_time(&time), "now");
    }

    // --- color_for_elapsed tests ---

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

    // --- elapsed_time_color tests ---

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
