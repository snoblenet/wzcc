//! Combined transcript reading: reads a transcript file once and extracts
//! status, last user prompt, and last assistant text in a single pass.
//!
//! This module acts as an orchestration layer between `parser` (file reading
//! and data extraction) and `state` (status detection logic), avoiding a
//! direct dependency from parser to state.

use super::parser::{
    extract_last_assistant_text, extract_last_user_prompt, AskUserQuestionInput, TranscriptSnapshot,
};
use super::state::{detect_session_status_from_entries, SessionStatus};
use anyhow::Result;
use std::path::Path;

/// Normalized prompt data for a WaitingForUser session.
#[derive(Debug, Clone)]
pub enum WaitingPrompt {
    /// AskUserQuestion with parsed questions and options.
    Ask(AskUserQuestionInput),
    /// Tool permission request (Bash, Edit, etc.).
    ToolPermission { tool_names: Vec<String> },
    /// ExitPlanMode — user should jump to pane. Carries plan text from input.
    PlanApproval { plan: String },
}

/// Result of reading all transcript information in a single file read.
pub struct TranscriptInfo {
    pub status: SessionStatus,
    pub last_prompt: Option<String>,
    pub last_output: Option<String>,
    pub waiting_prompt: Option<WaitingPrompt>,
}

/// Read a transcript file once and extract status, last user prompt, and
/// last assistant text. This replaces three separate file reads with one.
pub fn read_transcript_info(path: &Path) -> Result<TranscriptInfo> {
    let snapshot = TranscriptSnapshot::from_path(path)?;
    let entries = snapshot.last_entries(10);
    let status = detect_session_status_from_entries(&entries);
    let last_prompt = extract_last_user_prompt(&snapshot, 200);
    let last_output = extract_last_assistant_text(&snapshot, 1000);

    let waiting_prompt = if matches!(status, SessionStatus::WaitingForUser { .. }) {
        extract_waiting_prompt(&entries)
    } else {
        None
    };

    Ok(TranscriptInfo {
        status,
        last_prompt,
        last_output,
        waiting_prompt,
    })
}

/// Extract waiting prompt data from the last tool_use entry.
fn extract_waiting_prompt(entries: &[super::parser::TranscriptEntry]) -> Option<WaitingPrompt> {
    // Find the last tool_use entry (scanning from end)
    let tool_entry = entries.iter().rev().find(|e| e.is_tool_use())?;
    let names = tool_entry.get_tool_names();

    if names.iter().any(|n| n == "AskUserQuestion") {
        // Try to parse the AskUserQuestion input from the tool_use content block
        let ask_input = tool_entry
            .message
            .as_ref()
            .and_then(|m| {
                m.content
                    .iter()
                    .find(|c| c.type_ == "tool_use" && c.name.as_deref() == Some("AskUserQuestion"))
            })
            .and_then(|c| c.parse_ask_input());

        match ask_input {
            Some(input) if !input.questions.is_empty() => Some(WaitingPrompt::Ask(input)),
            _ => None, // Parse failed or empty → caller falls back to pane jump
        }
    } else if names.iter().any(|n| n == "ExitPlanMode") {
        let plan_text = tool_entry
            .message
            .as_ref()
            .and_then(|m| {
                m.content
                    .iter()
                    .find(|c| c.type_ == "tool_use" && c.name.as_deref() == Some("ExitPlanMode"))
            })
            .and_then(|c| c.parse_plan_text())
            .unwrap_or_default();
        Some(WaitingPrompt::PlanApproval { plan: plan_text })
    } else {
        Some(WaitingPrompt::ToolPermission { tool_names: names })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_transcript(entries: &[&str]) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        for entry in entries {
            writeln!(file, "{}", entry).unwrap();
        }
        file.flush().unwrap();
        file
    }

    #[test]
    fn test_read_transcript_info_empty_file() {
        let file = NamedTempFile::new().unwrap();
        let info = read_transcript_info(file.path()).unwrap();
        assert_eq!(info.status, SessionStatus::Unknown);
        assert!(info.last_prompt.is_none());
        assert!(info.last_output.is_none());
    }

    #[test]
    fn test_read_transcript_info_idle_with_prompt_and_output() {
        let file = create_transcript(&[
            r#"{"type":"user","timestamp":"2026-01-23T16:29:00.000Z","message":{"content":"Hello Claude"}}"#,
            r#"{"type":"assistant","timestamp":"2026-01-23T16:29:01.000Z","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"Hi! How can I help?"}]}}"#,
            r#"{"type":"system","subtype":"turn_duration","timestamp":"2026-01-23T16:29:02.000Z"}"#,
        ]);
        let info = read_transcript_info(file.path()).unwrap();
        assert_eq!(info.status, SessionStatus::Idle);
        assert_eq!(info.last_prompt.as_deref(), Some("Hello Claude"));
        assert_eq!(info.last_output.as_deref(), Some("Hi! How can I help?"));
    }

    #[test]
    fn test_read_transcript_info_processing() {
        let file = create_transcript(&[
            r#"{"type":"user","timestamp":"2026-01-23T16:29:00.000Z","message":{"content":"Do something"}}"#,
            r#"{"type":"progress","timestamp":"2026-01-23T16:29:01.000Z"}"#,
        ]);
        let info = read_transcript_info(file.path()).unwrap();
        assert_eq!(info.status, SessionStatus::Processing);
        assert_eq!(info.last_prompt.as_deref(), Some("Do something"));
        assert!(info.last_output.is_none());
    }

    #[test]
    fn test_read_transcript_info_matches_individual_functions() {
        // Verify that read_transcript_info produces the same results as
        // calling the three functions individually.
        use crate::transcript::{
            detect_session_status, get_last_assistant_text, get_last_user_prompt,
        };

        let file = create_transcript(&[
            r#"{"type":"user","timestamp":"2026-01-23T16:29:00.000Z","message":{"content":"Explain closures"}}"#,
            r#"{"type":"assistant","timestamp":"2026-01-23T16:29:01.000Z","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"A closure captures variables from its environment."}]}}"#,
            r#"{"type":"system","subtype":"stop_hook_summary","timestamp":"2026-01-23T16:29:02.000Z"}"#,
        ]);

        let info = read_transcript_info(file.path()).unwrap();
        let individual_status = detect_session_status(file.path()).unwrap();
        let individual_prompt = get_last_user_prompt(file.path(), 200).unwrap();
        let individual_output = get_last_assistant_text(file.path(), 1000).unwrap();

        assert_eq!(info.status, individual_status);
        assert_eq!(info.last_prompt, individual_prompt);
        assert_eq!(info.last_output, individual_output);
    }
}
