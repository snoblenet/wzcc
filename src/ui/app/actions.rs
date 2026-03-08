use super::*;

impl App {
    pub(super) fn enter_input_mode(&mut self) {
        if self.list_state.selected().is_some() && !self.sessions.is_empty() {
            self.input_mode = true;
            self.input_buffer.clear();

            // Scan slash commands for the selected session's CWD
            let session_cwd = self
                .list_state
                .selected()
                .and_then(|i| self.sessions.get(i))
                .and_then(|s| s.pane.cwd_path());
            self.slash_commands =
                crate::ui::slash_commands::scan_slash_commands(session_cwd.as_deref());
            self.slash_filtered.clear();
            self.slash_complete_active = false;
            self.slash_complete_state.select(Some(0));

            self.dirty = true;
            self.needs_full_redraw = true;
        }
    }

    /// Exit input mode
    pub(super) fn exit_input_mode(&mut self) {
        self.input_mode = false;
        self.input_buffer.clear();
        self.slash_commands.clear();
        self.slash_filtered.clear();
        self.slash_complete_active = false;
        self.dirty = true;
        self.needs_full_redraw = true;
    }

    /// Extract the slash command prefix from the current line at cursor position.
    /// Returns the text after `/` if the current line starts with `/` and has no spaces.
    pub(super) fn slash_prefix(&self) -> Option<&str> {
        let buf = self.input_buffer.as_str();
        let cursor = self.input_buffer.cursor();
        let line_start = buf[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_to_cursor = &buf[line_start..cursor];
        if line_to_cursor.starts_with('/') && !line_to_cursor[1..].contains(' ') {
            Some(&line_to_cursor[1..])
        } else {
            None
        }
    }

    /// Update the slash command filter based on current input.
    pub(super) fn update_slash_filter(&mut self) {
        if let Some(prefix) = self.slash_prefix() {
            let prefix_owned = prefix.to_string();
            self.slash_filtered = self
                .slash_commands
                .iter()
                .enumerate()
                .filter(|(_, cmd)| cmd.name.starts_with(&prefix_owned))
                .map(|(i, _)| i)
                .collect();

            if self.slash_filtered.is_empty() {
                self.slash_complete_active = false;
            } else {
                self.slash_complete_active = true;
                // Clamp selection index
                let max = self.slash_filtered.len().saturating_sub(1);
                let current = self.slash_complete_state.selected().unwrap_or(0);
                if current > max {
                    self.slash_complete_state.select(Some(0));
                }
            }
        } else {
            self.slash_complete_active = false;
            self.slash_filtered.clear();
        }
        self.dirty = true;
    }

    /// Accept the currently selected slash command completion.
    pub(super) fn accept_slash_completion(&mut self) {
        let selected = self.slash_complete_state.selected().unwrap_or(0);
        if let Some(&cmd_idx) = self.slash_filtered.get(selected) {
            if let Some(cmd) = self.slash_commands.get(cmd_idx) {
                let replacement = format!("/{} ", cmd.name);
                let buf = self.input_buffer.as_str();
                let cursor = self.input_buffer.cursor();
                let line_start = buf[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
                self.input_buffer
                    .replace_range(line_start, cursor, &replacement);
            }
        }
        self.slash_complete_active = false;
        self.slash_filtered.clear();
        self.dirty = true;
    }

    /// Send prompt to the selected session
    pub(super) fn send_prompt(&mut self) -> Result<()> {
        let text = self.input_buffer.as_str().trim().to_string();
        if text.is_empty() {
            self.toast = Some(Toast::error("Empty prompt".to_string()));
            self.dirty = true;
            return Ok(());
        }

        if let Some(i) = self.list_state.selected() {
            if let Some(session) = self.sessions.get(i) {
                let pane_id = session.pane.pane_id;
                let target_workspace = session.pane.workspace.clone();
                let switching_workspace = target_workspace != self.current_workspace;

                // Send text to pane
                match WeztermCli::send_text(pane_id, &text) {
                    Ok(()) => {
                        // Switch workspace if needed
                        if switching_workspace {
                            let _ = switch_workspace(&target_workspace);
                        }

                        // Activate pane
                        let _ = WeztermCli::activate_pane(pane_id);

                        self.toast = Some(Toast::success(format!("Sent to Pane {}", pane_id)));
                    }
                    Err(e) => {
                        self.toast = Some(Toast::error(format!("Failed: {}", e)));
                    }
                }
            }
        }

        self.exit_input_mode();
        Ok(())
    }

    /// Check if the given pane_id is the pane running wzcc itself
    pub(super) fn is_self_pane(pane_id: u32) -> bool {
        std::env::var("WEZTERM_PANE")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .is_some_and(|id| id == pane_id)
    }

    /// Enter kill confirmation mode for the selected session
    pub(super) fn request_kill_selected(&mut self) {
        if let Some(i) = self.list_state.selected() {
            if let Some(session) = self.sessions.get(i) {
                let pane_id = session.pane.pane_id;

                if Self::is_self_pane(pane_id) {
                    self.toast = Some(Toast::error(
                        "Cannot kill the pane running wzcc".to_string(),
                    ));
                    self.dirty = true;
                    return;
                }

                let label = format!("Pane {}", pane_id,);
                self.kill_confirm = Some((pane_id, label));
                self.dirty = true;
            }
        }
    }

    /// Execute the kill after confirmation
    pub(super) fn confirm_kill(&mut self) -> Result<()> {
        if let Some((pane_id, _label)) = self.kill_confirm.take() {
            match WeztermCli::kill_pane(pane_id) {
                Ok(()) => {
                    self.toast = Some(Toast::success(format!("Killed Pane {}", pane_id)));
                    self.refresh()?;
                    self.update_watched_dirs()?;
                }
                Err(e) => {
                    self.toast = Some(Toast::error(format!("Failed to kill pane: {}", e)));
                }
            }
            self.dirty = true;
            self.needs_full_redraw = true;
        }
        Ok(())
    }

    /// Cancel the kill confirmation
    pub(super) fn cancel_kill(&mut self) {
        self.kill_confirm = None;
        self.dirty = true;
    }

    /// Enter add-pane mode: show direction selection prompt
    pub(super) fn request_add_pane(&mut self) {
        if let Some(i) = self.list_state.selected() {
            if let Some(session) = self.sessions.get(i) {
                let pane_id = session.pane.pane_id;
                let cwd = match session.pane.cwd_path() {
                    Some(c) => c.to_string(),
                    None => {
                        self.toast = Some(Toast::error(
                            "No working directory available for selected session".to_string(),
                        ));
                        self.dirty = true;
                        return;
                    }
                };
                let window_id = session.pane.window_id;
                self.add_pane_pending = Some((pane_id, cwd, window_id));
                self.dirty = true;
            }
        }
    }

    /// After direction is selected, either spawn immediately (single command)
    /// or enter command selection mode (multiple commands).
    pub(super) fn select_command_or_spawn(&mut self, direction: SplitDirection) -> Result<()> {
        // Must .take() to clear the direction-selection state so the
        // add_pane_pending key handler no longer intercepts events.
        if let Some((pane_id, cwd, window_id)) = self.add_pane_pending.take() {
            if self.resolved_commands.len() <= 1 {
                // Single command (or default): spawn immediately
                let cmd = &self.resolved_commands[0];
                let (prog, args) = Config::program_and_args(cmd);
                let result = match direction {
                    SplitDirection::Tab => WeztermCli::spawn_tab(&cwd, window_id, prog, args),
                    SplitDirection::Right => {
                        WeztermCli::split_pane(pane_id, &cwd, prog, args, "--right")
                    }
                    SplitDirection::Bottom => {
                        WeztermCli::split_pane(pane_id, &cwd, prog, args, "--bottom")
                    }
                };
                match result {
                    Ok(new_pane_id) => {
                        self.toast = Some(Toast::success(format!("Added Pane {}", new_pane_id)));
                        self.refresh()?;
                        self.update_watched_dirs()?;
                    }
                    Err(e) => {
                        self.toast = Some(Toast::error(format!("Failed to add pane: {}", e)));
                    }
                }
                self.dirty = true;
                self.needs_full_redraw = true;
            } else {
                // Multiple commands: enter command selection mode
                self.command_select_pending = Some(AddPaneContext {
                    pane_id,
                    cwd,
                    window_id,
                    direction,
                });
                self.command_select_state.select(Some(0));
                self.dirty = true;
            }
        }
        Ok(())
    }

    /// Execute spawn with the selected command from the command selector.
    pub(super) fn confirm_command_select(&mut self, index: usize) -> Result<()> {
        if let Some(ctx) = self.command_select_pending.take() {
            if let Some(cmd) = self.resolved_commands.get(index) {
                let (prog, args) = Config::program_and_args(cmd);
                let result = match ctx.direction {
                    SplitDirection::Tab => {
                        WeztermCli::spawn_tab(&ctx.cwd, ctx.window_id, prog, args)
                    }
                    SplitDirection::Right => {
                        WeztermCli::split_pane(ctx.pane_id, &ctx.cwd, prog, args, "--right")
                    }
                    SplitDirection::Bottom => {
                        WeztermCli::split_pane(ctx.pane_id, &ctx.cwd, prog, args, "--bottom")
                    }
                };
                match result {
                    Ok(new_pane_id) => {
                        self.toast = Some(Toast::success(format!("Added Pane {}", new_pane_id)));
                        self.refresh()?;
                        self.update_watched_dirs()?;
                    }
                    Err(e) => {
                        self.toast = Some(Toast::error(format!("Failed to add pane: {}", e)));
                    }
                }
            }
            self.dirty = true;
            self.needs_full_redraw = true;
        }
        self.command_select_state.select(None);
        Ok(())
    }

    /// Cancel the command selection mode
    pub(super) fn cancel_command_select(&mut self) {
        self.command_select_pending = None;
        self.command_select_state.select(None);
        self.dirty = true;
    }

    /// Cancel the add-pane mode
    pub(super) fn cancel_add_pane(&mut self) {
        self.add_pane_pending = None;
        self.dirty = true;
    }

    /// Copy text to system clipboard.
    /// Uses pbcopy on macOS, xclip or xsel on Linux.
    pub(super) fn copy_to_clipboard(text: &str) -> Result<()> {
        /// Pipe `text` into the given command and verify it exits successfully.
        fn run_clipboard_cmd(cmd: &str, args: &[&str], text: &str) -> Result<()> {
            let mut child = Command::new(cmd)
                .args(args)
                .stdin(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()?;
            if let Some(stdin) = child.stdin.as_mut() {
                use std::io::Write;
                stdin.write_all(text.as_bytes())?;
            }
            let output = child.wait_with_output()?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("{} failed: {}", cmd, stderr.trim());
            }
            Ok(())
        }

        #[cfg(target_os = "macos")]
        {
            return run_clipboard_cmd("pbcopy", &[], text);
        }
        #[cfg(target_os = "linux")]
        {
            // Try xclip first, fall back to xsel on spawn failure or non-zero exit
            let xclip_result = run_clipboard_cmd("xclip", &["-selection", "clipboard"], text);
            if xclip_result.is_ok() {
                return xclip_result;
            }
            return run_clipboard_cmd("xsel", &["--clipboard", "--input"], text);
        }
        #[allow(unreachable_code)]
        Err(anyhow::anyhow!("Clipboard not supported on this platform"))
    }

    /// Yank (copy) the selected session's last output to clipboard.
    pub(super) fn yank_selected_output(&mut self) {
        if let Some(i) = self.list_state.selected() {
            if let Some(session) = self.sessions.get(i) {
                if let Some(output) = &session.last_output {
                    match Self::copy_to_clipboard(output) {
                        Ok(()) => {
                            self.toast = Some(Toast::success("Copied output".to_string()));
                        }
                        Err(e) => {
                            self.toast = Some(Toast::error(format!("Copy failed: {}", e)));
                        }
                    }
                } else {
                    self.toast = Some(Toast::error("No output to copy".to_string()));
                }
                self.dirty = true;
            }
        }
    }

    /// Yank (copy) a history turn's assistant response to clipboard.
    pub(super) fn yank_history_output(&mut self, index: usize) {
        if let Some(turn) = self.history_turns.get(index) {
            if turn.assistant_response.is_empty() {
                self.toast = Some(Toast::error("No response to copy".to_string()));
            } else {
                match Self::copy_to_clipboard(&turn.assistant_response) {
                    Ok(()) => {
                        self.toast = Some(Toast::success("Copied response".to_string()));
                    }
                    Err(e) => {
                        self.toast = Some(Toast::error(format!("Copy failed: {}", e)));
                    }
                }
            }
            self.dirty = true;
        }
    }

    /// Enter history list view for the selected session
    pub(super) fn enter_history_mode(&mut self) {
        // Exit live pane view if active
        if self.detail_mode == DetailMode::LivePane {
            self.exit_live_pane_view();
        }
        if let Some(i) = self.list_state.selected() {
            if let Some(session) = self.sessions.get(i) {
                if let Some(path) = &session.transcript_path {
                    match crate::transcript::extract_conversation_turns(path, 50) {
                        Ok(turns) if !turns.is_empty() => {
                            // Pre-parse timestamps once to avoid per-frame parsing
                            self.history_timestamps = turns
                                .iter()
                                .map(|t| {
                                    t.timestamp.as_ref().and_then(|ts| {
                                        chrono::DateTime::parse_from_rfc3339(ts)
                                            .ok()
                                            .map(|dt| dt.into())
                                    })
                                })
                                .collect();
                            self.history_turns = turns;
                            self.history_list_state.select(Some(0));
                            self.history_index = 0;
                            self.history_scroll_offset = 0;
                            self.detail_mode = DetailMode::HistoryList;
                            self.pending_g = false;
                            self.dirty = true;
                            self.needs_full_redraw = true;
                        }
                        Ok(_) => {
                            self.toast = Some(Toast::error("No conversation history".to_string()));
                            self.dirty = true;
                        }
                        Err(_) => {
                            self.toast = Some(Toast::error("Failed to read history".to_string()));
                            self.dirty = true;
                        }
                    }
                } else {
                    self.toast = Some(Toast::error("No transcript available".to_string()));
                    self.dirty = true;
                }
            }
        }
    }

    /// Exit history mode entirely (back to normal)
    pub(super) fn exit_history_mode(&mut self) {
        self.detail_mode = DetailMode::Summary;
        self.summary_scroll_offset = 0;
        self.history_turns.clear();
        self.history_list_state.select(None);
        self.history_index = 0;
        self.history_scroll_offset = 0;
        self.history_timestamps.clear();
        self.pending_g = false;
        self.dirty = true;
        self.needs_full_redraw = true;
    }

    /// Enter history detail view from list
    pub(super) fn enter_history_detail(&mut self) {
        if let Some(i) = self.history_list_state.selected() {
            if i < self.history_turns.len() {
                self.history_index = i;
                self.history_scroll_offset = 0;
                self.detail_mode = DetailMode::HistoryDetail;
                self.pending_g = false;
                self.dirty = true;
                self.needs_full_redraw = true;
            }
        }
    }

    /// Return from detail view to list view
    pub(super) fn exit_history_detail(&mut self) {
        self.detail_mode = DetailMode::HistoryList;
        self.history_list_state.select(Some(self.history_index));
        self.history_scroll_offset = 0;
        self.pending_g = false;
        self.dirty = true;
        self.needs_full_redraw = true;
    }

    /// Enter live pane view for the selected session.
    pub(super) fn enter_live_pane_view(&mut self) {
        if let Some(i) = self.list_state.selected() {
            if self.sessions.get(i).is_some() {
                // Exit history mode if active
                if self.detail_mode != DetailMode::Summary {
                    self.exit_history_mode();
                }
                self.detail_mode = DetailMode::LivePane;
                self.live_pane_bytes = None;
                // Start at the bottom so the most recent output is visible.
                // The render function clamps this to max_scroll.
                self.live_pane_scroll_offset = usize::MAX;
                self.live_pane_follow_tail = true;
                self.cached_live_pane_lines = None;
                self.live_pane_poll_failures = 0;
                // Force immediate first fetch
                self.last_live_pane_fetch =
                    Instant::now() - Duration::from_secs(LIVE_PANE_COOLDOWN_SECS + 1);
                self.pending_g = false;
                self.dirty = true;
                self.needs_full_redraw = true;
            }
        }
    }

    /// Exit live pane view back to normal detail view.
    /// No-op if not currently in live pane view.
    pub(super) fn exit_live_pane_view(&mut self) {
        if self.detail_mode != DetailMode::LivePane {
            return;
        }
        self.detail_mode = DetailMode::Summary;
        self.summary_scroll_offset = 0;
        self.live_pane_bytes = None;
        self.live_pane_bytes_hash = 0;
        self.live_pane_scroll_offset = 0;
        self.live_pane_follow_tail = true;
        self.cached_live_pane_lines = None;
        self.live_pane_poll_failures = 0;
        self.pending_g = false;
        self.dirty = true;
        self.needs_full_redraw = true;
    }

    /// Fetch live pane content from wezterm if enough time has elapsed.
    pub(super) fn poll_live_pane_content(&mut self) {
        if self.detail_mode != DetailMode::LivePane {
            return;
        }

        let elapsed = self.last_live_pane_fetch.elapsed();

        // Circuit-breaker: skip polling after failures
        if self.live_pane_poll_failures > 0
            && elapsed < Duration::from_secs(LIVE_PANE_COOLDOWN_SECS)
        {
            return;
        }

        if elapsed < Duration::from_millis(LIVE_PANE_POLL_MS) {
            return;
        }

        self.last_live_pane_fetch = Instant::now();

        if let Some(i) = self.list_state.selected() {
            if let Some(session) = self.sessions.get(i) {
                let pane_id = session.pane.pane_id;
                match WeztermCli::get_text(pane_id, true) {
                    Ok(bytes) => {
                        self.live_pane_poll_failures = 0;
                        // Use hash-based change detection to avoid O(n) byte
                        // comparison on every poll (scrollback can be large).
                        let new_hash = {
                            use std::hash::{Hash, Hasher};
                            let mut h = std::hash::DefaultHasher::new();
                            bytes.hash(&mut h);
                            h.finish()
                        };
                        let first_fetch = self.live_pane_bytes.is_none();
                        let changed = new_hash != self.live_pane_bytes_hash || first_fetch;
                        if changed {
                            self.live_pane_bytes_hash = new_hash;
                            self.live_pane_bytes = Some(bytes);
                            self.cached_live_pane_lines = None; // Invalidate render cache
                            if self.live_pane_follow_tail {
                                // Keep viewport pinned to the bottom so new
                                // output is always visible; render clamps to
                                // max_scroll.
                                self.live_pane_scroll_offset = usize::MAX;
                            }
                            self.dirty = true;
                        }
                    }
                    Err(_) => {
                        self.live_pane_poll_failures += 1;
                        self.dirty = true; // Show warning in render
                        if self.live_pane_poll_failures >= LIVE_PANE_MAX_FAILURES {
                            self.toast =
                                Some(Toast::error("Pane unavailable, exiting live view".into()));
                            self.exit_live_pane_view();
                        }
                    }
                }
            }
        }
    }

    /// Compute half-page scroll amount based on terminal height.
    pub(super) fn viewport_half_height(&self) -> usize {
        // terminal height minus header(1) + footer(1) + borders(2) + session header(2)
        let (_, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let usable = rows.saturating_sub(6) as usize;
        (usable / 2).max(1)
    }

    /// Yank (copy) the live pane content to clipboard (plain text, no ANSI).
    pub(super) fn yank_live_pane_content(&mut self) {
        if let Some(i) = self.list_state.selected() {
            if let Some(session) = self.sessions.get(i) {
                let pane_id = session.pane.pane_id;
                match WeztermCli::get_text_plain(pane_id) {
                    Ok(text) => match Self::copy_to_clipboard(&text) {
                        Ok(()) => {
                            self.toast = Some(Toast::success("Copied pane output".into()));
                        }
                        Err(e) => {
                            self.toast = Some(Toast::error(format!("Copy failed: {}", e)));
                        }
                    },
                    Err(e) => {
                        self.toast = Some(Toast::error(format!("Failed to get text: {}", e)));
                    }
                }
                self.dirty = true;
            }
        }
    }

    /// Open answer selection popup for the selected WaitingForUser session.
    pub(super) fn open_answer_select(&mut self) {
        use crate::transcript::WaitingPrompt;

        let Some(i) = self.list_state.selected() else {
            return;
        };
        let Some(session) = self.sessions.get(i) else {
            return;
        };

        let Some(ref waiting) = session.waiting_prompt else {
            // waiting_prompt parse failed but session is WaitingForUser — fall back to pane jump
            if matches!(
                session.status,
                crate::transcript::SessionStatus::WaitingForUser { .. }
            ) {
                let _ = self.jump_to_selected();
                return;
            }
            self.toast = Some(Toast::error("Session is not waiting for input".into()));
            self.dirty = true;
            return;
        };

        let pane_id = session.pane.pane_id;

        match waiting {
            WaitingPrompt::PlanApproval { .. } => {
                self.answer_select_pending = Some(AnswerSelectState {
                    pane_id,
                    title: "Plan Approval (ExitPlanMode)".into(),
                    prompt_kind: AnswerPromptKind::PlanApproval,
                    options: vec![
                        AnswerOption {
                            label: "Yes, clear & bypass".into(),
                            description: Some("Clear context and bypass permissions".into()),
                            keystroke: "1".into(),
                            enter_input_after: false,
                        },
                        AnswerOption {
                            label: "Yes, bypass".into(),
                            description: Some("Bypass permissions".into()),
                            keystroke: "2".into(),
                            enter_input_after: false,
                        },
                        AnswerOption {
                            label: "Yes, approve edits".into(),
                            description: Some("Manually approve edits".into()),
                            keystroke: "3".into(),
                            enter_input_after: false,
                        },
                        AnswerOption {
                            label: "Tell Claude".into(),
                            description: Some("Type feedback for Claude".into()),
                            keystroke: "4".into(),
                            enter_input_after: true,
                        },
                    ],
                });
                self.answer_select_state.select(Some(0));
                self.dirty = true;
            }
            WaitingPrompt::Ask(ask_input) => {
                let Some(question) = ask_input.questions.first() else {
                    // Empty questions — fall back to pane jump
                    let _ = self.jump_to_selected();
                    return;
                };

                if question.multi_select {
                    // Multi-select not supported via single keystroke — jump to pane
                    let _ = self.jump_to_selected();
                    return;
                }

                let options: Vec<AnswerOption> = question
                    .options
                    .iter()
                    .enumerate()
                    .map(|(i, opt)| AnswerOption {
                        label: opt.label.clone(),
                        description: opt.description.clone(),
                        keystroke: (i + 1).to_string(),
                        enter_input_after: false,
                    })
                    .collect();

                if options.is_empty() {
                    let _ = self.jump_to_selected();
                    return;
                }

                self.answer_select_pending = Some(AnswerSelectState {
                    pane_id,
                    title: question.question.clone(),
                    prompt_kind: AnswerPromptKind::Ask,
                    options,
                });
                self.answer_select_state.select(Some(0));
                self.dirty = true;
            }
            WaitingPrompt::ToolPermission { tool_names } => {
                let title = format!("Approve: {}", tool_names.join(", "));
                self.answer_select_pending = Some(AnswerSelectState {
                    pane_id,
                    title,
                    prompt_kind: AnswerPromptKind::ToolPermission,
                    options: vec![
                        AnswerOption {
                            label: "Allow".into(),
                            description: Some("Allow this tool call".into()),
                            keystroke: "1".into(),
                            enter_input_after: false,
                        },
                        AnswerOption {
                            label: "Always allow".into(),
                            description: Some("Allow all future calls of this tool".into()),
                            keystroke: "2".into(),
                            enter_input_after: false,
                        },
                        AnswerOption {
                            label: "Reject".into(),
                            description: Some("Deny this tool call".into()),
                            keystroke: "3".into(),
                            enter_input_after: false,
                        },
                    ],
                });
                self.answer_select_state.select(Some(0));
                self.dirty = true;
            }
        }
    }

    /// Confirm the selected answer and send the keystroke to the pane.
    pub(super) fn confirm_answer_select(&mut self, index: usize) {
        use crate::transcript::WaitingPrompt;
        if let Some(state) = self.answer_select_pending.take() {
            if let Some(option) = state.options.get(index) {
                // Re-check that the session is still waiting and prompt type matches
                let session_idx = self
                    .sessions
                    .iter()
                    .position(|s| s.pane.pane_id == state.pane_id);
                let current_session = session_idx.and_then(|i| self.sessions.get(i));
                let still_waiting = current_session.is_some_and(|s| {
                    matches!(
                        s.status,
                        crate::transcript::SessionStatus::WaitingForUser { .. }
                    )
                });
                let prompt_matches = current_session.is_some_and(|s| {
                    matches!(
                        (&s.waiting_prompt, state.prompt_kind),
                        (
                            Some(WaitingPrompt::PlanApproval { .. }),
                            AnswerPromptKind::PlanApproval
                        ) | (Some(WaitingPrompt::Ask(_)), AnswerPromptKind::Ask)
                            | (
                                Some(WaitingPrompt::ToolPermission { .. }),
                                AnswerPromptKind::ToolPermission,
                            )
                    )
                });

                if !still_waiting {
                    self.toast = Some(Toast::error(
                        "Session is no longer waiting for input".into(),
                    ));
                } else if !prompt_matches {
                    self.toast = Some(Toast::error("Prompt type changed — please retry".into()));
                } else {
                    match WeztermCli::send_keystroke(state.pane_id, &option.keystroke) {
                        Ok(()) => {
                            if option.enter_input_after {
                                // Enter i-mode so user can type follow-up text
                                self.enter_input_mode();
                            } else {
                                self.toast = Some(Toast::success(format!(
                                    "Sent '{}' to Pane {}",
                                    option.label, state.pane_id
                                )));
                            }
                        }
                        Err(e) => {
                            self.toast = Some(Toast::error(format!("Failed to send: {}", e)));
                        }
                    }
                }
            }
            self.answer_select_state.select(None);
            self.dirty = true;
            self.needs_full_redraw = true;
        }
    }

    /// Cancel the answer selection popup.
    pub(super) fn cancel_answer_select(&mut self) {
        self.answer_select_pending = None;
        self.answer_select_state.select(None);
        self.dirty = true;
    }

    // --- Embedded Terminal Actions ---

    /// Enter embedded terminal mode: spawn claude in a PTY.
    pub(super) fn enter_terminal_mode(&mut self) {
        use crate::pty::PtyHandle;
        use crate::ui::terminal_session::TerminalSession;
        use std::path::PathBuf;

        // Get selected session's CWD (or fallback to home dir)
        let cwd = self
            .list_state
            .selected()
            .and_then(|i| self.sessions.get(i))
            .and_then(|s| s.pane.cwd_path())
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));

        let title = cwd
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Terminal".to_string());

        // Clear all transient UI state (state invariant)
        self.input_mode = false;
        self.input_buffer.clear();
        self.kill_confirm = None;
        self.add_pane_pending = None;
        self.command_select_pending = None;
        self.answer_select_pending = None;
        self.slash_complete_active = false;
        self.slash_filtered.clear();

        // Default size (will be updated on first render via viewport resize detection)
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        // Approximate: use 60% width for terminal, subtract borders
        let term_cols = (cols as f32 * 0.6).max(20.0) as u16 - 2;
        let term_rows = rows.saturating_sub(4); // header + footer + borders

        match PtyHandle::spawn("claude", &cwd, term_cols, term_rows) {
            Ok(pty_handle) => {
                let parser = vt100::Parser::new(term_rows, term_cols, 1000);
                self.terminal_session = Some(TerminalSession {
                    cwd,
                    title,
                    pty_handle,
                    vt100_parser: parser,
                    viewport_rect: None,
                });
                self.detail_mode = DetailMode::Terminal;
                self.focus_pane = FocusPane::Terminal;
                self.dirty = true;
                self.needs_full_redraw = true;
            }
            Err(e) => {
                self.toast = Some(Toast::error(format!("Failed to spawn terminal: {}", e)));
                self.dirty = true;
            }
        }
    }

    /// Exit embedded terminal mode: kill PTY and return to Summary.
    pub(super) fn exit_terminal_mode(&mut self) {
        self.terminal_session = None; // Drop sends shutdown signal
        self.detail_mode = DetailMode::Summary;
        self.focus_pane = FocusPane::Sidebar;
        self.summary_scroll_offset = 0;
        self.dirty = true;
        self.needs_full_redraw = true;
    }

    /// Toggle focus between sidebar and embedded terminal.
    pub(super) fn toggle_terminal_focus(&mut self) {
        if self.terminal_session.is_some() {
            self.focus_pane = match self.focus_pane {
                FocusPane::Sidebar => FocusPane::Terminal,
                FocusPane::Terminal => FocusPane::Sidebar,
            };
            self.dirty = true;
        }
    }

    /// Convert a crossterm KeyEvent to terminal byte sequence and write to PTY.
    pub(super) fn send_key_to_pty(&mut self, key: &crossterm::event::KeyEvent) {
        let bytes = key_event_to_bytes(key);
        if !bytes.is_empty() {
            if let Some(ref mut ts) = self.terminal_session {
                let _ = ts.pty_handle.write(&bytes);
            }
        }
    }

    /// Send pasted text to PTY wrapped in bracketed paste markers.
    pub(super) fn send_paste_to_pty(&mut self, text: &str) {
        if let Some(ref mut ts) = self.terminal_session {
            let mut buf = Vec::new();
            buf.extend_from_slice(b"\x1b[200~"); // begin bracketed paste
            buf.extend_from_slice(text.as_bytes());
            buf.extend_from_slice(b"\x1b[201~"); // end bracketed paste
            let _ = ts.pty_handle.write(&buf);
        }
    }

    /// Poll PTY output and feed to vt100 parser. Returns true if any output received.
    pub(super) fn poll_pty_output(&mut self) -> bool {
        use crate::pty::PtyEvent;

        let Some(ref mut ts) = self.terminal_session else {
            return false;
        };

        let events = ts.pty_handle.try_recv();
        if events.is_empty() {
            return false;
        }

        let mut got_output = false;
        let mut exited = false;

        for event in events {
            match event {
                PtyEvent::Output(bytes) => {
                    ts.vt100_parser.process(&bytes);
                    got_output = true;
                }
                PtyEvent::Exited => {
                    exited = true;
                }
            }
        }

        if exited {
            self.toast = Some(Toast::success("Terminal process exited".into()));
            self.exit_terminal_mode();
            return true;
        }

        if got_output {
            self.dirty = true;
        }

        got_output
    }

    /// Check and handle viewport resize for embedded terminal.
    pub(super) fn check_terminal_resize(&mut self, current_area: Rect) {
        let Some(ref mut ts) = self.terminal_session else {
            return;
        };

        // Account for block borders (1 on each side)
        let inner_cols = current_area.width.saturating_sub(2);
        let inner_rows = current_area.height.saturating_sub(2);

        if inner_cols == 0 || inner_rows == 0 {
            return;
        }

        let needs_resize = ts
            .viewport_rect
            .map(|prev| {
                prev.width.saturating_sub(2) != inner_cols
                    || prev.height.saturating_sub(2) != inner_rows
            })
            .unwrap_or(true);

        if needs_resize {
            ts.viewport_rect = Some(current_area);
            ts.vt100_parser.set_size(inner_rows, inner_cols);
            let _ = ts.pty_handle.resize(inner_cols, inner_rows);
        }
    }
}

/// Convert a crossterm KeyEvent into terminal byte sequences.
fn key_event_to_bytes(key: &crossterm::event::KeyEvent) -> Vec<u8> {
    use crossterm::event::{KeyCode, KeyModifiers};

    match key.code {
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Ctrl+A = 0x01, Ctrl+Z = 0x1a, etc.
            if c.is_ascii_alphabetic() {
                vec![(c.to_ascii_lowercase() as u8) & 0x1f]
            } else {
                vec![]
            }
        }
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::ALT) => {
            // Alt+key = ESC + key
            let mut buf = vec![0x1b];
            let mut utf8 = [0u8; 4];
            let s = c.encode_utf8(&mut utf8);
            buf.extend_from_slice(s.as_bytes());
            buf
        }
        KeyCode::Char(c) => {
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            s.as_bytes().to_vec()
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => vec![0x1b, b'[', b'Z'], // Shift+Tab
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => b"\x1b[15~".to_vec(),
            6 => b"\x1b[17~".to_vec(),
            7 => b"\x1b[18~".to_vec(),
            8 => b"\x1b[19~".to_vec(),
            9 => b"\x1b[20~".to_vec(),
            10 => b"\x1b[21~".to_vec(),
            11 => b"\x1b[23~".to_vec(),
            12 => b"\x1b[24~".to_vec(),
            _ => vec![],
        },
        _ => vec![],
    }
}
