use anyhow::Result;
use crossterm::event::{
    self, Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers, MouseEvent,
};
use std::time::Duration;

/// TUI event
#[derive(Debug, Clone)]
pub enum Event {
    /// Key input
    Key(KeyEvent),
    /// Mouse input
    Mouse(MouseEvent),
    /// Bracketed paste input
    Paste(String),
    /// Tick (periodic update)
    Tick,
    /// Resize
    Resize(u16, u16),
}

/// Event handler
pub struct EventHandler {
    /// Tick interval (ms)
    tick_rate: Duration,
}

impl EventHandler {
    pub fn new(tick_rate_ms: u64) -> Self {
        Self {
            tick_rate: Duration::from_millis(tick_rate_ms),
        }
    }

    /// Get next event
    pub fn next(&self) -> Result<Event> {
        // Wait for event with timeout using crossterm poll
        if event::poll(self.tick_rate)? {
            match event::read()? {
                CrosstermEvent::Key(key) => Ok(Event::Key(key)),
                CrosstermEvent::Mouse(mouse) => Ok(Event::Mouse(mouse)),
                CrosstermEvent::Paste(text) => Ok(Event::Paste(text)),
                CrosstermEvent::Resize(w, h) => Ok(Event::Resize(w, h)),
                _ => Ok(Event::Tick),
            }
        } else {
            // Timeout -> Tick event
            Ok(Event::Tick)
        }
    }
}

/// Key event helper
pub fn is_quit_key(key: &KeyEvent) -> bool {
    matches!(
        key.code,
        KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('c')
    ) || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

pub fn is_up_key(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Up | KeyCode::Char('k'))
}

pub fn is_down_key(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Down | KeyCode::Char('j'))
}

pub fn is_enter_key(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Enter)
}

pub fn is_refresh_key(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('r'))
}
