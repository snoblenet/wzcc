/// A text input buffer with cursor management.
///
/// Supports multi-line editing with character-boundary-aware cursor movement.
/// All positions are tracked as byte offsets into the underlying UTF-8 string.
#[derive(Debug, Clone)]
pub struct InputBuffer {
    /// The text content
    buffer: String,
    /// Cursor position (byte offset)
    cursor: usize,
}

impl Default for InputBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl InputBuffer {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
        }
    }

    /// Get the buffer content as a string slice.
    pub fn as_str(&self) -> &str {
        &self.buffer
    }

    /// Get the cursor position (byte offset).
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Return true if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Clear the buffer and reset cursor to 0.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
    }

    /// Insert a character at the current cursor position.
    /// Always returns true (insertion always changes state).
    pub fn insert_char(&mut self, c: char) -> bool {
        self.buffer.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        true
    }

    /// Insert a string at the current cursor position.
    /// Returns true if the string is non-empty (insertion changes state).
    pub fn insert_str(&mut self, s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        self.buffer.insert_str(self.cursor, s);
        self.cursor += s.len();
        true
    }

    /// Delete the character before the cursor (backspace).
    /// Returns true if a character was deleted, false if cursor was already at start.
    pub fn backspace(&mut self) -> bool {
        if self.cursor > 0 {
            let prev = self.buffer[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.buffer.drain(prev..self.cursor);
            self.cursor = prev;
            true
        } else {
            false
        }
    }

    /// Move cursor one character to the left.
    /// Returns true if the cursor moved.
    pub fn cursor_left(&mut self) -> bool {
        if self.cursor > 0 {
            let prev = self.buffer[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.cursor = prev;
            true
        } else {
            false
        }
    }

    /// Move cursor one character to the right.
    /// Returns true if the cursor moved.
    pub fn cursor_right(&mut self) -> bool {
        if self.cursor < self.buffer.len() {
            let next = self.buffer[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(self.buffer.len());
            self.cursor = next;
            true
        } else {
            false
        }
    }

    /// Move cursor to the start of the current line.
    /// Returns true if the cursor moved.
    pub fn cursor_home(&mut self) -> bool {
        let before = &self.buffer[..self.cursor];
        let new_pos = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        if new_pos != self.cursor {
            self.cursor = new_pos;
            true
        } else {
            false
        }
    }

    /// Move cursor to the end of the current line.
    /// Returns true if the cursor moved.
    pub fn cursor_end(&mut self) -> bool {
        let after = &self.buffer[self.cursor..];
        let offset = after.find('\n').unwrap_or(after.len());
        if offset > 0 {
            self.cursor += offset;
            true
        } else {
            false
        }
    }

    /// Move cursor up one line, preserving column position where possible.
    /// Returns true if the cursor moved (false if already on the first line).
    pub fn cursor_up(&mut self) -> bool {
        let before = &self.buffer[..self.cursor];
        if let Some(current_line_start) = before.rfind('\n') {
            let col = self.cursor - current_line_start - 1;
            let prev_line_start = before[..current_line_start]
                .rfind('\n')
                .map(|i| i + 1)
                .unwrap_or(0);
            let prev_line_len = current_line_start - prev_line_start;
            let target = prev_line_start + col.min(prev_line_len);
            self.cursor = self.snap_to_char_boundary(target);
            true
        } else {
            false
        }
    }

    /// Move cursor down one line, preserving column position where possible.
    /// Returns true if the cursor moved (false if already on the last line).
    pub fn cursor_down(&mut self) -> bool {
        let after = &self.buffer[self.cursor..];
        if let Some(next_newline) = after.find('\n') {
            let before = &self.buffer[..self.cursor];
            let current_line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
            let col = self.cursor - current_line_start;
            let next_line_start = self.cursor + next_newline + 1;
            let next_line_end = self.buffer[next_line_start..]
                .find('\n')
                .map(|i| next_line_start + i)
                .unwrap_or(self.buffer.len());
            let next_line_len = next_line_end - next_line_start;
            let target = next_line_start + col.min(next_line_len);
            self.cursor = self.snap_to_char_boundary(target);
            true
        } else {
            false
        }
    }

    /// Snap a byte position to the nearest valid UTF-8 character boundary
    /// at or before the given position.
    fn snap_to_char_boundary(&self, pos: usize) -> usize {
        let mut p = pos.min(self.buffer.len());
        while p > 0 && !self.buffer.is_char_boundary(p) {
            p -= 1;
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Basic operations ---

    #[test]
    fn test_new_buffer_is_empty() {
        let buf = InputBuffer::new();
        assert!(buf.is_empty());
        assert_eq!(buf.as_str(), "");
        assert_eq!(buf.cursor(), 0);
    }

    #[test]
    fn test_insert_char_ascii() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        buf.insert_char('b');
        buf.insert_char('c');
        assert_eq!(buf.as_str(), "abc");
        assert_eq!(buf.cursor(), 3);
    }

    #[test]
    fn test_insert_char_multibyte() {
        let mut buf = InputBuffer::new();
        buf.insert_char('日');
        buf.insert_char('本');
        assert_eq!(buf.as_str(), "日本");
        // Each CJK char is 3 bytes in UTF-8
        assert_eq!(buf.cursor(), 6);
    }

    #[test]
    fn test_insert_char_emoji() {
        let mut buf = InputBuffer::new();
        buf.insert_char('🦀');
        assert_eq!(buf.as_str(), "🦀");
        assert_eq!(buf.cursor(), 4); // emoji is 4 bytes
    }

    #[test]
    fn test_insert_at_middle() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        buf.insert_char('c');
        buf.cursor_left(); // cursor before 'c'
        buf.insert_char('b');
        assert_eq!(buf.as_str(), "abc");
    }

    #[test]
    fn test_clear() {
        let mut buf = InputBuffer::new();
        buf.insert_char('x');
        buf.insert_char('y');
        buf.clear();
        assert!(buf.is_empty());
        assert_eq!(buf.cursor(), 0);
    }

    // --- Backspace ---

    #[test]
    fn test_backspace_at_start_is_noop() {
        let mut buf = InputBuffer::new();
        assert!(!buf.backspace());
        assert!(buf.is_empty());
        assert_eq!(buf.cursor(), 0);
    }

    #[test]
    fn test_backspace_ascii() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        buf.insert_char('b');
        buf.backspace();
        assert_eq!(buf.as_str(), "a");
        assert_eq!(buf.cursor(), 1);
    }

    #[test]
    fn test_backspace_multibyte() {
        let mut buf = InputBuffer::new();
        buf.insert_char('日');
        buf.insert_char('本');
        buf.backspace();
        assert_eq!(buf.as_str(), "日");
        assert_eq!(buf.cursor(), 3);
    }

    #[test]
    fn test_backspace_in_middle() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        buf.insert_char('b');
        buf.insert_char('c');
        buf.cursor_left(); // before 'c'
        buf.backspace(); // delete 'b'
        assert_eq!(buf.as_str(), "ac");
        assert_eq!(buf.cursor(), 1);
    }

    #[test]
    fn test_backspace_all_chars() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        buf.backspace();
        assert!(buf.is_empty());
        assert_eq!(buf.cursor(), 0);
    }

    // --- Cursor left/right ---

    #[test]
    fn test_cursor_left_at_start_is_noop() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        assert!(buf.cursor_left()); // moves from 1 to 0
        assert_eq!(buf.cursor(), 0);
        assert!(!buf.cursor_left()); // already at 0, returns false
        assert_eq!(buf.cursor(), 0);
    }

    #[test]
    fn test_cursor_right_at_end_is_noop() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        assert!(!buf.cursor_right()); // already at end
        assert_eq!(buf.cursor(), 1);
    }

    #[test]
    fn test_cursor_left_right_roundtrip() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        buf.insert_char('b');
        let end = buf.cursor();
        buf.cursor_left();
        buf.cursor_right();
        assert_eq!(buf.cursor(), end);
    }

    #[test]
    fn test_cursor_movement_multibyte() {
        let mut buf = InputBuffer::new();
        buf.insert_char('あ'); // 3 bytes
        buf.insert_char('い'); // 3 bytes
        assert_eq!(buf.cursor(), 6);
        buf.cursor_left();
        assert_eq!(buf.cursor(), 3); // before 'い'
        buf.cursor_left();
        assert_eq!(buf.cursor(), 0); // before 'あ'
        buf.cursor_right();
        assert_eq!(buf.cursor(), 3); // after 'あ'
    }

    // --- Home / End ---

    #[test]
    fn test_cursor_home_single_line() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        buf.insert_char('b');
        buf.insert_char('c');
        buf.cursor_home();
        assert_eq!(buf.cursor(), 0);
    }

    #[test]
    fn test_cursor_end_single_line() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        buf.insert_char('b');
        buf.cursor_home();
        buf.cursor_end();
        assert_eq!(buf.cursor(), 2); // end of "ab"
    }

    #[test]
    fn test_cursor_home_multiline() {
        let mut buf = InputBuffer::new();
        // "abc\ndef" with cursor at end of "def"
        buf.insert_char('a');
        buf.insert_char('b');
        buf.insert_char('c');
        buf.insert_char('\n');
        buf.insert_char('d');
        buf.insert_char('e');
        buf.insert_char('f');
        buf.cursor_home();
        assert_eq!(buf.cursor(), 4); // start of "def" (after the '\n')
    }

    #[test]
    fn test_cursor_end_multiline() {
        let mut buf = InputBuffer::new();
        // "abc\ndef" with cursor at start of "abc"
        buf.insert_char('a');
        buf.insert_char('b');
        buf.insert_char('c');
        buf.insert_char('\n');
        buf.insert_char('d');
        buf.insert_char('e');
        buf.insert_char('f');
        buf.cursor_home(); // at start of "def" (byte 4)
        buf.cursor_home(); // still at byte 4 (no '\n' before in this line)
                           // Go to first line
        buf.cursor_up();
        buf.cursor_home();
        assert_eq!(buf.cursor(), 0);
        buf.cursor_end();
        assert_eq!(buf.cursor(), 3); // end of "abc", before '\n'
    }

    // --- Cursor up/down ---

    #[test]
    fn test_cursor_up_on_first_line_is_noop() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        buf.insert_char('b');
        let pos = buf.cursor();
        assert!(!buf.cursor_up()); // returns false
        assert_eq!(buf.cursor(), pos);
    }

    #[test]
    fn test_cursor_down_on_last_line_is_noop() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        buf.insert_char('b');
        let pos = buf.cursor();
        assert!(!buf.cursor_down()); // returns false
        assert_eq!(buf.cursor(), pos);
    }

    #[test]
    fn test_cursor_up_down_roundtrip() {
        let mut buf = InputBuffer::new();
        // "abc\ndef"
        for c in "abc\ndef".chars() {
            buf.insert_char(c);
        }
        // cursor at end of "def" (byte 7)
        assert_eq!(buf.cursor(), 7);
        buf.cursor_up();
        // Should be at col 3 of first line, but first line has 3 chars, so byte 3
        assert_eq!(buf.cursor(), 3);
        buf.cursor_down();
        assert_eq!(buf.cursor(), 7); // back to end of "def"
    }

    #[test]
    fn test_cursor_up_clamps_to_shorter_line() {
        let mut buf = InputBuffer::new();
        // "ab\ncdefg" - first line shorter than second
        for c in "ab\ncdefg".chars() {
            buf.insert_char(c);
        }
        // cursor at end of "cdefg" (col 5)
        buf.cursor_up();
        // first line "ab" has len 2, so cursor should be at col 2 = byte 2
        assert_eq!(buf.cursor(), 2);
    }

    #[test]
    fn test_cursor_down_clamps_to_shorter_line() {
        let mut buf = InputBuffer::new();
        // "abcde\nfg" - second line shorter than first
        for c in "abcde\nfg".chars() {
            buf.insert_char(c);
        }
        // Move to end of first line
        buf.cursor_up();
        buf.cursor_end();
        assert_eq!(buf.cursor(), 5); // end of "abcde"
        buf.cursor_down();
        // second line "fg" has len 2, so cursor clamps to byte 6+2=8
        assert_eq!(buf.cursor(), 8);
    }

    #[test]
    fn test_cursor_up_down_three_lines() {
        let mut buf = InputBuffer::new();
        // "aa\nbbbb\ncc"
        for c in "aa\nbbbb\ncc".chars() {
            buf.insert_char(c);
        }
        // cursor at end: byte 10, line 3 col 2
        buf.cursor_up();
        // line 2 "bbbb", col 2 -> byte 3+2=5
        assert_eq!(buf.cursor(), 5);
        buf.cursor_up();
        // line 1 "aa", col 2 -> byte 2
        assert_eq!(buf.cursor(), 2);
        buf.cursor_down();
        assert_eq!(buf.cursor(), 5);
        buf.cursor_down();
        // line 3 "cc", col 2 -> byte 8+2=10
        assert_eq!(buf.cursor(), 10);
    }

    // --- Empty buffer edge cases ---

    #[test]
    fn test_empty_buffer_all_movements_are_noop() {
        let mut buf = InputBuffer::new();
        assert!(!buf.cursor_left());
        assert!(!buf.cursor_right());
        assert!(!buf.cursor_up());
        assert!(!buf.cursor_down());
        assert!(!buf.cursor_home());
        assert!(!buf.cursor_end());
        assert!(!buf.backspace());
        assert_eq!(buf.cursor(), 0);
        assert!(buf.is_empty());
    }

    // --- Newline handling ---

    #[test]
    fn test_insert_newline() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        buf.insert_char('\n');
        buf.insert_char('b');
        assert_eq!(buf.as_str(), "a\nb");
        assert_eq!(buf.cursor(), 3);
    }

    #[test]
    fn test_backspace_newline() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        buf.insert_char('\n');
        buf.insert_char('b');
        buf.cursor_left(); // before 'b'
        buf.backspace(); // delete '\n'
        assert_eq!(buf.as_str(), "ab");
    }

    // --- Mixed multibyte and newlines ---

    #[test]
    fn test_multibyte_multiline() {
        let mut buf = InputBuffer::new();
        // "日本\n語"
        for c in "日本\n語".chars() {
            buf.insert_char(c);
        }
        assert_eq!(buf.as_str(), "日本\n語");
        // "日本" = 6 bytes, "\n" = 1 byte, "語" = 3 bytes = 10 total
        assert_eq!(buf.cursor(), 10);
        buf.cursor_up();
        // line 1 "日本" col=3bytes(語), clamped to line len 6 -> byte 3 min 6 = 3
        // Actually col = cursor - line_start - 1 for the up logic
        // cursor=10, current_line_start = rfind('\n') in "日本\n" = byte 6
        // col = 10 - 6 - 1 = 3
        // prev_line_start = 0, prev_line_len = 6
        // result = 0 + min(3, 6) = 3
        assert_eq!(buf.cursor(), 3);
    }

    // --- cursor_up/down with mixed multibyte lines (char boundary safety) ---

    #[test]
    fn test_cursor_up_mixed_ascii_and_cjk_lines() {
        let mut buf = InputBuffer::new();
        // Line 1: "ab" (2 bytes), Line 2: "日本語" (9 bytes)
        // Moving up from end of line 2 (col=9) should clamp to line 1 len (2)
        buf.insert_str("ab\n日本語");
        assert_eq!(buf.cursor(), 12); // 2 + 1 + 9
        buf.cursor_up();
        // Should be at byte 2 (end of "ab"), not panic
        assert_eq!(buf.cursor(), 2);
        assert!(buf.buffer.is_char_boundary(buf.cursor()));
    }

    #[test]
    fn test_cursor_up_lands_mid_char_snaps_back() {
        let mut buf = InputBuffer::new();
        // Line 1: "日" (3 bytes), Line 2: "ab" (2 bytes)
        // col from line 2 = 2 bytes, applied to line 1 = byte 2 (mid-char!)
        // Should snap to byte 0 (before "日")
        buf.insert_str("日\nab");
        buf.cursor_up();
        assert!(buf.buffer.is_char_boundary(buf.cursor()));
        assert_eq!(buf.cursor(), 0);
    }

    #[test]
    fn test_cursor_down_lands_mid_char_snaps_back() {
        let mut buf = InputBuffer::new();
        // Line 1: "ab" (2 bytes), Line 2: "日" (3 bytes)
        // Move to end of line 1 (col=2), then down
        // col=2 applied to line 2 = byte offset 2 within "日" (mid-char!)
        // Should snap to byte 0 of line 2 (= overall byte 3)
        buf.insert_str("ab\n日");
        // cursor at end (byte 6). Move to end of line 1.
        buf.cursor_up();
        assert_eq!(buf.cursor(), 2); // end of "ab"
        buf.cursor_down();
        assert!(buf.buffer.is_char_boundary(buf.cursor()));
        // byte 3 (start of "日") since col=2 snaps back from mid-char
        assert_eq!(buf.cursor(), 3);
    }

    #[test]
    fn test_cursor_up_pasted_mixed_content_no_panic() {
        let mut buf = InputBuffer::new();
        // Simulate pasting mixed Japanese/ASCII multi-line content
        buf.insert_str("実装完了\ncommit & push\nテスト");
        // cursor at end. Press up twice - should not panic.
        assert!(buf.cursor_up());
        assert!(buf.buffer.is_char_boundary(buf.cursor()));
        assert!(buf.cursor_up());
        assert!(buf.buffer.is_char_boundary(buf.cursor()));
    }

    // --- insert_str ---

    #[test]
    fn test_insert_str_empty() {
        let mut buf = InputBuffer::new();
        assert!(!buf.insert_str(""));
        assert!(buf.is_empty());
        assert_eq!(buf.cursor(), 0);
    }

    #[test]
    fn test_insert_str_ascii() {
        let mut buf = InputBuffer::new();
        assert!(buf.insert_str("hello"));
        assert_eq!(buf.as_str(), "hello");
        assert_eq!(buf.cursor(), 5);
    }

    #[test]
    fn test_insert_str_multiline() {
        let mut buf = InputBuffer::new();
        buf.insert_str("line1\nline2\nline3");
        assert_eq!(buf.as_str(), "line1\nline2\nline3");
        assert_eq!(buf.cursor(), 17);
    }

    #[test]
    fn test_insert_str_at_middle() {
        let mut buf = InputBuffer::new();
        buf.insert_str("ac");
        buf.cursor_left(); // cursor before 'c'
        buf.insert_str("b");
        assert_eq!(buf.as_str(), "abc");
        assert_eq!(buf.cursor(), 2);
    }

    #[test]
    fn test_insert_str_multibyte() {
        let mut buf = InputBuffer::new();
        buf.insert_str("日本語");
        assert_eq!(buf.as_str(), "日本語");
        assert_eq!(buf.cursor(), 9); // 3 chars * 3 bytes
    }
}
