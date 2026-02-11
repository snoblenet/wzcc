use crate::transcript::ConversationTurn;
use crate::ui::markdown;
use crate::ui::session::wrap_text_lines;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use std::hash::{Hash, Hasher};
use std::time::SystemTime;
use unicode_width::UnicodeWidthStr;

use super::summary::{elapsed_time_color, format_relative_time};
use super::HistoryLinesCache;

fn hash_str(s: &str) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

const HISTORY_LIST_HIGHLIGHT_SYMBOL: &str = ">> ";
const HISTORY_LIST_BORDER_WIDTH: usize = 2;

fn history_list_inner_width(area_width: u16, highlight_symbol: &str) -> usize {
    let reserved = HISTORY_LIST_BORDER_WIDTH + highlight_symbol.width();
    usize::from(area_width).saturating_sub(reserved)
}

fn truncate_history_prompt(
    first_line: &str,
    turn_num: usize,
    time_display: &str,
    inner_width: usize,
) -> String {
    let num_str = format!("#{}", turn_num);
    let prefix_len = num_str.len() + 1 + time_display.len() + 1;
    let prompt_max = inner_width.saturating_sub(prefix_len);

    if first_line.chars().count() > prompt_max {
        let s: String = first_line
            .chars()
            .take(prompt_max.saturating_sub(3))
            .collect();
        format!("{}...", s)
    } else {
        first_line.to_string()
    }
}

/// Render the history turn list in the details panel area.
pub(super) fn render_history_list(
    f: &mut ratatui::Frame,
    area: Rect,
    turns: &[ConversationTurn],
    list_state: &mut ListState,
    timestamps: &[Option<SystemTime>],
) {
    let inner_width = history_list_inner_width(area.width, HISTORY_LIST_HIGHLIGHT_SYMBOL);

    let items: Vec<ListItem> = turns
        .iter()
        .enumerate()
        .map(|(i, turn)| {
            let turn_num = turns.len() - i;

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

            let first_line = turn.user_prompt.lines().next().unwrap_or("");

            let num_str = format!("#{}", turn_num);
            let truncated_prompt =
                truncate_history_prompt(first_line, turn_num, &time_display, inner_width);

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
        .highlight_symbol(HISTORY_LIST_HIGHLIGHT_SYMBOL);

    f.render_stateful_widget(list, area, list_state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_history_list_inner_width_matches_highlight_symbol_width() {
        assert_eq!(history_list_inner_width(20, ">> "), 15);
    }

    #[test]
    fn test_history_list_inner_width_uses_unicode_display_width() {
        // Full-width greater-than signs are width=2 each.
        assert_eq!(history_list_inner_width(20, "＞＞ "), 13);
    }

    #[test]
    fn test_history_list_inner_width_saturates_at_zero() {
        assert_eq!(history_list_inner_width(4, ">> "), 0);
    }

    #[test]
    fn test_truncate_history_prompt_no_truncation() {
        let prompt = truncate_history_prompt("hello world", 12, "10m", 40);
        assert_eq!(prompt, "hello world");
    }

    #[test]
    fn test_truncate_history_prompt_narrow_width() {
        let prompt = truncate_history_prompt("abcdefghij", 999, "1h", 8);
        assert_eq!(prompt, "...");
    }
}

/// Render details panel in history browsing mode.
pub(super) fn render_history_details(
    f: &mut ratatui::Frame,
    area: Rect,
    turns: &[ConversationTurn],
    index: usize,
    scroll_offset: &mut usize,
    cached_history_lines: &mut HistoryLinesCache,
) {
    let turn = &turns[index];
    let total = turns.len();
    let turn_num = total - index;
    let inner_width = area.width.saturating_sub(2) as usize;

    let max_lines = usize::MAX;

    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::from(vec![Span::styled(
        "💬 Prompt:",
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(Color::Cyan),
    )]));

    let prompt_lines = wrap_text_lines(&turn.user_prompt, inner_width, max_lines, Color::White);
    lines.extend(prompt_lines);

    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "─".repeat(inner_width),
        Style::default().fg(Color::DarkGray),
    )]));

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

    let content_height = lines.len();
    let viewport_height = area.height.saturating_sub(2) as usize;
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
