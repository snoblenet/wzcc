//! Install/uninstall the statusLine bridge for session tracking.
//!
//! This module provides commands to set up the Claude Code statusLine integration
//! that enables accurate session tracking when multiple sessions share the same CWD.

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// The bridge script content.
/// This script is executed by Claude Code's statusLine feature and writes
/// session information to a file that wzcc can read.
const BRIDGE_SCRIPT: &str = r#"#!/bin/bash
# wzcc statusLine bridge script
# This script is called by Claude Code's statusLine feature with session info on stdin.
# It writes the session information to a file that wzcc can read.

# Read JSON input from stdin
input=$(cat)

# Get TTY name from parent process (since stdin is piped, tty command won't work)
TTY=$(ps -o tty= -p $PPID 2>/dev/null | tr -d ' ' | tr '/' '-')

# Extract session info from JSON
SESSION_ID=$(echo "$input" | jq -r '.session_id // empty')
TRANSCRIPT_PATH=$(echo "$input" | jq -r '.transcript_path // empty')
CWD=$(echo "$input" | jq -r '.cwd // empty')

# Read hook-reported status so wzcc can override transcript-based WaitingForUser
# detection. When a tool is auto-approved and running for >10s the transcript
# heuristic incorrectly shows "Waiting"; the hook writes "active" in that case.
HOOK_STATUS=""
if [[ -n "$WEZTERM_PANE" ]]; then
    STATUS_FILE="/tmp/claude-status-msg-$WEZTERM_PANE"
    if [[ -f "$STATUS_FILE" ]]; then
        HOOK_STATUS=$(sed -n '2p' "$STATUS_FILE")
    fi
fi

# Only write if we have valid session info and TTY
if [[ -n "$SESSION_ID" && -n "$TTY" ]]; then
    mkdir -p ~/.claude/wzcc/sessions
    cat > ~/.claude/wzcc/sessions/${TTY}.json << EOF
{"session_id":"$SESSION_ID","transcript_path":"$TRANSCRIPT_PATH","cwd":"$CWD","tty":"$TTY","updated_at":"$(date -u +%Y-%m-%dT%H:%M:%SZ)","status":"$HOOK_STATUS"}
EOF
fi

# Chain to original statusLine command if configured
ORIGINAL_STATUSLINE="{{ORIGINAL_STATUSLINE}}"
if [[ -n "$ORIGINAL_STATUSLINE" && "$ORIGINAL_STATUSLINE" != "{{ORIGINAL_STATUSLINE}}" && "$ORIGINAL_STATUSLINE" != "" ]]; then
    echo "$input" | eval "$ORIGINAL_STATUSLINE"
fi
"#;

/// Get the path to the bridge script.
pub fn bridge_script_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".claude").join("wzcc_statusline_bridge.sh"))
}

/// Get the path to Claude's settings.json.
pub fn claude_settings_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".claude").join("settings.json"))
}

/// Read and parse Claude's settings.json.
fn read_claude_settings() -> Result<Value> {
    let path = claude_settings_path().context("Could not determine home directory")?;

    if !path.exists() {
        // Return empty object if settings don't exist
        return Ok(json!({}));
    }

    let content = fs::read_to_string(&path).context("Failed to read Claude settings.json")?;

    serde_json::from_str(&content).context("Failed to parse Claude settings.json")
}

/// Write Claude's settings.json.
fn write_claude_settings(settings: &Value) -> Result<()> {
    let path = claude_settings_path().context("Could not determine home directory")?;

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Failed to create .claude directory")?;
    }

    let content = serde_json::to_string_pretty(settings).context("Failed to serialize settings")?;

    fs::write(&path, content).context("Failed to write Claude settings.json")?;

    Ok(())
}

