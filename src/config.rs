use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

/// A named command that can be spawned in a new pane.
#[derive(Debug, Deserialize, Clone)]
pub struct SpawnCommand {
    /// Display name shown in the command selector (e.g. "Claude Code").
    pub name: String,
    /// Program and arguments (e.g. ["claude", "--flag"]).
    pub command: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    /// Legacy field — kept for backward compatibility.
    /// Prefer `[[commands]]` for new configurations.
    pub spawn_command: Option<Vec<String>>,

    /// Named command list. Each entry has a `name` and `command`.
    pub commands: Option<Vec<SpawnCommand>>,
}

impl Config {
    /// Load configuration from ~/.config/wzcc/config.toml
    ///
    /// - File missing: returns default config (Ok)
    /// - File exists but invalid TOML: returns Err so caller can show warning
    /// - Field missing or empty array: uses default ["claude"]
    pub fn load() -> Result<Self> {
        let path = match Self::config_path() {
            Some(p) => p,
            None => return Ok(Self::default()),
        };

        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

        Ok(config)
    }

    /// Resolve the list of spawn commands from config.
    ///
    /// Priority:
    ///   1. `commands` array — filter out invalid entries, use if non-empty
    ///   2. `spawn_command` — wrap as a single SpawnCommand
    ///   3. Default: `[{ name: "Claude", command: ["claude"] }]`
    pub fn resolved_commands(&self) -> Vec<SpawnCommand> {
        // Try `commands` first
        if let Some(cmds) = &self.commands {
            let valid: Vec<SpawnCommand> = cmds
                .iter()
                .filter(|c| Self::is_valid_command(c))
                .cloned()
                .collect();
            if !valid.is_empty() {
                return valid;
            }
        }

        // Fall back to legacy `spawn_command`
        if let Some(cmd) = &self.spawn_command {
            if !cmd.is_empty() && !cmd[0].trim().is_empty() {
                let name = cmd[0].clone();
                return vec![SpawnCommand {
                    name,
                    command: cmd.clone(),
                }];
            }
        }

        // Default
        vec![SpawnCommand {
            name: "Claude".to_string(),
            command: vec!["claude".to_string()],
        }]
    }

    /// Extract (program, args) from a SpawnCommand.
    /// Falls back to ("claude", []) for invalid entries.
    pub fn program_and_args(cmd: &SpawnCommand) -> (&str, &[String]) {
        if !cmd.command.is_empty() && !cmd.command[0].trim().is_empty() {
            (&cmd.command[0], &cmd.command[1..])
        } else {
            ("claude", &[])
        }
    }

    /// Check whether a SpawnCommand has a valid (non-empty, non-whitespace) program.
    fn is_valid_command(cmd: &SpawnCommand) -> bool {
        !cmd.command.is_empty() && !cmd.command[0].trim().is_empty()
    }

    fn config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|d| d.join(".config").join("wzcc").join("config.toml"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_config(
        spawn_command: Option<Vec<String>>,
        commands: Option<Vec<SpawnCommand>>,
    ) -> Config {
        Config {
            spawn_command,
            commands,
        }
    }

    // --- resolved_commands tests ---

    #[test]
    fn test_resolved_commands_default() {
        let config = Config::default();
        let cmds = config.resolved_commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "Claude");
        assert_eq!(cmds[0].command, vec!["claude"]);
    }

