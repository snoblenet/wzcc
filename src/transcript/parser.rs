//! JSONL transcript file parser.

use anyhow::Result;
use regex::Regex;
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

/// Remove internal XML tags (system-reminder, local-command-*, command-*) from text.
fn remove_internal_tags(text: &str) -> String {
    const TAGS: &[&str] = &[
        "system-reminder",
        "local-command-stdout",
        "local-command-caveat",
        "command-name",
        "command-message",
        "command-args",
    ];
    let mut result = text.to_string();
    for tag in TAGS {
        let re = Regex::new(&format!(r"<{0}>[\s\S]*?</{0}>", regex::escape(tag))).unwrap();
        result = re.replace_all(&result, "").to_string();
    }
    result.trim().to_string()
}

/// Truncate text to max_chars, appending "..." if truncated.
fn truncate_with_ellipsis(text: String, max_chars: usize) -> String {
    if text.chars().count() > max_chars {
        let mut s: String = text.chars().take(max_chars).collect();
        s.push_str("...");
        s
    } else {
        text
    }
}

/// Read lines from a file, optionally seeking near the end for large files.
/// Returns non-empty lines from the file.
fn read_lines_from_end(path: &Path, seek_multiplier: u64) -> Result<Vec<String>> {
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    let file_size = metadata.len();

    if file_size == 0 {
        return Ok(Vec::new());
    }

    let mut reader = BufReader::new(file);
    let mut lines = Vec::new();

    if file_size < 1024 * 1024 {
        // < 1MB: read all lines
        for line in reader.lines() {
            let line = line?;
            if !line.trim().is_empty() {
                lines.push(line);
            }
        }
    } else {
        // Large file: seek near end
        let seek_pos = file_size.saturating_sub(seek_multiplier * 100 * 1024);
        reader.seek(SeekFrom::Start(seek_pos))?;

        // Skip partial line if we seeked to middle
        if seek_pos > 0 {
            let mut _skip = String::new();
            reader.read_line(&mut _skip)?;
        }

        for line in reader.lines() {
            let line = line?;
            if !line.trim().is_empty() {
                lines.push(line);
            }
        }
    }

    Ok(lines)
}

/// An option within an AskUserQuestion question.
#[derive(Debug, Clone, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    pub description: Option<String>,
}

/// A single question in an AskUserQuestion tool call.
#[derive(Debug, Clone, Deserialize)]
pub struct Question {
    pub question: String,
    pub header: Option<String>,
    pub options: Vec<QuestionOption>,
    #[serde(rename = "multiSelect", default)]
    pub multi_select: bool,
}

/// Parsed input for the AskUserQuestion tool.
#[derive(Debug, Clone, Deserialize)]
pub struct AskUserQuestionInput {
    pub questions: Vec<Question>,
}

/// A content block within a message.
#[derive(Debug, Clone, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub type_: String,
    pub name: Option<String>,
    pub text: Option<String>,
    pub content: Option<String>,
    pub is_error: Option<bool>,
    #[serde(default)]
    pub input: Option<serde_json::Value>,
}

