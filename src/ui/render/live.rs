use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use std::sync::Arc;

use super::LivePaneLinesCache;

/// Render the live pane view in the details panel area.
pub(super) fn render_live_pane(
    f: &mut ratatui::Frame,
    area: Rect,
    raw_bytes: Option<&[u8]>,
    content_hash: u64,
    scroll_offset: &mut usize,
    cached_lines: &mut LivePaneLinesCache,
    has_error: bool,
) {
    let inner_width = area.width.saturating_sub(2) as usize;

    let lines_arc: Arc<Vec<Line<'static>>> = if let Some(bytes) = raw_bytes {
        let cache_key = (content_hash, inner_width);

        if let Some((cached_key, cached)) = cached_lines.as_ref() {
            if *cached_key == cache_key {
                Arc::clone(cached)
            } else {
                let rendered = Arc::new(ansi_bytes_to_lines(bytes));
                *cached_lines = Some((cache_key, Arc::clone(&rendered)));
                rendered
            }
        } else {
            let rendered = Arc::new(ansi_bytes_to_lines(bytes));
            *cached_lines = Some((cache_key, Arc::clone(&rendered)));
            rendered
        }
    } else {
        Arc::new(vec![Line::from(Span::styled(
            "Loading...",
            Style::default().fg(Color::DarkGray),
        ))])
    };

    // Build the final line list. We only clone when an error line must be
    // appended; otherwise we borrow through the Arc.
    let (lines_ref, _owned);
    if has_error {
        let mut lines = (*lines_arc).clone();
        lines.push(Line::from(Span::styled(
            "⚠ Pane unavailable (retrying...)",
            Style::default().fg(Color::Yellow),
        )));
        _owned = lines;
        lines_ref = &_owned;
    } else {
        _owned = Vec::new(); // unused but needed to satisfy borrow checker
        lines_ref = lines_arc.as_ref();
    }

    let content_height = lines_ref.len();
    let viewport_height = area.height.saturating_sub(2) as usize;
    let max_scroll = content_height.saturating_sub(viewport_height);
    *scroll_offset = (*scroll_offset).min(max_scroll);
    let clamped_offset = (*scroll_offset).min(u16::MAX as usize) as u16;

    let border_color = if has_error {
        Color::Yellow
    } else {
        Color::Green
    };
    let paragraph = Paragraph::new(lines_ref.clone())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Live Pane ")
                .border_style(Style::default().fg(border_color)),
        )
        .scroll((clamped_offset, 0));

    f.render_widget(paragraph, area);
}

