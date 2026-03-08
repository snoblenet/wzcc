use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders},
    Frame,
};

/// Convert a vt100::Color to a ratatui Color.
fn convert_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Convert vt100 cell attributes to a ratatui Style.
fn cell_style(cell: &vt100::Cell) -> Style {
    let mut style = Style::default();
    style = style.fg(convert_color(cell.fgcolor()));
    style = style.bg(convert_color(cell.bgcolor()));

    let mut modifier = Modifier::empty();
    if cell.bold() {
        modifier |= Modifier::BOLD;
    }
    if cell.italic() {
        modifier |= Modifier::ITALIC;
    }
    if cell.underline() {
        modifier |= Modifier::UNDERLINED;
    }
    if cell.inverse() {
        modifier |= Modifier::REVERSED;
    }
    style = style.add_modifier(modifier);
    style
}

/// Render the embedded terminal pane.
pub fn render_terminal_pane(
    f: &mut Frame,
    area: Rect,
    screen: &vt100::Screen,
    focused: bool,
    title: &str,
) {
    let border_color = if focused {
        Color::Green
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(format!(" {} ", title));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Render terminal cells into the inner area
    let buf = f.buffer_mut();
    render_screen_to_buffer(screen, inner, buf);

    // Draw cursor if focused
    if focused {
        let (cursor_row, cursor_col) = (screen.cursor_position().0, screen.cursor_position().1);
        let cursor_x = inner.x + cursor_col;
        let cursor_y = inner.y + cursor_row;
        if cursor_x < inner.right() && cursor_y < inner.bottom() {
            f.set_cursor_position((cursor_x, cursor_y));
        }
    }
}

/// Render vt100 screen cells into a ratatui buffer.
fn render_screen_to_buffer(screen: &vt100::Screen, area: Rect, buf: &mut Buffer) {
    let rows = area.height as usize;
    let cols = area.width as usize;

    for row in 0..rows {
        for col in 0..cols {
            let cell = screen.cell(row as u16, col as u16);
            if let Some(cell) = cell {
                let x = area.x + col as u16;
                let y = area.y + row as u16;
                if x < area.right() && y < area.bottom() {
                    let style = cell_style(cell);
                    let ch = cell.contents();
                    let buf_cell = &mut buf[(x, y)];
                    buf_cell.set_style(style);
                    if ch.is_empty() {
                        buf_cell.set_char(' ');
                    } else {
                        // Set first char; multi-byte chars are handled by the terminal
                        buf_cell.set_symbol(&ch);
                    }
                }
            }
        }
    }
}
