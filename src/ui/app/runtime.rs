use super::*;

impl App {
    pub fn run(&mut self) -> Result<()> {
        // Clean up stale session mappings for TTYs that no longer exist
        // This prevents stale data from affecting new sessions on the same TTY
        self.cleanup_inactive_session_mappings();

        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        )?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Setup file watcher
        self.transcript_watcher = Some(TranscriptWatcher::new()?);

        // Initial refresh
        self.refresh()?;

        // Start watching transcript directories
        self.update_watched_dirs()?;

        // Event handler - shorter poll interval (100ms) since we're event-driven now
        // This is just for keyboard/mouse events, not for status updates
        let event_handler = EventHandler::new(100);

        // Track last full refresh time (for new session detection)
        let mut last_full_refresh = std::time::Instant::now();
        let full_refresh_interval = std::time::Duration::from_secs(5);

        // Main loop
        let result = loop {
            // Check for file changes from notify (lightweight transcript-only refresh)
            if self.drain_file_changes() && self.should_refresh_transcripts() {
                self.refresh_transcripts();

                // Check for actual changes in output
                let current_outputs: Vec<Option<String>> = self
                    .sessions
                    .iter()
                    .map(|s| s.last_output.clone())
                    .collect();

                if current_outputs != self.prev_last_outputs {
                    self.needs_full_redraw = true;
                    self.prev_last_outputs = current_outputs;
                }
            }

            // Only draw when dirty flag is set
            if self.dirty {
                // Clear terminal when full redraw is needed
                if self.needs_full_redraw {
                    terminal.clear()?;
                    self.needs_full_redraw = false;
                }
                terminal.draw(|f| self.render(f))?;
                self.dirty = false;
            }

            // Clear expired toast
            if let Some(ref toast) = self.toast {
                if toast.is_expired() {
                    self.toast = None;
                    self.dirty = true;
                }
            }

            // Event processing
            match event_handler.next()? {
                Event::Key(key) if self.input_mode => {
                    // Input mode key handling
                    if self.slash_complete_active {
                        // Slash autocomplete is active: intercept navigation keys
                        match key.code {
                            KeyCode::Esc => {
                                // Dismiss autocomplete only, stay in input mode
                                self.slash_complete_active = false;
                                self.slash_filtered.clear();
                                self.dirty = true;
                            }
                            KeyCode::Tab | KeyCode::Enter => {
                                // Accept the selected completion
                                self.accept_slash_completion();
                            }
                            KeyCode::Up | KeyCode::Char('k')
                                if key.code == KeyCode::Up
                                    || key.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                // Navigate up in autocomplete list
                                if let Some(i) = self.slash_complete_state.selected() {
                                    if i > 0 {
                                        self.slash_complete_state.select(Some(i - 1));
                                        self.dirty = true;
                                    }
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j')
                                if key.code == KeyCode::Down
                                    || key.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                // Navigate down in autocomplete list
                                if let Some(i) = self.slash_complete_state.selected() {
                                    let max = self.slash_filtered.len().saturating_sub(1);
                                    if i < max {
                                        self.slash_complete_state.select(Some(i + 1));
                                        self.dirty = true;
                                    }
                                }
                            }
                            KeyCode::Backspace => {
                                self.dirty |= self.input_buffer.backspace();
                                self.update_slash_filter();
                            }
                            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                self.input_buffer.clear();
                                self.slash_complete_active = false;
                                self.slash_filtered.clear();
                                self.dirty = true;
                            }
                            KeyCode::Char(_) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                // Ignore other Ctrl combinations (Ctrl+a/e/h/l/o etc.)
                                // to prevent them being inserted as literal characters
                            }
                            KeyCode::Char(c) => {
                                self.dirty |= self.input_buffer.insert_char(c);
                                self.update_slash_filter();
                            }
                            _ => {}
                        }
                    } else {
                        // Normal input mode (no autocomplete active)
                        match key.code {
                            KeyCode::Esc => {
                                self.exit_input_mode();
                            }
                            KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                // Ctrl+O -> newline
                                self.dirty |= self.input_buffer.insert_char('\n');
                            }
                            KeyCode::Enter => {
                                // Enter -> submit
                                self.send_prompt()?;
                            }
                            KeyCode::Backspace => {
                                self.dirty |= self.input_buffer.backspace();
                                self.update_slash_filter();
                            }
                            KeyCode::Left => {
                                self.dirty |= self.input_buffer.cursor_left();
                                self.update_slash_filter();
                            }
                            KeyCode::Right => {
                                self.dirty |= self.input_buffer.cursor_right();
                                self.update_slash_filter();
                            }
                            KeyCode::Up => {
                                self.dirty |= self.input_buffer.cursor_up();
                                self.update_slash_filter();
                            }
                            KeyCode::Down => {
                                self.dirty |= self.input_buffer.cursor_down();
                                self.update_slash_filter();
                            }
                            KeyCode::Home => {
                                self.dirty |= self.input_buffer.cursor_home();
                                self.update_slash_filter();
                            }
                            KeyCode::End => {
                                self.dirty |= self.input_buffer.cursor_end();
                                self.update_slash_filter();
                            }
                            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                self.dirty |= self.input_buffer.cursor_home();
                                self.update_slash_filter();
                            }
                            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                self.dirty |= self.input_buffer.cursor_end();
                                self.update_slash_filter();
                            }
                            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                if !self.input_buffer.is_empty() {
                                    self.input_buffer.clear();
                                    self.slash_complete_active = false;
                                    self.slash_filtered.clear();
                                    self.dirty = true;
                                }
                            }
                            KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                self.dirty |= self.input_buffer.cursor_left();
                                self.update_slash_filter();
                            }
                            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                self.dirty |= self.input_buffer.cursor_down();
                                self.update_slash_filter();
                            }
                            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                self.dirty |= self.input_buffer.cursor_up();
                                self.update_slash_filter();
                            }
                            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                self.dirty |= self.input_buffer.cursor_right();
                                self.update_slash_filter();
                            }
                            KeyCode::Char(c) => {
                                self.dirty |= self.input_buffer.insert_char(c);
                                self.update_slash_filter();
                            }
                            _ => {}
                        }
                    }
                }
                Event::Paste(text) if self.input_mode => {
                    // Normalize line endings and insert pasted text
                    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
                    self.dirty |= self.input_buffer.insert_str(&normalized);
                    self.update_slash_filter();
                }
                Event::Key(key) if self.kill_confirm.is_some() => {
                    // Kill confirmation mode key handling
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            self.confirm_kill()?;
                        }
                        _ => {
                            self.cancel_kill();
                        }
                    }
                }
                Event::Key(key) if self.add_pane_pending.is_some() => {
                    // Add pane mode selection: split right, down, or new tab
                    match key.code {
                        KeyCode::Char('r') | KeyCode::Char('R') => {
                            self.select_command_or_spawn(SplitDirection::Right)?;
                        }
                        KeyCode::Char('d') | KeyCode::Char('D') => {
                            self.select_command_or_spawn(SplitDirection::Bottom)?;
                        }
                        KeyCode::Char('t') | KeyCode::Char('T') => {
                            self.select_command_or_spawn(SplitDirection::Tab)?;
                        }
                        _ => {
                            self.cancel_add_pane();
                        }
                    }
                }
                Event::Key(key) if self.command_select_pending.is_some() => {
                    // Command selection mode: pick which command to spawn
                    match key.code {
                        KeyCode::Char('j') | KeyCode::Down => {
                            let len = self.resolved_commands.len();
                            if let Some(i) = self.command_select_state.selected() {
                                if i + 1 < len {
                                    self.command_select_state.select(Some(i + 1));
                                    self.dirty = true;
                                }
                            }
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            if let Some(i) = self.command_select_state.selected() {
                                if i > 0 {
                                    self.command_select_state.select(Some(i - 1));
                                    self.dirty = true;
                                }
                            }
                        }
                        KeyCode::Enter => {
                            if let Some(i) = self.command_select_state.selected() {
                                self.confirm_command_select(i)?;
                            }
                        }
                        KeyCode::Esc => {
                            self.cancel_command_select();
                        }
                        _ => {}
                    }
                }
                Event::Key(key) if self.detail_mode == DetailMode::HistoryList => {
                    // History list view key handling
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('H') => {
                            self.exit_history_mode();
                        }
                        KeyCode::Enter => {
                            self.pending_g = false;
                            self.enter_history_detail();
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            self.pending_g = false;
                            let len = self.history_turns.len();
                            if let Some(i) = self.history_list_state.selected() {
                                if i + 1 < len {
                                    self.history_list_state.select(Some(i + 1));
                                    self.dirty = true;
                                }
                            }
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            self.pending_g = false;
                            if let Some(i) = self.history_list_state.selected() {
                                if i > 0 {
                                    self.history_list_state.select(Some(i - 1));
                                    self.dirty = true;
                                }
                            }
                        }
                        KeyCode::Char('g') => {
                            if self.pending_g {
                                // gg -> jump to newest (first in list)
                                self.history_list_state.select(Some(0));
                                self.dirty = true;
                                self.pending_g = false;
                            } else {
                                self.pending_g = true;
                            }
                        }
                        KeyCode::Char('G') => {
                            // G -> jump to oldest (last in list)
                            self.pending_g = false;
                            if !self.history_turns.is_empty() {
                                self.history_list_state
                                    .select(Some(self.history_turns.len() - 1));
                                self.dirty = true;
                            }
                        }
                        KeyCode::Char('y') => {
                            // y -> yank selected history turn's response to clipboard
                            self.pending_g = false;
                            if let Some(i) = self.history_list_state.selected() {
                                self.yank_history_output(i);
                            }
                        }
                        KeyCode::Char('h') => {
                            self.pending_g = false;
                            if self.details_width_percent < 80 {
                                self.details_width_percent += 5;
                                self.dirty = true;
                                self.needs_full_redraw = true;
                            }
                        }
                        KeyCode::Char('l') => {
                            self.pending_g = false;
                            if self.details_width_percent > 20 {
                                self.details_width_percent -= 5;
                                self.dirty = true;
                                self.needs_full_redraw = true;
                            }
                        }
                        _ => {
                            self.pending_g = false;
                        }
                    }
                }
                Event::Key(key) if self.detail_mode == DetailMode::HistoryDetail => {
                    // History detail view key handling
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => {
                            // Back to list (NOT exit history entirely)
                            self.exit_history_detail();
                        }
                        KeyCode::Char('H') => {
                            // H in detail -> back to list
                            self.exit_history_detail();
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            // Scroll content down line-by-line
                            self.pending_g = false;
                            self.history_scroll_offset =
                                self.history_scroll_offset.saturating_add(1);
                            self.dirty = true;
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            // Scroll content up line-by-line
                            self.pending_g = false;
                            self.history_scroll_offset =
                                self.history_scroll_offset.saturating_sub(1);
                            self.dirty = true;
                        }
                        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            // Ctrl+D -> scroll down half page
                            self.pending_g = false;
                            let half = self.viewport_half_height();
                            self.history_scroll_offset =
                                self.history_scroll_offset.saturating_add(half);
                            self.dirty = true;
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            // Ctrl+U -> scroll up half page
                            self.pending_g = false;
                            let half = self.viewport_half_height();
                            self.history_scroll_offset =
                                self.history_scroll_offset.saturating_sub(half);
                            self.dirty = true;
                        }
                        KeyCode::Char('g') => {
                            if self.pending_g {
                                // gg -> scroll to top
                                self.history_scroll_offset = 0;
                                self.dirty = true;
                                self.pending_g = false;
                            } else {
                                self.pending_g = true;
                            }
                        }
                        KeyCode::Char('G') => {
                            // G -> scroll to bottom (clamped in render)
                            self.pending_g = false;
                            self.history_scroll_offset = usize::MAX;
                            self.dirty = true;
                        }
                        KeyCode::Char('y') => {
                            // y -> yank current turn's response to clipboard
                            self.pending_g = false;
                            self.yank_history_output(self.history_index);
                        }
                        KeyCode::Char('h') => {
                            self.pending_g = false;
                            if self.details_width_percent < 80 {
                                self.details_width_percent += 5;
                                self.dirty = true;
                                self.needs_full_redraw = true;
                            }
                        }
                        KeyCode::Char('l') => {
                            self.pending_g = false;
                            if self.details_width_percent > 20 {
                                self.details_width_percent -= 5;
                                self.dirty = true;
                                self.needs_full_redraw = true;
                            }
                        }
                        _ => {
                            self.pending_g = false;
                        }
                    }
                }
                Event::Key(key) if self.detail_mode == DetailMode::LivePane => {
                    // Live pane view key handling
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('v') => {
                            self.exit_live_pane_view();
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            self.pending_g = false;
                            self.live_pane_scroll_offset =
                                self.live_pane_scroll_offset.saturating_add(1);
                            self.dirty = true;
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            self.pending_g = false;
                            self.live_pane_scroll_offset =
                                self.live_pane_scroll_offset.saturating_sub(1);
                            self.dirty = true;
                        }
                        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            self.pending_g = false;
                            let half_page = self.viewport_half_height();
                            self.live_pane_scroll_offset =
                                self.live_pane_scroll_offset.saturating_add(half_page);
                            self.dirty = true;
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            self.pending_g = false;
                            let half_page = self.viewport_half_height();
                            self.live_pane_scroll_offset =
                                self.live_pane_scroll_offset.saturating_sub(half_page);
                            self.dirty = true;
                        }
                        KeyCode::Char('g') => {
                            if self.pending_g {
                                self.live_pane_scroll_offset = 0;
                                self.dirty = true;
                                self.pending_g = false;
                            } else {
                                self.pending_g = true;
                            }
                        }
                        KeyCode::Char('G') => {
                            self.pending_g = false;
                            self.live_pane_scroll_offset = usize::MAX;
                            self.dirty = true;
                        }
                        KeyCode::Char('y') => {
                            self.pending_g = false;
                            self.yank_live_pane_content();
                        }
                        KeyCode::Char('h') => {
                            self.pending_g = false;
                            if self.details_width_percent < 80 {
                                self.details_width_percent += 5;
                                self.dirty = true;
                                self.needs_full_redraw = true;
                            }
                        }
                        KeyCode::Char('l') => {
                            self.pending_g = false;
                            if self.details_width_percent > 20 {
                                self.details_width_percent -= 5;
                                self.dirty = true;
                                self.needs_full_redraw = true;
                            }
                        }
                        _ => {
                            self.pending_g = false;
                        }
                    }
                }
                Event::Key(key) => {
                    // Normal mode key handling
                    // Handle gg sequence
                    if self.pending_g {
                        self.pending_g = false;
                        if key.code == KeyCode::Char('g') {
                            // gg -> jump to first
                            self.select_first();
                            continue;
                        }
                        // Reset pending if different key comes after g
                    }

                    if is_quit_key(&key) {
                        break Ok(());
                    } else if is_down_key(&key) {
                        self.select_next();
                    } else if is_up_key(&key) {
                        self.select_previous();
                    } else if key.code == KeyCode::Char('g') {
                        // First g -> set pending state
                        self.pending_g = true;
                    } else if key.code == KeyCode::Char('G') {
                        // G -> jump to last
                        self.select_last();
                    } else if is_enter_key(&key) {
                        // Try to jump (TUI continues)
                        let _ = self.jump_to_selected();
                    } else if key.code == KeyCode::Char('h') {
                        // Expand details panel (move divider left)
                        if self.details_width_percent < 80 {
                            self.details_width_percent += 5;
                            self.dirty = true;
                            self.needs_full_redraw = true;
                        }
                    } else if key.code == KeyCode::Char('l') {
                        // Shrink details panel (move divider right)
                        if self.details_width_percent > 20 {
                            self.details_width_percent -= 5;
                            self.dirty = true;
                            self.needs_full_redraw = true;
                        }
                    } else if key.code == KeyCode::Char('i') {
                        // Enter input mode
                        self.enter_input_mode();
                    } else if key.code == KeyCode::Char('x') {
                        // Request kill for selected session (shows confirmation)
                        self.request_kill_selected();
                    } else if key.code == KeyCode::Char('y') {
                        // Yank selected session's last output to clipboard
                        self.yank_selected_output();
                    } else if key.code == KeyCode::Char('H') {
                        // Enter history browsing mode
                        self.enter_history_mode();
                    } else if key.code == KeyCode::Char('v') {
                        // Enter live pane view
                        self.enter_live_pane_view();
                    } else if key.code == KeyCode::Char('a') {
                        // Enter add-pane mode (split direction selection)
                        self.request_add_pane();
                    } else if is_refresh_key(&key) {
                        // Show refreshing indicator then update
                        self.refreshing = true;
                        self.dirty = true;
                        terminal.draw(|f| self.render(f))?;
                        self.git_branch_cache.clear();
                        self.refresh()?;
                        self.refreshing = false;
                    } else if let KeyCode::Char(c) = key.code {
                        // Quick select with number keys [1-9]
                        if let Some(digit) = c.to_digit(10) {
                            if (1..=9).contains(&digit) {
                                let index = (digit - 1) as usize;
                                if index < self.sessions.len() {
                                    self.list_state.select(Some(index));
                                    self.exit_live_pane_view();
                                    self.dirty = true;
                                    // Also jump to the session
                                    let _ = self.jump_to_selected();
                                }
                            }
                        }
                    }
                }
                Event::Mouse(mouse)
                    if self.input_mode
                        || self.detail_mode != DetailMode::Summary
                        || self.command_select_pending.is_some() =>
                {
                    // Ignore mouse in input mode, history mode, live pane mode, and command selection
                    let _ = mouse;
                }
                Event::Mouse(mouse) => {
                    // Handle left click only
                    if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                        // Check if click is inside list area
                        if let Some(area) = self.list_area {
                            if mouse.column >= area.x
                                && mouse.column < area.x + area.width
                                && mouse.row >= area.y
                                && mouse.row < area.y + area.height
                            {
                                // Relative row excluding border and title (first row)
                                let relative_row = mouse.row.saturating_sub(area.y + 1);

                                // Calculate clicked session index
                                if let Some(idx) = self.row_to_session_index(relative_row as usize)
                                {
                                    let now = std::time::Instant::now();

                                    // Double click detection (click same item within 300ms)
                                    let is_double_click = self
                                        .last_click
                                        .map(|(time, last_idx)| {
                                            last_idx == idx
                                                && now.duration_since(time).as_millis() < 300
                                        })
                                        .unwrap_or(false);

                                    if is_double_click {
                                        // Double click -> jump
                                        self.list_state.select(Some(idx));
                                        let _ = self.jump_to_selected();
                                        self.last_click = None;
                                    } else {
                                        // Single click -> select
                                        self.list_state.select(Some(idx));
                                        self.dirty = true;
                                        self.last_click = Some((now, idx));
                                    }
                                }
                            }
                        }
                    }
                }
                Event::Paste(_) => {
                    // Ignore paste events outside input mode
                }
                Event::Resize(_, _) => {
                    self.dirty = true;
                }
                Event::Tick => {
                    // Poll live pane content if active
                    self.poll_live_pane_content();

                    // Advance animation frame for Processing indicator
                    self.animation_frame = (self.animation_frame + 1) % 4;

                    // Trigger redraw if any session is Processing (for animation)
                    let has_processing = self
                        .sessions
                        .iter()
                        .any(|s| matches!(s.status, crate::transcript::SessionStatus::Processing));
                    if has_processing {
                        self.dirty = true;
                    }

                    // Flush pending transcript refresh (trailing-edge debounce)
                    if self.pending_transcript_refresh
                        && self.last_transcript_refresh.elapsed()
                            >= Duration::from_millis(TRANSCRIPT_DEBOUNCE_MS)
                    {
                        self.refresh_transcripts();
                        self.pending_transcript_refresh = false;
                        self.last_transcript_refresh = Instant::now();

                        let current_outputs: Vec<Option<String>> = self
                            .sessions
                            .iter()
                            .map(|s| s.last_output.clone())
                            .collect();
                        if current_outputs != self.prev_last_outputs {
                            self.needs_full_redraw = true;
                            self.prev_last_outputs = current_outputs;
                        }
                    }

                    // Periodic full refresh for new session detection (every 5 seconds)
                    if last_full_refresh.elapsed() >= full_refresh_interval {
                        self.refresh()?;
                        self.update_watched_dirs()?;
                        last_full_refresh = std::time::Instant::now();

                        // Check for actual changes in output
                        let current_outputs: Vec<Option<String>> = self
                            .sessions
                            .iter()
                            .map(|s| s.last_output.clone())
                            .collect();

                        if current_outputs != self.prev_last_outputs {
                            self.needs_full_redraw = true;
                            self.prev_last_outputs = current_outputs;
                        }
                    }
                }
            }
        };

        // Cleanup terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            DisableBracketedPaste
        )?;
        terminal.show_cursor()?;

        result
    }

    /// Render
    fn render(&mut self, f: &mut ratatui::Frame) {
        let size = f.area();

        // Vertical layout: main content + footer
        let vertical_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(size);

        let main_area = vertical_chunks[0];
        let footer_area = vertical_chunks[1];

        // 2-column layout (left: list, right: details - resizable with h/l)
        let list_percent = 100 - self.details_width_percent;
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(list_percent),
                Constraint::Percentage(self.details_width_percent),
            ])
            .split(main_area);

        // Render list (update list_area)
        self.list_area = render_list(
            f,
            chunks[0],
            &self.sessions,
            &mut self.list_state,
            self.refreshing,
            self.animation_frame,
            &self.current_workspace,
        );

        // Render details
        let mut ctx = DetailsRenderCtx {
            sessions: &self.sessions,
            selected: self.list_state.selected(),
            input_mode: self.input_mode,
            input_buffer: self.input_buffer.as_str(),
            cursor_position: self.input_buffer.cursor(),
            detail_mode: self.detail_mode,
            history_turns: &self.history_turns,
            history_index: self.history_index,
            history_scroll_offset: &mut self.history_scroll_offset,
            history_list_state: &mut self.history_list_state,
            history_timestamps: &self.history_timestamps,
            cached_history_lines: &mut self.cached_history_lines,
            cached_preview_lines: &mut self.cached_preview_lines,
            live_pane_bytes: self.live_pane_bytes.as_deref(),
            live_pane_bytes_hash: self.live_pane_bytes_hash,
            live_pane_scroll_offset: &mut self.live_pane_scroll_offset,
            cached_live_pane_lines: &mut self.cached_live_pane_lines,
            live_pane_error: self.live_pane_poll_failures > 0,
        };
        render_details(f, chunks[1], &mut ctx);

        // Render footer with keybindings help
        render_footer(
            f,
            footer_area,
            self.input_mode,
            self.detail_mode,
            self.toast.as_ref(),
            self.kill_confirm.as_ref(),
            self.add_pane_pending.as_ref(),
            self.command_select_pending.is_some(),
            self.slash_complete_active,
        );

        // Render slash command autocomplete popup (anchored to details area)
        if self.slash_complete_active && self.input_mode {
            render_slash_complete(
                f,
                chunks[1],
                &self.slash_commands,
                &self.slash_filtered,
                &mut self.slash_complete_state,
            );
        }

        // Render command selection popup overlay (on top of everything)
        if self.command_select_pending.is_some() {
            render_command_select(
                f,
                size,
                &self.resolved_commands,
                &mut self.command_select_state,
            );
        }
    }
}
