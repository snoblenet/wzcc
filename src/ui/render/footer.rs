use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::ui::toast::{Toast, ToastType};

use super::DetailMode;

/// Render the footer with keybindings help.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_footer(
    f: &mut ratatui::Frame,
    area: Rect,
    input_mode: bool,
    detail_mode: DetailMode,
    toast: Option<&Toast>,
    kill_confirm: Option<&(u32, String)>,
    add_pane_pending: Option<&(u32, String, u32)>,
    command_select_active: bool,
    slash_complete_active: bool,
    answer_select_active: bool,
    has_waiting_session: bool,
) {
    if let Some(toast) = toast {
        let (color, prefix) = match toast.toast_type {
            ToastType::Success => (Color::Green, "✓"),
            ToastType::Error => (Color::Red, "✗"),
        };
        let toast_text = Line::from(vec![
            Span::styled(format!("{} ", prefix), Style::default().fg(color)),
            Span::styled(&toast.message, Style::default().fg(color)),
        ]);
        let paragraph = Paragraph::new(toast_text);
        f.render_widget(paragraph, area);
        return;
    }

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

    if answer_select_active {
        let prompt_text = Line::from(vec![
            Span::styled("Answer: ", Style::default().fg(Color::Magenta)),
            Span::styled(
                "[jk]",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Select "),
            Span::styled(
                "[1-9]",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Quick "),
            Span::styled(
                "[Enter]",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Send "),
            Span::styled(
                "[Esc]",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Cancel"),
        ]);
        let paragraph = Paragraph::new(prompt_text);
        f.render_widget(paragraph, area);
        return;
    }

    if command_select_active {
        let prompt_text = Line::from(vec![
            Span::styled("Select command: ", Style::default().fg(Color::Green)),
            Span::styled(
                "[jk]",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Select "),
            Span::styled(
                "[Enter]",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Confirm "),
            Span::styled(
                "[Esc]",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Cancel"),
        ]);
        let paragraph = Paragraph::new(prompt_text);
        f.render_widget(paragraph, area);
        return;
    }

    if let Some((_pane_id, _cwd, _window_id)) = add_pane_pending {
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

    let help_text = if slash_complete_active {
        Line::from(vec![
            Span::styled(
                "[↑↓]",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Select "),
            Span::styled(
                "[Tab/Enter]",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Accept "),
            Span::styled(
                "[Esc]",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Dismiss"),
        ])
    } else if detail_mode == DetailMode::HistoryList {
        Line::from(vec![
            Span::styled("[jk]", Style::default().fg(Color::Yellow)),
            Span::raw("Select "),
            Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
            Span::raw("Open "),
            Span::styled("[y]", Style::default().fg(Color::Yellow)),
            Span::raw("Yank "),
            Span::styled("[gg]", Style::default().fg(Color::Yellow)),
            Span::raw("Newest "),
            Span::styled("[G]", Style::default().fg(Color::Yellow)),
            Span::raw("Oldest "),
            Span::styled("[h/l]", Style::default().fg(Color::Yellow)),
            Span::raw("Resize "),
            Span::styled("[Esc/q]", Style::default().fg(Color::Yellow)),
            Span::raw("Back"),
        ])
    } else if detail_mode == DetailMode::HistoryDetail {
        Line::from(vec![
            Span::styled("[jk]", Style::default().fg(Color::Yellow)),
            Span::raw("Scroll "),
            Span::styled("[^D/^U]", Style::default().fg(Color::Yellow)),
            Span::raw("HalfPage "),
            Span::styled("[y]", Style::default().fg(Color::Yellow)),
            Span::raw("Yank "),
            Span::styled("[gg]", Style::default().fg(Color::Yellow)),
            Span::raw("Top "),
            Span::styled("[G]", Style::default().fg(Color::Yellow)),
            Span::raw("Bottom "),
            Span::styled("[h/l]", Style::default().fg(Color::Yellow)),
            Span::raw("Resize "),
            Span::styled("[Esc/q]", Style::default().fg(Color::Yellow)),
            Span::raw("Back"),
        ])
    } else if detail_mode == DetailMode::LivePane {
        Line::from(vec![
            Span::styled("[jk]", Style::default().fg(Color::Green)),
            Span::raw("Scroll "),
            Span::styled("[^D/^U]", Style::default().fg(Color::Green)),
            Span::raw("HalfPage "),
            Span::styled("[y]", Style::default().fg(Color::Green)),
            Span::raw("Yank "),
            Span::styled("[gg]", Style::default().fg(Color::Green)),
            Span::raw("Top "),
            Span::styled("[G]", Style::default().fg(Color::Green)),
            Span::raw("Bottom "),
            Span::styled("[h/l]", Style::default().fg(Color::Green)),
            Span::raw("Resize "),
            Span::styled("[Esc/q]", Style::default().fg(Color::Green)),
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
        let mut spans = vec![
            Span::styled("[↑↓/jk]", Style::default().fg(Color::Cyan)),
            Span::raw("Select "),
            Span::styled("[Enter]", Style::default().fg(Color::Cyan)),
            Span::raw("Focus "),
            Span::styled("[y]", Style::default().fg(Color::Cyan)),
            Span::raw("Yank "),
            Span::styled("[^J/^K/^D/^U]", Style::default().fg(Color::Cyan)),
            Span::raw("Scroll "),
            Span::styled("[i]", Style::default().fg(Color::Cyan)),
            Span::raw("Prompt "),
            Span::styled("[1-9]", Style::default().fg(Color::Cyan)),
            Span::raw("Quick "),
            Span::styled("[h/l]", Style::default().fg(Color::Cyan)),
            Span::raw("Resize "),
            Span::styled("[H]", Style::default().fg(Color::Cyan)),
            Span::raw("History "),
            Span::styled("[v]", Style::default().fg(Color::Cyan)),
            Span::raw("Live "),
            Span::styled("[r]", Style::default().fg(Color::Cyan)),
            Span::raw("Refresh "),
            Span::styled("[x]", Style::default().fg(Color::Cyan)),
            Span::raw("Kill "),
            Span::styled("[a]", Style::default().fg(Color::Cyan)),
            Span::raw("Add "),
        ];
        if has_waiting_session {
            spans.push(Span::styled("[o]", Style::default().fg(Color::Magenta)));
            spans.push(Span::raw("Answer "));
        }
        spans.push(Span::styled("[q]", Style::default().fg(Color::Cyan)));
        spans.push(Span::raw("Quit"));
        Line::from(spans)
    };

    let paragraph = Paragraph::new(help_text).style(Style::default().fg(Color::DarkGray));
    f.render_widget(paragraph, area);
}