/// Remove SCS (Select Character Set) escape sequences from raw bytes.
fn strip_scs_sequences(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b
            && i + 2 < bytes.len()
            && (bytes[i + 1] == b'(' || bytes[i + 1] == b')')
            && (0x20..=0x7E).contains(&bytes[i + 2])
        {
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// Convert ANSI-escaped bytes to ratatui Lines using ansi-to-tui.
fn ansi_bytes_to_lines(bytes: &[u8]) -> Vec<Line<'static>> {
    use ansi_to_tui::IntoText as _;

    let cleaned: Vec<u8> = strip_scs_sequences(bytes);
    let bytes = cleaned.as_slice();

    match bytes.into_text() {
        Ok(text) => text
            .lines
            .into_iter()
            .map(|line| {
                let spans: Vec<Span<'static>> = line
                    .spans
                    .into_iter()
                    .map(|span| Span::styled(span.content.into_owned(), convert_style(span.style)))
                    .collect();
                Line::from(spans)
            })
            .collect(),
        Err(_) => {
            let s = String::from_utf8_lossy(bytes);
            s.lines().map(|l| Line::from(l.to_string())).collect()
        }
    }
}

/// Convert a `ratatui_core::Style` to a `ratatui::style::Style`.
fn convert_style(src: ratatui_core::style::Style) -> Style {
    let mut dst = Style::default();
    if let Some(fg) = src.fg {
        dst = dst.fg(convert_color(fg));
    }
    if let Some(bg) = src.bg {
        dst = dst.bg(convert_color(bg));
    }
    let add_bits = src.add_modifier.bits();
    let sub_bits = src.sub_modifier.bits();
    dst = dst.add_modifier(Modifier::from_bits_truncate(add_bits));
    dst = dst.remove_modifier(Modifier::from_bits_truncate(sub_bits));
    dst
}

/// Convert a `ratatui_core::Color` to a `ratatui::style::Color`.
fn convert_color(c: ratatui_core::style::Color) -> Color {
    use ratatui_core::style::Color as C;
    match c {
        C::Reset => Color::Reset,
        C::Black => Color::Black,
        C::Red => Color::Red,
        C::Green => Color::Green,
        C::Yellow => Color::Yellow,
        C::Blue => Color::Blue,
        C::Magenta => Color::Magenta,
        C::Cyan => Color::Cyan,
        C::Gray => Color::Gray,
        C::DarkGray => Color::DarkGray,
        C::LightRed => Color::LightRed,
        C::LightGreen => Color::LightGreen,
        C::LightYellow => Color::LightYellow,
        C::LightBlue => Color::LightBlue,
        C::LightMagenta => Color::LightMagenta,
        C::LightCyan => Color::LightCyan,
        C::White => Color::White,
        C::Rgb(r, g, b) => Color::Rgb(r, g, b),
        C::Indexed(i) => Color::Indexed(i),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ansi_bytes_to_lines_plain_text() {
        let bytes = b"hello\nworld";
        let lines = ansi_bytes_to_lines(bytes);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans.len(), 1);
        assert_eq!(lines[0].spans[0].content.as_ref(), "hello");
        assert_eq!(lines[1].spans[0].content.as_ref(), "world");
    }

    #[test]
    fn test_ansi_bytes_to_lines_with_color() {
        let bytes = b"\x1b[31mred text\x1b[0m";
        let lines = ansi_bytes_to_lines(bytes);
        assert_eq!(lines.len(), 1);
        assert!(lines[0]
            .spans
            .iter()
            .any(|s| s.content.contains("red text")));
        let red_span = lines[0]
            .spans
            .iter()
            .find(|s| s.content.contains("red text"))
            .unwrap();
        assert_eq!(red_span.style.fg, Some(Color::Red));
    }

    #[test]
    fn test_ansi_bytes_to_lines_empty() {
        let bytes = b"";
        let lines = ansi_bytes_to_lines(bytes);
        assert!(lines.len() <= 1);
    }

    #[test]
    fn test_ansi_bytes_to_lines_bold_modifier() {
        let bytes = b"\x1b[1mbold text\x1b[0m";
        let lines = ansi_bytes_to_lines(bytes);
        assert_eq!(lines.len(), 1);
        let bold_span = lines[0]
            .spans
            .iter()
            .find(|s| s.content.contains("bold text"))
            .unwrap();
        assert!(bold_span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_ansi_bytes_to_lines_rgb_color() {
        let bytes = b"\x1b[38;2;255;128;0morange\x1b[0m";
        let lines = ansi_bytes_to_lines(bytes);
        assert_eq!(lines.len(), 1);
        let span = lines[0]
            .spans
            .iter()
            .find(|s| s.content.contains("orange"))
            .unwrap();
        assert_eq!(span.style.fg, Some(Color::Rgb(255, 128, 0)));
    }

    #[test]
    fn test_ansi_bytes_to_lines_strips_scs_esc_b() {
        let bytes = b"hello\x1b(B world";
        let lines = ansi_bytes_to_lines(bytes);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(text, "hello world");
        assert!(!text.contains("(B"));
    }

    #[test]
    fn test_ansi_bytes_to_lines_strips_scs_esc_paren_zero() {
        let bytes = b"foo\x1b)0bar";
        let lines = ansi_bytes_to_lines(bytes);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(text, "foobar");
        assert!(!text.contains(")0"));
    }

    #[test]
    fn test_strip_scs_ignores_invalid_designator() {
        let bytes = b"a\x1b(\x01z";
        let result = strip_scs_sequences(bytes);
        assert_eq!(result, b"a\x1b(\x01z");
    }

    #[test]
    fn test_convert_color_named() {
        use ratatui_core::style::Color as C;
        assert_eq!(convert_color(C::Red), Color::Red);
        assert_eq!(convert_color(C::Blue), Color::Blue);
        assert_eq!(convert_color(C::Reset), Color::Reset);
    }

    #[test]
    fn test_convert_color_rgb() {
        use ratatui_core::style::Color as C;
        assert_eq!(convert_color(C::Rgb(10, 20, 30)), Color::Rgb(10, 20, 30));
    }

    #[test]
    fn test_convert_color_indexed() {
        use ratatui_core::style::Color as C;
        assert_eq!(convert_color(C::Indexed(42)), Color::Indexed(42));
    }

    #[test]
    fn test_convert_style_default() {
        let src = ratatui_core::style::Style::default();
        let dst = convert_style(src);
        assert_eq!(dst, Style::default());
    }

    #[test]
    fn test_convert_style_with_colors_and_modifiers() {
        let src = ratatui_core::style::Style::new()
            .fg(ratatui_core::style::Color::Green)
            .bg(ratatui_core::style::Color::Black)
            .add_modifier(
                ratatui_core::style::Modifier::BOLD | ratatui_core::style::Modifier::ITALIC,
            );
        let dst = convert_style(src);
        assert_eq!(dst.fg, Some(Color::Green));
        assert_eq!(dst.bg, Some(Color::Black));
        assert!(dst.add_modifier.contains(Modifier::BOLD));
        assert!(dst.add_modifier.contains(Modifier::ITALIC));
    }

    /// Helper: call render_live_pane inside a test terminal and return the
    /// final scroll_offset value.
    fn render_and_get_scroll_offset(
        content: Option<&[u8]>,
        content_hash: u64,
        initial_offset: usize,
        area_height: u16,
    ) -> usize {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let backend = TestBackend::new(80, area_height);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut offset = initial_offset;
        let mut cache: super::LivePaneLinesCache = None;
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, 80, area_height);
                render_live_pane(
                    f,
                    area,
                    content,
                    content_hash,
                    &mut offset,
                    &mut cache,
                    false,
                );
            })
            .unwrap();
        offset
    }

    #[test]
    fn test_scroll_offset_usize_max_clamps_to_bottom() {
        // 10 lines of content in a 5-row viewport (3 usable after borders)
        let content = b"1\n2\n3\n4\n5\n6\n7\n8\n9\n10";
        let offset = render_and_get_scroll_offset(Some(content), 42, usize::MAX, 5);
        // content_height=10, viewport_height=5-2=3, max_scroll=7
        assert_eq!(offset, 7);
    }

    #[test]
    fn test_scroll_offset_zero_stays_at_top() {
        let content = b"1\n2\n3\n4\n5\n6\n7\n8\n9\n10";
        let offset = render_and_get_scroll_offset(Some(content), 42, 0, 5);
        assert_eq!(offset, 0);
    }

    #[test]
    fn test_scroll_offset_clamped_when_content_shorter_than_viewport() {
        // 2 lines of content in a 10-row viewport -> max_scroll=0
        let content = b"hello\nworld";
        let offset = render_and_get_scroll_offset(Some(content), 42, usize::MAX, 10);
        assert_eq!(offset, 0);
    }

    #[test]
    fn test_cache_hit_same_hash_and_width() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let content = b"hello\nworld";
        let hash = 123;
        let mut cache: super::LivePaneLinesCache = None;

        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut offset = 0;

        // First render — populates cache
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, 80, 10);
                render_live_pane(f, area, Some(content), hash, &mut offset, &mut cache, false);
            })
            .unwrap();
        assert!(cache.is_some());
        let (cached_key, _) = cache.as_ref().unwrap();
        assert_eq!(*cached_key, (123, 78)); // 80 - 2 borders = 78

        // Second render with same hash — cache should still be the same key
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, 80, 10);
                render_live_pane(f, area, Some(content), hash, &mut offset, &mut cache, false);
            })
            .unwrap();
        let (cached_key2, _) = cache.as_ref().unwrap();
        assert_eq!(*cached_key2, (123, 78));
    }

    #[test]
    fn test_cache_miss_different_hash() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let content = b"hello\nworld";
        let mut cache: super::LivePaneLinesCache = None;

        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut offset = 0;

        // First render with hash=100
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, 80, 10);
                render_live_pane(f, area, Some(content), 100, &mut offset, &mut cache, false);
            })
            .unwrap();
        let (key1, _) = cache.as_ref().unwrap();
        assert_eq!(key1.0, 100);

        // Second render with hash=200 — cache should update
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, 80, 10);
                render_live_pane(f, area, Some(content), 200, &mut offset, &mut cache, false);
            })
            .unwrap();
        let (key2, _) = cache.as_ref().unwrap();
        assert_eq!(key2.0, 200);
    }
}
