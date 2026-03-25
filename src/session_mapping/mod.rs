//! Session mapping module for TTY-based session identification.
//!
//! This module provides functionality to map Claude Code sessions to their
//! transcript files using TTY as the key. This enables accurate session
//! tracking even when multiple sessions share the same working directory.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Session mapping information written by the statusLine bridge script.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMapping {
    /// Claude Code session ID (UUID)
    pub session_id: String,
    /// Path to the transcript file
    pub transcript_path: PathBuf,
    /// Current working directory
    pub cwd: String,
    /// TTY name (without /dev/ prefix, e.g., "ttys003")
    pub tty: String,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
    /// Optional status override from external hooks (e.g., "active" or "waiting").
    /// When present and set to "active", takes precedence over transcript-based
    /// WaitingForUser detection to prevent false positives from long-running tools.
    #[serde(default)]
    pub status: Option<String>,
}

/// Result of reading a session mapping
#[derive(Debug, Clone)]
pub enum MappingResult {
    /// Valid, fresh mapping
    Valid(SessionMapping),
    /// Mapping exists but is stale (>5 minutes old)
    Stale(SessionMapping),
    /// No mapping exists for this TTY
    NotFound,
}

impl SessionMapping {
    /// Get the sessions directory path (~/.claude/wzcc/sessions/)
    pub fn sessions_dir() -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        Some(home.join(".claude").join("wzcc").join("sessions"))
    }

    /// Get the mapping file path for a given TTY
    pub fn mapping_file_path(tty: &str) -> Option<PathBuf> {
        // Sanitize TTY name for use as filename (replace / with -)
        let safe_tty = tty.replace('/', "-");
        Some(Self::sessions_dir()?.join(format!("{}.json", safe_tty)))
    }

    /// Read session mapping from a TTY.
    ///
    /// # Arguments
    /// * `tty` - TTY name (e.g., "ttys003" or "/dev/ttys003")
    ///
    /// # Returns
    /// * `Some(SessionMapping)` if a valid mapping exists and is not stale
    /// * `None` if no mapping exists, is invalid, or is stale (>5 minutes old)
    pub fn from_tty(tty: &str) -> Option<Self> {
        match Self::from_tty_with_status(tty) {
            MappingResult::Valid(mapping) => Some(mapping),
            _ => None,
        }
    }

    /// Read session mapping from a TTY with detailed status.
    ///
    /// # Arguments
    /// * `tty` - TTY name (e.g., "ttys003" or "/dev/ttys003")
    ///
    /// # Returns
    /// * `MappingResult::Valid(mapping)` if a valid mapping exists and is fresh
    /// * `MappingResult::Stale(mapping)` if mapping exists but is >5 minutes old
    /// * `MappingResult::NotFound` if no mapping exists or is invalid
    pub fn from_tty_with_status(tty: &str) -> MappingResult {
        // Normalize TTY name (remove /dev/ prefix if present)
        let tty_short = tty.strip_prefix("/dev/").unwrap_or(tty);

        let path = match Self::mapping_file_path(tty_short) {
            Some(p) => p,
            None => return MappingResult::NotFound,
        };

        if !path.exists() {
            return MappingResult::NotFound;
        }

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return MappingResult::NotFound,
        };

        let mapping: SessionMapping = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(_) => return MappingResult::NotFound,
        };

        // Check if mapping is stale (>5 minutes old)
        // statusLine updates every 300ms, so 5 minutes without update means session is gone
        let now = Utc::now();
        let age = now.signed_duration_since(mapping.updated_at);
        if age.num_minutes() > 5 {
            return MappingResult::Stale(mapping);
        }

        MappingResult::Valid(mapping)
    }

    /// Read all valid session mappings from the sessions directory.
    ///
    /// # Returns
    /// A vector of all non-stale session mappings
    pub fn all_mappings() -> Vec<Self> {
        let sessions_dir = match Self::sessions_dir() {
            Some(dir) => dir,
            None => return Vec::new(),
        };

        if !sessions_dir.exists() {
            return Vec::new();
        }

        let mut mappings = Vec::new();

        if let Ok(entries) = fs::read_dir(&sessions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();

                // Only consider .json files
                if path.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }

                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(mapping) = serde_json::from_str::<SessionMapping>(&content) {
                        // Check staleness
                        let now = Utc::now();
                        let age = now.signed_duration_since(mapping.updated_at);
                        if age.num_minutes() <= 5 {
                            mappings.push(mapping);
                        } else {
                            // Remove stale mapping
                            let _ = fs::remove_file(&path);
                        }
                    }
                }
            }
        }

        mappings
    }

    /// Clean up stale mapping files (>5 minutes old).
    ///
    /// This is called periodically to remove mappings from sessions that
    /// have been closed without proper cleanup.
    pub fn cleanup_stale() -> Result<()> {
        let sessions_dir = match Self::sessions_dir() {
            Some(dir) => dir,
            None => return Ok(()),
        };

        if !sessions_dir.exists() {
            return Ok(());
        }

        let now = Utc::now();

        for entry in fs::read_dir(&sessions_dir)? {
            let entry = entry?;
            let path = entry.path();

            // Only consider .json files
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }

            // Try to read and parse the mapping
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(mapping) = serde_json::from_str::<SessionMapping>(&content) {
                    let age = now.signed_duration_since(mapping.updated_at);
                    if age.num_minutes() > 5 {
                        // Remove stale mapping
                        let _ = fs::remove_file(&path);
                    }
                } else {
                    // Invalid JSON, remove it
                    let _ = fs::remove_file(&path);
                }
            }
        }

        Ok(())
    }

    /// Clean up mapping files for TTYs that no longer exist.
    ///
    /// This function removes mapping files for TTYs that are not in the
    /// provided list of active TTYs. This is safe to call at startup
    /// because it only removes mappings for TTYs that definitely don't
    /// exist in the current WezTerm session.
    ///
    /// # Arguments
    /// * `active_ttys` - List of TTY names currently in use (e.g., ["ttys001", "ttys005"])
    ///
    /// # Returns
    /// Number of files removed
    pub fn cleanup_inactive_ttys(active_ttys: &[String]) -> usize {
        let sessions_dir = match Self::sessions_dir() {
            Some(dir) => dir,
            None => return 0,
        };

        if !sessions_dir.exists() {
            return 0;
        }

        let mut removed_count = 0;

        if let Ok(entries) = fs::read_dir(&sessions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();

                // Only consider .json files
                if path.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }

                // Extract TTY name from filename (e.g., "ttys001.json" -> "ttys001")
                let tty_name = match path.file_stem().and_then(|s| s.to_str()) {
                    Some(name) => name,
                    None => continue,
                };

                // Check if this TTY is in the active list
                if !active_ttys.iter().any(|t| t == tty_name) {
                    // TTY not active, safe to remove
                    if fs::remove_file(&path).is_ok() {
                        removed_count += 1;
                    }
                }
            }
        }

        removed_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sessions_dir() {
        let dir = SessionMapping::sessions_dir();
        assert!(dir.is_some());
        let dir = dir.unwrap();
        assert!(dir.ends_with(".claude/wzcc/sessions"));
    }

    #[test]
    fn test_mapping_file_path() {
        let path = SessionMapping::mapping_file_path("ttys003");
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(path.ends_with("ttys003.json"));
    }

    #[test]
    fn test_mapping_file_path_with_slash() {
        let path = SessionMapping::mapping_file_path("pts/0");
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(path.ends_with("pts-0.json"));
    }

    #[test]
    fn test_from_tty_nonexistent() {
        // Should return None for nonexistent TTY
        let mapping = SessionMapping::from_tty("nonexistent_tty_12345");
        assert!(mapping.is_none());
    }

    #[test]
    fn test_from_tty_strips_dev_prefix() {
        // Both should work the same way
        let m1 = SessionMapping::from_tty("ttys999");
        let m2 = SessionMapping::from_tty("/dev/ttys999");
        // Both should be None since the file doesn't exist
        assert!(m1.is_none());
        assert!(m2.is_none());
    }

    #[test]
    fn test_stale_mapping_preserves_data() {
        // Write a mapping file with a timestamp >5 minutes in the past
        let sessions_dir = SessionMapping::sessions_dir().unwrap();
        fs::create_dir_all(&sessions_dir).unwrap();

        let tty = "test_stale_tty_99999";
        let path = SessionMapping::mapping_file_path(tty).unwrap();
        let transcript_path = PathBuf::from("/tmp/test-transcript.jsonl");

        let mapping = SessionMapping {
            session_id: "stale-session-id".to_string(),
            transcript_path: transcript_path.clone(),
            cwd: "/tmp/test".to_string(),
            tty: tty.to_string(),
            updated_at: Utc::now() - chrono::Duration::minutes(10),
            status: None,
        };
        fs::write(&path, serde_json::to_string(&mapping).unwrap()).unwrap();

        let result = SessionMapping::from_tty_with_status(tty);

        // Clean up before assertions so file is removed even on failure
        let _ = fs::remove_file(&path);

        match result {
            MappingResult::Stale(stale_mapping) => {
                assert_eq!(stale_mapping.session_id, "stale-session-id");
                assert_eq!(stale_mapping.transcript_path, transcript_path);
            }
            other => panic!("Expected MappingResult::Stale, got {:?}", other),
        }
    }

    #[test]
    fn test_session_mapping_serialization() {
        let mapping = SessionMapping {
            session_id: "test-uuid-1234".to_string(),
            transcript_path: PathBuf::from("/Users/test/.claude/projects/test/abc.jsonl"),
            cwd: "/Users/test/project".to_string(),
            tty: "ttys003".to_string(),
            updated_at: Utc::now(),
            status: None,
        };

        let json = serde_json::to_string(&mapping).unwrap();
        let parsed: SessionMapping = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.session_id, mapping.session_id);
        assert_eq!(parsed.transcript_path, mapping.transcript_path);
        assert_eq!(parsed.cwd, mapping.cwd);
        assert_eq!(parsed.tty, mapping.tty);
    }
}