/// Install the statusLine bridge.
///
/// This function:
/// 1. Creates the bridge script at ~/.claude/wzcc_statusline_bridge.sh
/// 2. Updates ~/.claude/settings.json to use the bridge script for statusLine.command
/// 3. If an existing statusLine.command is configured, chains to it from the bridge
pub fn install_bridge() -> Result<()> {
    let bridge_path = bridge_script_path().context("Could not determine home directory")?;

    // Read existing settings
    let mut settings = read_claude_settings()?;

    // Check for existing statusLine command
    let existing_command = settings
        .get("statusLine")
        .and_then(|sl| sl.get("command"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string());

    // Don't chain to ourselves if we're already installed
    let original_command = match &existing_command {
        Some(cmd) if cmd.contains("wzcc_statusline_bridge") => None,
        other => other.clone(),
    };

    // Create bridge script with optional chaining
    let script_content = if let Some(ref original) = original_command {
        BRIDGE_SCRIPT.replace("{{ORIGINAL_STATUSLINE}}", original)
    } else {
        BRIDGE_SCRIPT.replace("{{ORIGINAL_STATUSLINE}}", "")
    };

    // Ensure parent directory exists
    if let Some(parent) = bridge_path.parent() {
        fs::create_dir_all(parent).context("Failed to create .claude directory")?;
    }

    // Write bridge script
    fs::write(&bridge_path, script_content).context("Failed to write bridge script")?;

    // Make script executable
    let mut perms = fs::metadata(&bridge_path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&bridge_path, perms).context("Failed to set script permissions")?;

    // Update settings.json
    let bridge_command = bridge_path.to_string_lossy().to_string();

    // Ensure statusLine object exists
    if settings.get("statusLine").is_none() {
        settings["statusLine"] = json!({});
    }

    settings["statusLine"]["command"] = json!(bridge_command);

    write_claude_settings(&settings)?;

    println!("Bridge installed successfully!");
    println!();
    println!("  Bridge script: {}", bridge_path.display());
    println!(
        "  statusLine.command: {}",
        settings["statusLine"]["command"]
    );

    if let Some(original) = original_command {
        println!();
        println!("  Note: Your existing statusLine command has been preserved:");
        println!("    {}", original);
        println!("  It will be called after wzcc writes session info.");
    }

    println!();
    println!("Please restart your Claude Code sessions for changes to take effect.");

    Ok(())
}

/// Uninstall the statusLine bridge.
///
/// This function:
/// 1. Removes the bridge script from ~/.claude/wzcc_statusline_bridge.sh
/// 2. Restores the original statusLine.command if one was chained, or removes it
/// 3. Cleans up the sessions directory
pub fn uninstall_bridge() -> Result<()> {
    let bridge_path = bridge_script_path().context("Could not determine home directory")?;

    // Check if bridge script exists and extract original command
    let original_command = if bridge_path.exists() {
        let content = fs::read_to_string(&bridge_path).ok();
        content.and_then(|c| {
            // Extract the ORIGINAL_STATUSLINE value from the script
            for line in c.lines() {
                if line.starts_with("ORIGINAL_STATUSLINE=") {
                    let value = line
                        .trim_start_matches("ORIGINAL_STATUSLINE=")
                        .trim_matches('"');
                    if !value.is_empty() && !value.contains("{{ORIGINAL_STATUSLINE}}") {
                        return Some(value.to_string());
                    }
                }
            }
            None
        })
    } else {
        None
    };

    // Remove bridge script
    if bridge_path.exists() {
        fs::remove_file(&bridge_path).context("Failed to remove bridge script")?;
        println!("Removed bridge script: {}", bridge_path.display());
    }

    // Update settings.json
    let mut settings = read_claude_settings()?;

    if let Some(status_line) = settings.get_mut("statusLine") {
        if let Some(obj) = status_line.as_object_mut() {
            if let Some(original) = original_command {
                // Restore original command
                obj.insert("command".to_string(), json!(original));
                println!("Restored original statusLine command: {}", original);
            } else {
                // Remove command entirely
                obj.remove("command");
                println!("Removed statusLine.command from settings");
            }

            // Remove statusLine object if empty
            if obj.is_empty() {
                if let Some(root) = settings.as_object_mut() {
                    root.remove("statusLine");
                }
            }
        }
    }

    write_claude_settings(&settings)?;

    // Clean up sessions directory
    if let Some(sessions_dir) = crate::session_mapping::SessionMapping::sessions_dir() {
        if sessions_dir.exists() {
            fs::remove_dir_all(&sessions_dir).ok();
            println!("Cleaned up sessions directory: {}", sessions_dir.display());
        }
    }

    println!();
    println!("Bridge uninstalled successfully!");
    println!("Please restart your Claude Code sessions for changes to take effect.");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bridge_script_path() {
        let path = bridge_script_path();
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(path.ends_with("wzcc_statusline_bridge.sh"));
    }

    #[test]
    fn test_claude_settings_path() {
        let path = claude_settings_path();
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(path.ends_with("settings.json"));
    }

    #[test]
    fn test_bridge_script_content() {
        // Verify the script contains expected elements
        assert!(BRIDGE_SCRIPT.contains("#!/bin/bash"));
        assert!(BRIDGE_SCRIPT.contains("jq"));
        assert!(BRIDGE_SCRIPT.contains("session_id"));
        assert!(BRIDGE_SCRIPT.contains("transcript_path"));
        assert!(BRIDGE_SCRIPT.contains("{{ORIGINAL_STATUSLINE}}"));
    }

    #[test]
    fn test_bridge_script_replacement() {
        let script = BRIDGE_SCRIPT.replace("{{ORIGINAL_STATUSLINE}}", "echo 'hello'");
        assert!(script.contains("echo 'hello'"));
        assert!(!script.contains("{{ORIGINAL_STATUSLINE}}"));
    }
}
