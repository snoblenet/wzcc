use super::*;

impl App {
    pub fn select_next(&mut self) {
        if self.sessions.is_empty() {
            return;
        }

        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= self.sessions.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };

        self.list_state.select(Some(i));
        self.summary_scroll_offset = 0;
        self.exit_live_pane_view();
        self.dirty = true;
    }

    /// Select previous item
    pub fn select_previous(&mut self) {
        if self.sessions.is_empty() {
            return;
        }

        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.sessions.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };

        self.list_state.select(Some(i));
        self.summary_scroll_offset = 0;
        self.exit_live_pane_view();
        self.dirty = true;
    }

    /// Select first item (gg)
    pub fn select_first(&mut self) {
        if !self.sessions.is_empty() {
            self.list_state.select(Some(0));
            self.summary_scroll_offset = 0;
            self.exit_live_pane_view();
            self.dirty = true;
        }
    }

    /// Select last item (G)
    pub fn select_last(&mut self) {
        if !self.sessions.is_empty() {
            self.list_state.select(Some(self.sessions.len() - 1));
            self.summary_scroll_offset = 0;
            self.exit_live_pane_view();
            self.dirty = true;
        }
    }

    /// Jump to selected session
    pub fn jump_to_selected(&mut self) -> Result<()> {
        if let Some(i) = self.list_state.selected() {
            if let Some(session) = self.sessions.get(i) {
                let pane_id = session.pane.pane_id;
                let target_workspace = &session.pane.workspace;
                let switching_workspace = target_workspace != &self.current_workspace;

                // Switch workspace if needed
                if switching_workspace {
                    switch_workspace(target_workspace)?;
                }

                // Activate pane
                WeztermCli::activate_pane(pane_id)?;

                // Refresh session list after workspace switch to update ordering
                if switching_workspace {
                    // Small delay to allow WezTerm to complete workspace switch
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    self.refresh()?;
                }
            }
        }

        Ok(())
    }

    /// Calculate session index from list display row
    /// Returns the session corresponding to the clicked row, considering group headers
    pub(super) fn row_to_session_index(&self, row: usize) -> Option<usize> {
        row_to_session_index(&self.sessions, row)
    }
}
