use crate::models::Pane;
use crate::session_mapping::{MappingResult, SessionMapping};
use std::path::PathBuf;
use std::time::SystemTime;

use super::info::WaitingPrompt;
use super::{
    get_latest_transcript, get_transcript_dir, read_transcript_info, SessionStatus, TranscriptInfo,
};

/// Result of session info detection.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub status: SessionStatus,
    pub last_prompt: Option<String>,
    pub last_output: Option<String>,
    pub session_id: Option<String>,
    pub transcript_path: Option<PathBuf>,
    /// Last updated time (from transcript file modification time)
    pub updated_at: Option<SystemTime>,
    /// Warning message to display (e.g., stale mapping)
    pub warning: Option<String>,
    /// Parsed waiting prompt data when status is WaitingForUser.
    pub waiting_prompt: Option<WaitingPrompt>,
}

/// Get file modification time.
fn get_file_mtime(path: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Detect session info from statusLine bridge mapping (TTY-based).
///
/// This function tries to find session information using the TTY as the key.
/// If a valid mapping exists, it uses the transcript_path from the mapping
/// instead of guessing based on CWD.
pub fn detect_session_info(pane: &Pane) -> SessionInfo {
    // Try to get session mapping from TTY
    if let Some(tty) = pane.tty_short() {
        match SessionMapping::from_tty_with_status(&tty) {
            MappingResult::Valid(mapping) => {
                // We have a valid mapping - use the transcript path from it
                let transcript_path = mapping.transcript_path.clone();

                let (status, last_prompt, last_output, updated_at, waiting_prompt) =
                    if transcript_path.exists() {
                        let info =
                            read_transcript_info(&transcript_path).unwrap_or(TranscriptInfo {
                                status: SessionStatus::Unknown,
                                last_prompt: None,
                                last_output: None,
                                waiting_prompt: None,
                            });
                        let mtime = get_file_mtime(&transcript_path);
                        // If the bridge reports "active" but the transcript heuristic says
                        // WaitingForUser, the tool was auto-approved and is still executing.
                        // Downgrade to Processing to avoid false "Waiting" indicators.
                        let effective_status =
                            match (&info.status, mapping.status.as_deref()) {
                                (SessionStatus::WaitingForUser { .. }, Some("active")) => {
                                    SessionStatus::Processing
                                }
                                _ => info.status,
                            };
                        let effective_waiting_prompt =
                            if matches!(effective_status, SessionStatus::Processing) {
                                None
                            } else {
                                info.waiting_prompt
                            };
                        (
                            effective_status,
                            info.last_prompt,
                            info.last_output,
                            mtime,
                            effective_waiting_prompt,
                        )
                    } else {
                        (SessionStatus::Ready, None, None, None, None)
                    };

                return SessionInfo {
                    status,
                    last_prompt,
                    last_output,
                    session_id: Some(mapping.session_id),
                    transcript_path: Some(transcript_path),
                    updated_at,
                    warning: None,
                    waiting_prompt,
                };
            }
            MappingResult::Stale(mapping) => {
                // Mapping exists but is stale - don't fallback to CWD
                // This prevents showing wrong status from another session with same CWD
                // Read transcript for actual status instead of showing Unknown
                let transcript_path = mapping.transcript_path.clone();
                let (status, last_prompt, last_output, updated_at, waiting_prompt) =
                    if transcript_path.exists() {
                        let info =
                            read_transcript_info(&transcript_path).unwrap_or(TranscriptInfo {
                                status: SessionStatus::Unknown,
                                last_prompt: None,
                                last_output: None,
                                waiting_prompt: None,
                            });
                        (
                            info.status,
                            info.last_prompt,
                            info.last_output,
                            get_file_mtime(&transcript_path),
                            info.waiting_prompt,
                        )
                    } else {
                        (SessionStatus::Unknown, None, None, None, None)
                    };

                return SessionInfo {
                    status,
                    last_prompt,
                    last_output,
                    session_id: Some(mapping.session_id),
                    transcript_path: Some(transcript_path),
                    updated_at,
                    warning: Some(
                        "Session info stale (statusLine not updating). Try interacting with the session.".to_string(),
                    ),
                    waiting_prompt,
                };
            }
            MappingResult::NotFound => {
                // No mapping - fall through to CWD-based detection
            }
        }
    }

    // Fallback to CWD-based detection
    let (status, last_prompt, last_output, updated_at, waiting_prompt) =
        detect_status_and_output_by_cwd(pane);

    SessionInfo {
        status,
        last_prompt,
        last_output,
        session_id: None,
        transcript_path: None,
        updated_at,
        warning: None,
        waiting_prompt,
    }
}

/// Detect session info by CWD (legacy method).
fn detect_status_and_output_by_cwd(
    pane: &Pane,
) -> (
    SessionStatus,
    Option<String>,
    Option<String>,
    Option<SystemTime>,
    Option<WaitingPrompt>,
) {
    let cwd = match pane.cwd_path() {
        Some(cwd) => cwd,
        None => return (SessionStatus::Unknown, None, None, None, None),
    };

    let dir = match get_transcript_dir(&cwd) {
        Some(dir) => dir,
        // No transcript directory = Claude Code is running but no session yet
        None => return (SessionStatus::Ready, None, None, None, None),
    };

    let transcript_path = match get_latest_transcript(&dir) {
        Ok(Some(path)) => path,
        // No transcript file = Claude Code is running but no session yet
        _ => return (SessionStatus::Ready, None, None, None, None),
    };

    let info = read_transcript_info(&transcript_path).unwrap_or(TranscriptInfo {
        status: SessionStatus::Unknown,
        last_prompt: None,
        last_output: None,
        waiting_prompt: None,
    });
    let updated_at = get_file_mtime(&transcript_path);

    (
        info.status,
        info.last_prompt,
        info.last_output,
        updated_at,
        info.waiting_prompt,
    )
}
