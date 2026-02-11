use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState},
};

use crate::config::SpawnCommand;

/// Render the command selection popup overlay.
pub(super) fn render_command_select(
    f: &mut ratatui::Frame,
    area: Rect,
    commands: &[SpawnCommand],
    list_state: &mut ListState,
) {
    // Calculate popup dimensions: 40% of terminal width, clamped to area
    let popup_width = (area.width * 40 / 100)
        .max(20)
        .min(area.width.saturating_sub(4));
    // +2 for top/bottom borders
    let popup_height = ((commands.len() as u16) + 2).min(area.height.saturating_sub(4));

    // Center the popup
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear the area behind the popup
    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = commands
        .iter()
        .map(|cmd| {
            let line = Line::from(vec![Span::raw(&cmd.name)]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Select Command ")
                .border_style(Style::default().fg(Color::Green)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    f.render_stateful_widget(list, popup_area, list_state);
}