impl ContentBlock {
    /// Parse the `input` field as an AskUserQuestion input.
    pub fn parse_ask_input(&self) -> Option<AskUserQuestionInput> {
        self.input
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Extract plan text from ExitPlanMode input.
    pub fn parse_plan_text(&self) -> Option<String> {
        self.input
            .as_ref()
            .and_then(|v| v.get("plan"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

/// The message structure within an assistant entry.
#[derive(Debug, Clone, Deserialize)]
pub struct AssistantMessage {
    /// Distinguishes three states:
    /// - `None` — field absent from JSON (legacy/malformed entry)
    /// - `Some(None)` — field explicitly `null` (streaming in progress)
    /// - `Some(Some("end_turn"))` / `Some(Some("tool_use"))` — completed with reason
    #[serde(default, deserialize_with = "deserialize_present_field")]
    pub stop_reason: Option<Option<String>>,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
}

/// Deserializer that wraps any present JSON value in `Some()`, letting
/// `#[serde(default)]` supply `None` for missing fields. This gives us
/// `Option<Option<T>>` semantics: absent → `None`, null → `Some(None)`,
/// value → `Some(Some(v))`.
fn deserialize_present_field<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

/// Progress data for hook progress entries.
#[derive(Debug, Clone, Deserialize)]
pub struct ProgressData {
    #[serde(rename = "type")]
    pub type_: Option<String>,
}

/// A transcript entry (one line from the JSONL file).
#[derive(Debug, Clone, Deserialize)]
pub struct TranscriptEntry {
    #[serde(rename = "type")]
    pub type_: String,
    pub subtype: Option<String>,
    pub timestamp: Option<String>,
    pub message: Option<AssistantMessage>,
    pub data: Option<ProgressData>,
}

impl TranscriptEntry {
    /// Check if this entry is an assistant message with tool_use.
    /// Checks both stop_reason == "tool_use" and content containing tool_use blocks.
    pub fn is_tool_use(&self) -> bool {
        if self.type_ != "assistant" {
            return false;
        }

        let Some(msg) = &self.message else {
            return false;
        };

        // Check stop_reason (flatten Option<Option<String>> to Option<&str>)
        if msg.stop_reason.as_ref().and_then(|o| o.as_deref()) == Some("tool_use") {
            return true;
        }

        // Also check if content has tool_use blocks (even with stop_reason: null)
        // This happens when Claude is waiting for user approval
        msg.content.iter().any(|c| c.type_ == "tool_use")
    }

    /// Check if this entry is an assistant message with an end_turn stop_reason.
    pub fn is_end_turn(&self) -> bool {
        self.type_ == "assistant"
            && self
                .message
                .as_ref()
                .and_then(|m| m.stop_reason.as_ref())
                .and_then(|o| o.as_ref())
                .map(|s| s == "end_turn")
                .unwrap_or(false)
    }

    /// Check if this is a streaming assistant entry (stop_reason is explicitly null,
    /// no tool_use). Returns false when stop_reason is absent (legacy/malformed entries).
    pub fn is_streaming(&self) -> bool {
        if self.type_ != "assistant" {
            return false;
        }
        let Some(msg) = &self.message else {
            return false;
        };
        // Some(None) = explicitly null (streaming); None = field absent (not streaming)
        matches!(msg.stop_reason, Some(None)) && !msg.content.iter().any(|c| c.type_ == "tool_use")
    }

    /// Check if this is a progress entry (indicates processing).
    pub fn is_progress(&self) -> bool {
        self.type_ == "progress"
    }

    /// Check if this is a hook_progress entry (session hooks, not Claude processing).
    pub fn is_hook_progress(&self) -> bool {
        self.type_ == "progress"
            && self
                .data
                .as_ref()
                .and_then(|d| d.type_.as_deref())
                .map(|t| t == "hook_progress")
                .unwrap_or(false)
    }

    /// Check if this is a system stop_hook_summary (indicates idle).
    pub fn is_stop_hook_summary(&self) -> bool {
        self.type_ == "system" && self.subtype.as_deref() == Some("stop_hook_summary")
    }

    /// Check if this is a system turn_duration (indicates idle - turn completed).
    pub fn is_turn_duration(&self) -> bool {
        self.type_ == "system" && self.subtype.as_deref() == Some("turn_duration")
    }

    /// Check if this is a user entry with a tool_result.
    pub fn is_tool_result(&self) -> bool {
        if self.type_ != "user" {
            return false;
        }
        self.message
            .as_ref()
            .map(|m| m.content.iter().any(|c| c.type_ == "tool_result"))
            .unwrap_or(false)
    }

    /// Check if this is an interrupted user entry.
    /// Detects both:
    /// - tool_result with is_error: true containing "[Request interrupted by user"
    /// - Plain text message containing "[Request interrupted by user"
    pub fn is_interrupted(&self) -> bool {
        if self.type_ != "user" {
            return false;
        }

        let Some(msg) = &self.message else {
            return false;
        };

        for block in &msg.content {
            // Check tool_result with is_error: true
            if block.type_ == "tool_result" && block.is_error == Some(true) {
                if let Some(content) = &block.content {
                    if content.contains("[Request interrupted by user") {
                        return true;
                    }
                }
            }

            // Check plain text message
            if block.type_ == "text" {
                if let Some(text) = &block.text {
                    if text.contains("[Request interrupted by user") {
                        return true;
                    }
                }
            }
        }

        false
    }

    /// Get the tool names from a tool_use message.
    pub fn get_tool_names(&self) -> Vec<String> {
        self.message
            .as_ref()
            .map(|m| {
                m.content
                    .iter()
                    .filter(|c| c.type_ == "tool_use")
                    .filter_map(|c| c.name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Read the last N entries from a transcript file.
/// Uses reverse file reading for efficiency with large files.
pub fn read_last_entries(path: &Path, count: usize) -> Result<Vec<TranscriptEntry>> {
    let lines = read_lines_from_end(path, count as u64 + 10)?;

    if lines.is_empty() {
        return Ok(Vec::new());
    }

    // Parse the last N lines
    let mut entries: Vec<TranscriptEntry> = lines
        .iter()
        .rev()
        .take(count)
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();

    // Reverse to get chronological order
    entries.reverse();
    Ok(entries)
}

/// Pre-read raw lines from a transcript file, shared across multiple
/// extraction functions without re-reading the file.
pub struct TranscriptSnapshot {
    lines: Vec<String>,
}

impl TranscriptSnapshot {
    /// Read the tail of a transcript file once.
    /// Uses seek_multiplier=30 to cover the needs of all consumers
    /// (status detection uses 20, prompt/assistant extraction use 30).
    pub fn from_path(path: &Path) -> Result<Self> {
        let lines = read_lines_from_end(path, 30)?;
        Ok(Self { lines })
    }

    /// Return true if no lines were read (empty file).
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Parse the last `count` lines as TranscriptEntry (chronological order).
    /// Same logic as `read_last_entries`, but from in-memory lines.
    pub fn last_entries(&self, count: usize) -> Vec<TranscriptEntry> {
        if self.lines.is_empty() {
            return Vec::new();
        }

        let mut entries: Vec<TranscriptEntry> = self
            .lines
            .iter()
            .rev()
            .take(count)
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();

        entries.reverse();
        entries
    }

    /// Access raw lines for extraction that needs different deserialization
    /// (e.g., UserTranscriptEntry for user prompt extraction).
    pub fn raw_lines(&self) -> &[String] {
        &self.lines
    }
}

/// A user message structure (content can be string or array).
#[derive(Debug, Clone, Deserialize)]
pub struct UserMessage {
    #[serde(default)]
    pub content: UserContent,
}

/// User content can be a string or an array of content blocks.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(untagged)]
pub enum UserContent {
    #[default]
    Empty,
    Text(String),
    Blocks(Vec<ContentBlock>),
}

/// A user transcript entry.
#[derive(Debug, Clone, Deserialize)]
pub struct UserTranscriptEntry {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(rename = "isMeta")]
    pub is_meta: Option<bool>,
    pub timestamp: Option<String>,
    pub message: Option<UserMessage>,
}

/// Extract the last user prompt from pre-read lines.
/// Same logic as `get_last_user_prompt`, but operates on in-memory snapshot data.
pub fn extract_last_user_prompt(snapshot: &TranscriptSnapshot, max_chars: usize) -> Option<String> {
    if snapshot.is_empty() {
        return None;
    }

    // Search from the end for a user message with text content (not tool_result, not isMeta)
    for line in snapshot.raw_lines().iter().rev().take(200) {
        let entry: UserTranscriptEntry = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(_) => continue,
        };

        if entry.type_ != "user" || entry.is_meta == Some(true) {
            continue;
        }

        let Some(msg) = &entry.message else {
            continue;
        };

        let text = match &msg.content {
            UserContent::Text(s) => {
                // Skip if it's only tool_result content
                if s.contains("tool_result") && !s.contains('\n') {
                    continue;
                }
                let cleaned = remove_internal_tags(s);
                if cleaned.trim().is_empty() {
                    continue;
                }
                cleaned
            }
            UserContent::Blocks(blocks) => {
                if blocks.iter().any(|b| b.type_ == "tool_result") {
                    continue;
                }
                let raw_text = blocks
                    .iter()
                    .filter(|b| b.type_ == "text")
                    .filter_map(|b| b.text.as_ref())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n");
                let cleaned = remove_internal_tags(&raw_text);
                if cleaned.trim().is_empty() {
                    continue;
                }
                cleaned
            }
            UserContent::Empty => continue,
        };

        if !text.is_empty() {
            return Some(truncate_with_ellipsis(text, max_chars));
        }
    }

    None
}

/// Get the last user prompt from a transcript file.
/// Returns the text content (up to max_chars) from the most recent user message.
pub fn get_last_user_prompt(path: &Path, max_chars: usize) -> Result<Option<String>> {
    let snapshot = TranscriptSnapshot::from_path(path)?;
    Ok(extract_last_user_prompt(&snapshot, max_chars))
}

/// Extract the last assistant text from pre-read entries.
/// Same logic as `get_last_assistant_text`, but operates on in-memory snapshot data.
pub fn extract_last_assistant_text(
    snapshot: &TranscriptSnapshot,
    max_chars: usize,
) -> Option<String> {
    let entries = snapshot.last_entries(20);

    // Search from the end for an assistant message with text content
    for entry in entries.iter().rev() {
        if entry.type_ != "assistant" {
            continue;
        }

        let Some(msg) = &entry.message else {
            continue;
        };

        let text: String = msg
            .content
            .iter()
            .filter(|c| c.type_ == "text")
            .filter_map(|c| c.text.as_ref())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");

        if !text.is_empty() {
            return Some(truncate_with_ellipsis(text, max_chars));
        }
    }

    None
}

/// Get the last assistant text output from a transcript file.
/// Returns the text content (up to max_chars) from the most recent assistant message.
pub fn get_last_assistant_text(path: &Path, max_chars: usize) -> Result<Option<String>> {
    let snapshot = TranscriptSnapshot::from_path(path)?;
    Ok(extract_last_assistant_text(&snapshot, max_chars))
}

/// A conversation turn: a user prompt paired with the assistant's response.
#[derive(Debug, Clone)]
pub struct ConversationTurn {
    pub user_prompt: String,
    pub assistant_response: String,
    pub timestamp: Option<String>,
}

/// Extract conversation turns from a transcript file.
/// Returns turns in reverse chronological order (newest first).
/// Reads up to `max_turns` most recent turns.
pub fn extract_conversation_turns(path: &Path, max_turns: usize) -> Result<Vec<ConversationTurn>> {
    // Use larger seek_multiplier for more history coverage
    let lines = read_lines_from_end(path, 100)?;

    let mut turns: Vec<ConversationTurn> = Vec::new();
    let mut current_prompt: Option<String> = None;
    let mut current_timestamp: Option<String> = None;
    let mut last_assistant_text = String::new();

    for line in &lines {
        // Quick type check to avoid unnecessary full parsing
        #[derive(Deserialize)]
        struct TypeOnly {
            #[serde(rename = "type")]
            type_: String,
        }
        let entry_type = match serde_json::from_str::<TypeOnly>(line) {
            Ok(t) => t.type_,
            Err(_) => continue,
        };

        match entry_type.as_str() {
            "user" => {
                let entry: UserTranscriptEntry = match serde_json::from_str(line) {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                if entry.is_meta == Some(true) {
                    continue;
                }

                let Some(msg) = &entry.message else {
                    continue;
                };

                let text = match &msg.content {
                    UserContent::Text(s) => {
                        if s.contains("tool_result") && !s.contains('\n') {
                            continue;
                        }
                        let cleaned = remove_internal_tags(s);
                        if cleaned.trim().is_empty() {
                            continue;
                        }
                        cleaned
                    }
                    UserContent::Blocks(blocks) => {
                        if blocks.iter().any(|b| b.type_ == "tool_result") {
                            continue;
                        }
                        let raw = blocks
                            .iter()
                            .filter(|b| b.type_ == "text")
                            .filter_map(|b| b.text.as_ref())
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("\n");
                        let cleaned = remove_internal_tags(&raw);
                        if cleaned.trim().is_empty() {
                            continue;
                        }
                        cleaned
                    }
                    UserContent::Empty => continue,
                };

                // Save previous turn if exists
                if let Some(prev_prompt) = current_prompt.take() {
                    turns.push(ConversationTurn {
                        user_prompt: prev_prompt,
                        assistant_response: std::mem::take(&mut last_assistant_text),
                        timestamp: current_timestamp.take(),
                    });
                }

                current_prompt = Some(text);
                current_timestamp = entry.timestamp.clone();
                last_assistant_text.clear();
            }
            "assistant" => {
                let entry: TranscriptEntry = match serde_json::from_str(line) {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                if let Some(msg) = &entry.message {
                    let text: String = msg
                        .content
                        .iter()
                        .filter(|c| c.type_ == "text")
                        .filter_map(|c| c.text.as_ref())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n");

                    if !text.is_empty() {
                        // Keep only the last assistant text for this turn
                        last_assistant_text = text;
                    }
                }
            }
            _ => {}
        }
    }

    // Handle final turn
    if let Some(prompt) = current_prompt {
        turns.push(ConversationTurn {
            user_prompt: prompt,
            assistant_response: last_assistant_text,
            timestamp: current_timestamp,
        });
    }

    // Reverse to newest-first, then truncate
    turns.reverse();
    turns.truncate(max_turns);

    Ok(turns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_assistant_entry() {
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","name":"Bash"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.type_, "assistant");
        assert!(entry.is_tool_use());
        assert_eq!(entry.get_tool_names(), vec!["Bash"]);
    }

    #[test]
    fn test_parse_assistant_entry_with_null_stop_reason() {
        // This is the case when Claude is waiting for user approval
        // stop_reason is null but content has tool_use
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z","message":{"stop_reason":null,"content":[{"type":"tool_use","name":"AskUserQuestion"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.type_, "assistant");
        assert!(entry.is_tool_use());
        assert_eq!(entry.get_tool_names(), vec!["AskUserQuestion"]);
    }

    #[test]
    fn test_parse_progress_entry() {
        let json = r#"{"type":"progress","timestamp":"2026-01-23T16:29:06.719Z"}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_progress());
    }

    #[test]
    fn test_parse_system_stop_hook() {
        let json = r#"{"type":"system","subtype":"stop_hook_summary","timestamp":"2026-01-23T16:29:06.719Z"}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_stop_hook_summary());
    }

    // Additional is_tool_use tests
    #[test]
    fn test_is_tool_use_not_assistant() {
        // is_tool_use should return false for non-assistant entries
        let json = r#"{"type":"user","timestamp":"2026-01-23T16:29:06.719Z"}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_tool_use());
    }

    #[test]
    fn test_is_tool_use_no_message() {
        // Assistant without message should return false
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z"}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_tool_use());
    }

    #[test]
    fn test_is_tool_use_empty_content() {
        // Assistant with empty content and no stop_reason should return false
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z","message":{"stop_reason":null,"content":[]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_tool_use());
    }

    #[test]
    fn test_is_tool_use_text_only() {
        // Assistant with only text content should return false
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"Hello"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_tool_use());
    }

    #[test]
    fn test_is_tool_use_multiple_tools() {
        // Multiple tool_use blocks
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","name":"Read"},{"type":"tool_use","name":"Glob"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_tool_use());
        assert_eq!(entry.get_tool_names(), vec!["Read", "Glob"]);
    }

    #[test]
    fn test_is_tool_use_mixed_content() {
        // Text and tool_use mixed
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z","message":{"stop_reason":"tool_use","content":[{"type":"text","text":"Let me check"},{"type":"tool_use","name":"Read"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_tool_use());
        assert_eq!(entry.get_tool_names(), vec!["Read"]);
    }

    // is_end_turn tests
    #[test]
    fn test_is_end_turn_true() {
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z","message":{"stop_reason":"end_turn","content":[]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_end_turn());
    }

    #[test]
    fn test_is_end_turn_false_tool_use() {
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z","message":{"stop_reason":"tool_use","content":[]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_end_turn());
    }

    // is_hook_progress tests
    #[test]
    fn test_is_hook_progress_true() {
        let json = r#"{"type":"progress","timestamp":"2026-01-23T16:29:06.719Z","data":{"type":"hook_progress"}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_hook_progress());
        assert!(entry.is_progress());
    }

    #[test]
    fn test_is_hook_progress_false_no_data() {
        let json = r#"{"type":"progress","timestamp":"2026-01-23T16:29:06.719Z"}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_hook_progress());
        assert!(entry.is_progress());
    }

    #[test]
    fn test_is_hook_progress_false_different_type() {
        let json = r#"{"type":"progress","timestamp":"2026-01-23T16:29:06.719Z","data":{"type":"other_progress"}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_hook_progress());
    }

    // is_turn_duration tests
    #[test]
    fn test_is_turn_duration_true() {
        let json =
            r#"{"type":"system","subtype":"turn_duration","timestamp":"2026-01-23T16:29:06.719Z"}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_turn_duration());
    }

    #[test]
    fn test_is_turn_duration_false() {
        let json = r#"{"type":"system","subtype":"other","timestamp":"2026-01-23T16:29:06.719Z"}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_turn_duration());
    }

    // is_tool_result tests
    #[test]
    fn test_is_tool_result_true() {
        let json = r#"{"type":"user","timestamp":"2026-01-23T16:29:06.719Z","message":{"content":[{"type":"tool_result","tool_use_id":"123"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_tool_result());
    }

    #[test]
    fn test_is_tool_result_false_not_user() {
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z","message":{"content":[{"type":"tool_result"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_tool_result());
    }

    #[test]
    fn test_is_tool_result_false_no_tool_result() {
        let json = r#"{"type":"user","timestamp":"2026-01-23T16:29:06.719Z","message":{"content":[{"type":"text","text":"hello"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_tool_result());
    }

    // remove_internal_tags tests
    #[test]
    fn test_remove_internal_tags() {
        let text = "Hello <system-reminder>some reminder</system-reminder> World";
        assert_eq!(remove_internal_tags(text), "Hello  World");
    }

    #[test]
    fn test_remove_internal_tags_multiline() {
        let text = "Hello <system-reminder>\nmultiline\nreminder\n</system-reminder> World";
        assert_eq!(remove_internal_tags(text), "Hello  World");
    }

    #[test]
    fn test_remove_internal_tags_multiple() {
        let text = "<system-reminder>first</system-reminder> Middle <system-reminder>second</system-reminder>";
        assert_eq!(remove_internal_tags(text), "Middle");
    }

    #[test]
    fn test_remove_internal_tags_none() {
        let text = "No reminders here";
        assert_eq!(remove_internal_tags(text), "No reminders here");
    }

    // get_tool_names tests
    #[test]
    fn test_get_tool_names_empty() {
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z","message":{"content":[{"type":"text","text":"hello"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(entry.get_tool_names().is_empty());
    }

    #[test]
    fn test_get_tool_names_no_name() {
        // tool_use without name field
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z","message":{"content":[{"type":"tool_use"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(entry.get_tool_names().is_empty());
    }

    // truncate_with_ellipsis tests
    #[test]
    fn test_truncate_with_ellipsis_no_truncation() {
        let text = "Short text".to_string();
        assert_eq!(truncate_with_ellipsis(text, 20), "Short text");
    }

    #[test]
    fn test_truncate_with_ellipsis_exact_length() {
        let text = "Exact".to_string();
        assert_eq!(truncate_with_ellipsis(text, 5), "Exact");
    }

    #[test]
    fn test_truncate_with_ellipsis_truncated() {
        let text = "This is a long text that needs truncation".to_string();
        assert_eq!(truncate_with_ellipsis(text, 10), "This is a ...");
    }

    #[test]
    fn test_truncate_with_ellipsis_multibyte() {
        let text = "日本語テスト".to_string();
        assert_eq!(truncate_with_ellipsis(text, 3), "日本語...");
    }

    // is_interrupted tests
    #[test]
    fn test_is_interrupted_text_message() {
        // Plain text interruption message
        let json = r#"{"type":"user","timestamp":"2026-01-23T16:29:06.719Z","message":{"content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_interrupted());
    }

    #[test]
    fn test_is_interrupted_tool_result_with_error() {
        // tool_result with is_error: true and interruption content
        let json = r#"{"type":"user","timestamp":"2026-01-23T16:29:06.719Z","message":{"content":[{"type":"tool_result","content":"Exit code 137\n[Request interrupted by user for tool use]","is_error":true}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_interrupted());
    }

    #[test]
    fn test_is_interrupted_false_normal_user_message() {
        // Normal user message should not be interrupted
        let json = r#"{"type":"user","timestamp":"2026-01-23T16:29:06.719Z","message":{"content":[{"type":"text","text":"Hello Claude"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_interrupted());
    }

    #[test]
    fn test_is_interrupted_false_tool_result_no_error() {
        // tool_result without is_error should not be interrupted
        let json = r#"{"type":"user","timestamp":"2026-01-23T16:29:06.719Z","message":{"content":[{"type":"tool_result","content":"some output"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_interrupted());
    }

    #[test]
    fn test_is_interrupted_false_not_user() {
        // Non-user entry should not be interrupted
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_interrupted());
    }

    #[test]
    fn test_is_interrupted_simple_format() {
        // Simple interruption without "for tool use"
        let json = r#"{"type":"user","timestamp":"2026-01-23T16:29:06.719Z","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_interrupted());
    }

    // --- extract_conversation_turns tests ---

    #[test]
    fn test_extract_turns_basic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        let content = [
            r#"{"type":"user","message":{"content":"hello"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hi there!"}]}}"#,
            r#"{"type":"user","message":{"content":"fix the bug"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done!"}]}}"#,
        ]
        .join("\n");
        std::fs::write(&path, content).unwrap();

        let turns = extract_conversation_turns(&path, 50).unwrap();
        assert_eq!(turns.len(), 2);
        // Newest first
        assert_eq!(turns[0].user_prompt, "fix the bug");
        assert_eq!(turns[0].assistant_response, "Done!");
        assert_eq!(turns[1].user_prompt, "hello");
        assert_eq!(turns[1].assistant_response, "Hi there!");
    }

    #[test]
    fn test_extract_turns_skips_tool_result() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        let content = [
            r#"{"type":"user","message":{"content":"fix it"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Let me check..."},{"type":"tool_use","name":"Read"}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"file data"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Fixed!"}]}}"#,
        ]
        .join("\n");
        std::fs::write(&path, content).unwrap();

        let turns = extract_conversation_turns(&path, 50).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].user_prompt, "fix it");
        // Should keep the LAST assistant text (overwrite intermediate)
        assert_eq!(turns[0].assistant_response, "Fixed!");
    }

    #[test]
    fn test_extract_turns_max_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        let mut lines = Vec::new();
        for i in 0..10 {
            lines.push(format!(
                r#"{{"type":"user","message":{{"content":"prompt {}"}}}}"#,
                i
            ));
            lines.push(format!(
                r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"response {}"}}]}}}}"#,
                i
            ));
        }
        std::fs::write(&path, lines.join("\n")).unwrap();

        let turns = extract_conversation_turns(&path, 3).unwrap();
        assert_eq!(turns.len(), 3);
        // Newest first, so turn 9, 8, 7
        assert_eq!(turns[0].user_prompt, "prompt 9");
        assert_eq!(turns[2].user_prompt, "prompt 7");
    }

    #[test]
    fn test_extract_turns_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        std::fs::write(&path, "").unwrap();

        let turns = extract_conversation_turns(&path, 50).unwrap();
        assert!(turns.is_empty());
    }

    #[test]
    fn test_extract_turns_prompt_without_response() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        let content = r#"{"type":"user","message":{"content":"waiting..."}}"#;
        std::fs::write(&path, content).unwrap();

        let turns = extract_conversation_turns(&path, 50).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].user_prompt, "waiting...");
        assert_eq!(turns[0].assistant_response, "");
    }

    #[test]
    fn test_extract_turns_preserves_long_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        // Long text should be preserved in full (scrollable in detail view)
        let long_prompt = "x".repeat(10000);
        let long_response = "y".repeat(10000);
        let content = format!(
            r#"{{"type":"user","message":{{"content":"{}"}}}}"#,
            long_prompt
        ) + "\n"
            + &format!(
                r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"{}"}}]}}}}"#,
                long_response
            );
        std::fs::write(&path, content).unwrap();

        let turns = extract_conversation_turns(&path, 50).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].user_prompt.chars().count(), 10000);
        assert_eq!(turns[0].assistant_response.chars().count(), 10000);
    }

    #[test]
    fn test_extract_turns_with_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        let content = [
            r#"{"type":"user","timestamp":"2026-01-23T16:00:00.000Z","message":{"content":"hello"}}"#,
            r#"{"type":"assistant","timestamp":"2026-01-23T16:00:05.000Z","message":{"content":[{"type":"text","text":"Hi!"}]}}"#,
        ]
        .join("\n");
        std::fs::write(&path, content).unwrap();

        let turns = extract_conversation_turns(&path, 50).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].user_prompt, "hello");
        assert_eq!(turns[0].assistant_response, "Hi!");
        assert_eq!(
            turns[0].timestamp.as_deref(),
            Some("2026-01-23T16:00:00.000Z")
        );
    }

    // is_streaming tests
    #[test]
    fn test_is_streaming_true() {
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z","message":{"stop_reason":null,"content":[]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_streaming());
    }

    #[test]
    fn test_is_streaming_true_with_text() {
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z","message":{"stop_reason":null,"content":[{"type":"text","text":"partial"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(entry.is_streaming());
    }

    #[test]
    fn test_is_streaming_false_end_turn() {
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"Done"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_streaming());
    }

    #[test]
    fn test_is_streaming_false_tool_use() {
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z","message":{"stop_reason":null,"content":[{"type":"tool_use","name":"Read"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_streaming());
    }

    #[test]
    fn test_is_streaming_false_no_message() {
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z"}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_streaming());
    }

    #[test]
    fn test_is_streaming_false_not_assistant() {
        let json = r#"{"type":"user","timestamp":"2026-01-23T16:29:06.719Z"}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_streaming());
    }

    #[test]
    fn test_is_streaming_false_stop_reason_absent() {
        // stop_reason field missing entirely (legacy/malformed) must NOT be treated as streaming
        let json = r#"{"type":"assistant","timestamp":"2026-01-23T16:29:06.719Z","message":{"content":[{"type":"text","text":"done"}]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(json).unwrap();
        assert!(!entry.is_streaming());
    }

    #[test]
    fn test_stop_reason_absent_vs_null_deserialization() {
        // Absent: stop_reason field not in JSON → None
        let absent = r#"{"type":"assistant","timestamp":"t","message":{"content":[]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(absent).unwrap();
        assert_eq!(entry.message.as_ref().unwrap().stop_reason, None);

        // Null: stop_reason explicitly null → Some(None)
        let null =
            r#"{"type":"assistant","timestamp":"t","message":{"stop_reason":null,"content":[]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(null).unwrap();
        assert_eq!(entry.message.as_ref().unwrap().stop_reason, Some(None));

        // Present: stop_reason has value → Some(Some("end_turn"))
        let present = r#"{"type":"assistant","timestamp":"t","message":{"stop_reason":"end_turn","content":[]}}"#;
        let entry: TranscriptEntry = serde_json::from_str(present).unwrap();
        assert_eq!(
            entry.message.as_ref().unwrap().stop_reason,
            Some(Some("end_turn".to_string()))
        );
    }

    #[test]
    fn test_extract_turns_multi_turn_timestamps_with_max() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        // 3 turns: oldest(T1) -> middle(T2) -> newest(T3)
        let content = [
            r#"{"type":"user","timestamp":"2026-01-23T10:00:00.000Z","message":{"content":"first"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"resp1"}]}}"#,
            r#"{"type":"user","timestamp":"2026-01-23T11:00:00.000Z","message":{"content":"second"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"resp2"}]}}"#,
            r#"{"type":"user","timestamp":"2026-01-23T12:00:00.000Z","message":{"content":"third"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"resp3"}]}}"#,
        ]
        .join("\n");
        std::fs::write(&path, content).unwrap();

        // Request only 2 turns (newest first due to reverse)
        let turns = extract_conversation_turns(&path, 2).unwrap();
        assert_eq!(turns.len(), 2);
        // Newest first
        assert_eq!(turns[0].user_prompt, "third");
        assert_eq!(
            turns[0].timestamp.as_deref(),
            Some("2026-01-23T12:00:00.000Z")
        );
        assert_eq!(turns[1].user_prompt, "second");
        assert_eq!(
            turns[1].timestamp.as_deref(),
            Some("2026-01-23T11:00:00.000Z")
        );
    }

    // --- AskUserQuestion input parsing tests ---

    #[test]
    fn test_parse_ask_input_basic() {
        let json = r#"{"type":"tool_use","name":"AskUserQuestion","input":{"questions":[{"question":"Which approach?","header":"Approach","options":[{"label":"Simple","description":"Keep it simple"},{"label":"Complex","description":"Full featured"}],"multiSelect":false}]}}"#;
        let block: ContentBlock = serde_json::from_str(json).unwrap();
        assert_eq!(block.type_, "tool_use");
        assert_eq!(block.name.as_deref(), Some("AskUserQuestion"));
        let ask = block.parse_ask_input().unwrap();
        assert_eq!(ask.questions.len(), 1);
        assert_eq!(ask.questions[0].question, "Which approach?");
        assert_eq!(ask.questions[0].header.as_deref(), Some("Approach"));
        assert_eq!(ask.questions[0].options.len(), 2);
        assert_eq!(ask.questions[0].options[0].label, "Simple");
        assert_eq!(
            ask.questions[0].options[0].description.as_deref(),
            Some("Keep it simple")
        );
        assert!(!ask.questions[0].multi_select);
    }

    #[test]
    fn test_parse_ask_input_missing() {
        let json = r#"{"type":"tool_use","name":"AskUserQuestion"}"#;
        let block: ContentBlock = serde_json::from_str(json).unwrap();
        assert!(block.parse_ask_input().is_none());
    }

    #[test]
    fn test_parse_ask_input_wrong_shape() {
        let json = r#"{"type":"tool_use","name":"AskUserQuestion","input":{"not_questions":true}}"#;
        let block: ContentBlock = serde_json::from_str(json).unwrap();
        // Should fail to parse as AskUserQuestionInput (missing required `questions` field)
        assert!(block.parse_ask_input().is_none());
    }

    #[test]
    fn test_parse_ask_input_multiple_questions() {
        let json = r#"{"type":"tool_use","name":"AskUserQuestion","input":{"questions":[{"question":"Q1","header":"H1","options":[{"label":"A"}],"multiSelect":false},{"question":"Q2","header":"H2","options":[{"label":"B"},{"label":"C"}],"multiSelect":true}]}}"#;
        let block: ContentBlock = serde_json::from_str(json).unwrap();
        let ask = block.parse_ask_input().unwrap();
        assert_eq!(ask.questions.len(), 2);
        assert_eq!(ask.questions[1].question, "Q2");
        assert!(ask.questions[1].multi_select);
    }

    #[test]
    fn test_content_block_input_ignored_for_non_tool_use() {
        // input field on a text block should still deserialize fine
        let json = r#"{"type":"text","text":"hello","input":{"foo":"bar"}}"#;
        let block: ContentBlock = serde_json::from_str(json).unwrap();
        assert_eq!(block.type_, "text");
        assert!(block.input.is_some());
        assert!(block.parse_ask_input().is_none());
    }
}