    #[test]
    fn test_resolved_commands_from_legacy_spawn_command() {
        let config = make_config(Some(vec!["claude".into(), "--flag".into()]), None);
        let cmds = config.resolved_commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "claude");
        assert_eq!(cmds[0].command, vec!["claude", "--flag"]);
    }

    #[test]
    fn test_resolved_commands_from_commands_array() {
        let config = make_config(
            None,
            Some(vec![
                SpawnCommand {
                    name: "Claude".into(),
                    command: vec!["claude".into()],
                },
                SpawnCommand {
                    name: "Codex".into(),
                    command: vec!["codex".into()],
                },
            ]),
        );
        let cmds = config.resolved_commands();
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].name, "Claude");
        assert_eq!(cmds[1].name, "Codex");
    }

    #[test]
    fn test_resolved_commands_commands_takes_precedence() {
        let config = make_config(
            Some(vec!["old-cmd".into()]),
            Some(vec![SpawnCommand {
                name: "New".into(),
                command: vec!["new-cmd".into()],
            }]),
        );
        let cmds = config.resolved_commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "New");
    }

    #[test]
    fn test_resolved_commands_filters_invalid_entries() {
        let config = make_config(
            None,
            Some(vec![
                SpawnCommand {
                    name: "Valid".into(),
                    command: vec!["good".into()],
                },
                SpawnCommand {
                    name: "Empty".into(),
                    command: vec![],
                },
                SpawnCommand {
                    name: "Whitespace".into(),
                    command: vec!["  ".into()],
                },
            ]),
        );
        let cmds = config.resolved_commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "Valid");
    }

    #[test]
    fn test_resolved_commands_all_invalid_falls_back_to_legacy() {
        let config = make_config(
            Some(vec!["legacy-cmd".into()]),
            Some(vec![SpawnCommand {
                name: "Bad".into(),
                command: vec![],
            }]),
        );
        let cmds = config.resolved_commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].command, vec!["legacy-cmd"]);
    }

    #[test]
    fn test_resolved_commands_all_invalid_no_legacy_uses_default() {
        let config = make_config(
            None,
            Some(vec![SpawnCommand {
                name: "Bad".into(),
                command: vec!["".into()],
            }]),
        );
        let cmds = config.resolved_commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "Claude");
        assert_eq!(cmds[0].command, vec!["claude"]);
    }

    // --- program_and_args tests ---

    #[test]
    fn test_program_and_args_normal() {
        let cmd = SpawnCommand {
            name: "Test".into(),
            command: vec!["prog".into(), "arg1".into()],
        };
        let (prog, args) = Config::program_and_args(&cmd);
        assert_eq!(prog, "prog");
        assert_eq!(args, &["arg1".to_string()]);
    }

    #[test]
    fn test_program_and_args_no_args() {
        let cmd = SpawnCommand {
            name: "Test".into(),
            command: vec!["prog".into()],
        };
        let (prog, args) = Config::program_and_args(&cmd);
        assert_eq!(prog, "prog");
        assert!(args.is_empty());
    }

    #[test]
    fn test_program_and_args_empty_command_fallback() {
        let cmd = SpawnCommand {
            name: "Test".into(),
            command: vec![],
        };
        let (prog, args) = Config::program_and_args(&cmd);
        assert_eq!(prog, "claude");
        assert!(args.is_empty());
    }

    #[test]
    fn test_program_and_args_whitespace_fallback() {
        let cmd = SpawnCommand {
            name: "Test".into(),
            command: vec!["  ".into()],
        };
        let (prog, args) = Config::program_and_args(&cmd);
        assert_eq!(prog, "claude");
        assert!(args.is_empty());
    }

    // --- TOML parsing tests ---

    #[test]
    fn test_parse_toml_commands_format() {
        let content = r#"
[[commands]]
name = "Claude Code"
command = ["claude"]

[[commands]]
name = "Codex"
command = ["codex", "--flag"]
"#;
        let config: Config = toml::from_str(content).unwrap();
        let cmds = config.resolved_commands();
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].name, "Claude Code");
        assert_eq!(cmds[1].name, "Codex");
        assert_eq!(cmds[1].command, vec!["codex", "--flag"]);
    }

    #[test]
    fn test_parse_toml_legacy_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut file = fs::File::create(&path).unwrap();
        writeln!(file, r#"spawn_command = ["claude", "--flag"]"#).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let config: Config = toml::from_str(&content).unwrap();
        let cmds = config.resolved_commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].command, vec!["claude", "--flag"]);
    }

    #[test]
    fn test_parse_toml_invalid() {
        let invalid = "spawn_command = [[[invalid";
        let result: std::result::Result<Config, _> = toml::from_str(invalid);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_toml_empty() {
        let content = "# empty config\n";
        let config: Config = toml::from_str(content).unwrap();
        let cmds = config.resolved_commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "Claude");
    }
}
