use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug, PartialEq)]
#[command(
    name = "jabberwok",
    version,
    about = "A key event handler — runs as a CLI or a background service"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum Commands {
    /// Run as a background service that responds to key events
    Daemon {
        /// Path to the model (overrides the default from config)
        #[arg(short, long)]
        model: Option<std::path::PathBuf>,
        /// Models config file
        #[arg(long)]
        config: Option<std::path::PathBuf>,
        /// Save each utterance as a WAV + TXT pair in the utterances/ directory
        #[arg(long)]
        save_utterances: bool,
    },
    /// List all available audio input and output devices
    ListDevices {
        /// App config file
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// Record audio from the default input device to a 16 kHz mono 16-bit PCM WAV file
    Record {
        /// Output file path
        #[arg(short, long, default_value = "recording.wav")]
        output: std::path::PathBuf,
        /// Recording duration in seconds
        #[arg(short, long, default_value_t = 5)]
        duration: u64,
    },
    /// Select the audio input or output device by index or name
    SelectDevice {
        /// Input device: 1-based index, full name, or partial name (0 or "" to clear)
        #[arg(long)]
        input: Option<String>,
        /// Output device: 1-based index, full name, or partial name (0 or "" to clear)
        #[arg(long)]
        output: Option<String>,
        /// Hostname to configure (defaults to current machine; use "default" for a global fallback)
        #[arg(long)]
        host: Option<String>,
        /// App config file
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// Download a known model by name and set it as the default
    DownloadModel {
        /// Name of the model to download (defaults to the configured default or parakeet-v3)
        model: Option<String>,
        /// Models config file
        #[arg(long)]
        config: Option<std::path::PathBuf>,
        /// Directory to store downloaded model files (defaults next to the active config)
        #[arg(short, long)]
        models_dir: Option<std::path::PathBuf>,
    },
    /// Transcribe audio from a file or from the microphone (default)
    Transcribe {
        /// Path to the model (overrides the default from config)
        #[arg(short, long)]
        model: Option<std::path::PathBuf>,
        /// Models config file
        #[arg(long)]
        config: Option<std::path::PathBuf>,
        /// WAV file to transcribe; if omitted, records from the microphone
        #[arg(short, long)]
        file: Option<std::path::PathBuf>,
        /// Microphone recording duration in seconds (ignored when --file is used)
        #[arg(short, long, default_value_t = 5)]
        duration: u64,
        /// Inject the transcription into the focused input instead of printing it
        #[arg(long = "input")]
        inject: bool,
        /// Save the utterance as a WAV + TXT pair in the utterances/ directory
        #[arg(long)]
        save_utterances: bool,
    },
    /// Inject text at the currently focused text input via the Accessibility API
    Type {
        /// The text to inject
        text: String,
    },
    /// Show setup/ready state for first run
    #[command(hide = true)]
    Setup {
        /// App config file
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// Run the first-launch tutorial window (ignores has_seen_tutorial)
    #[command(hide = true)]
    Tutorial {
        /// App config file
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// Show runtime readiness for daemon startup
    Doctor {
        /// App config file
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// Check and request permissions
    Permissions {
        /// Permission area
        target: PermissionTarget,
        /// Remove cached permission state (debug-only; requires debug build)
        #[arg(long)]
        remove: bool,
    },
    /// Manage the macOS LaunchAgent used to keep Jabberwok running after login
    LaunchAgent {
        /// Operation to perform
        action: LaunchAgentAction,
    },
    /// Remove local Jabberwok data and optionally relaunch into setup
    Reset {
        /// App config file
        #[arg(long)]
        config: Option<std::path::PathBuf>,
        /// Wait for a process to exit before removing local data
        #[arg(long)]
        wait_for_pid: Option<u32>,
        /// Relaunch Jabberwok after reset completes
        #[arg(long)]
        relaunch: bool,
    },
}

#[derive(Clone, Debug, PartialEq, ValueEnum)]
pub enum PermissionTarget {
    Microphone,
    Accessibility,
    All,
}

#[derive(Clone, Debug, PartialEq, ValueEnum)]
pub enum LaunchAgentAction {
    Install,
    Uninstall,
    Status,
}

impl PermissionTarget {
    pub fn command_name(&self) -> &'static str {
        match self {
            PermissionTarget::Microphone => "microphone",
            PermissionTarget::Accessibility => "accessibility",
            PermissionTarget::All => "all",
        }
    }
}

/// Map `-?` to `--help` so both spellings work.
pub fn normalize_args<I, S>(args: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    args.into_iter()
        .filter_map(|a| {
            let s: String = a.into();
            if s == "-?" {
                Some("--help".to_string())
            } else if is_macos_process_serial_arg(&s) {
                None
            } else {
                Some(s)
            }
        })
        .collect()
}

fn is_macos_process_serial_arg(arg: &str) -> bool {
    arg.starts_with("-psn_")
}

/// Return the help text as a string (useful in tests and for `--help` output).
pub fn help_text() -> String {
    let mut cmd = Cli::command();
    let mut buf = Vec::new();
    cmd.write_help(&mut buf)
        .expect("clap failed to render help");
    String::from_utf8(buf).expect("help text is not valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_question_mark_to_help() {
        let args = normalize_args(["jabberwok", "-?"]);
        assert_eq!(args, vec!["jabberwok", "--help"]);
    }

    #[test]
    fn normalize_leaves_other_args_unchanged() {
        let args = normalize_args(["jabberwok", "daemon", "--version"]);
        assert_eq!(args, vec!["jabberwok", "daemon", "--version"]);
    }

    #[test]
    fn normalize_drops_macos_process_serial_number_arg() {
        let args = normalize_args(["jabberwok", "-psn_0_12345"]);
        assert_eq!(args, vec!["jabberwok"]);
    }

    #[test]
    fn no_subcommand_parses_ok() {
        let cli = Cli::try_parse_from(["jabberwok"]).expect("parse failed");
        assert_eq!(cli.command, None);
    }

    #[test]
    fn daemon_subcommand_parses_ok() {
        let cli = Cli::try_parse_from(["jabberwok", "daemon"]).expect("parse failed");
        assert_eq!(
            cli.command,
            Some(Commands::Daemon {
                model: None,
                config: None,
                save_utterances: false,
            })
        );
    }

    #[test]
    fn list_devices_subcommand_parses_ok() {
        let cli = Cli::try_parse_from(["jabberwok", "list-devices"]).expect("parse failed");
        assert_eq!(cli.command, Some(Commands::ListDevices { config: None }));
    }

    #[test]
    fn unknown_subcommand_is_an_error() {
        let result = Cli::try_parse_from(["jabberwok", "bogus"]);
        assert!(result.is_err());
    }

    #[test]
    fn launch_agent_subcommand_parses_ok() {
        let cli =
            Cli::try_parse_from(["jabberwok", "launch-agent", "status"]).expect("parse failed");
        assert_eq!(
            cli.command,
            Some(Commands::LaunchAgent {
                action: LaunchAgentAction::Status,
            })
        );
    }

    #[test]
    fn help_flag_triggers_exit() {
        // clap returns an Err with kind DisplayHelp when --help is passed
        let err = Cli::try_parse_from(["jabberwok", "--help"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);
    }

    #[test]
    fn question_mark_help_triggers_exit_after_normalize() {
        let args = normalize_args(["jabberwok", "-?"]);
        let err = Cli::try_parse_from(&args).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);
    }

    #[test]
    fn help_text_contains_expected_content() {
        let text = help_text();
        assert!(
            text.contains("daemon"),
            "help should mention the daemon subcommand"
        );
        assert!(
            text.contains("list-devices"),
            "help should mention the list-devices subcommand"
        );
        assert!(
            text.contains("record"),
            "help should mention the record subcommand"
        );
        assert!(
            text.contains("download-model"),
            "help should mention the download-model subcommand"
        );
        assert!(
            text.contains("permissions"),
            "help should mention the permissions subcommand"
        );
        assert!(
            text.contains("reset"),
            "help should mention the reset subcommand"
        );
        assert!(text.contains("--help"), "help should mention --help flag");
        assert!(
            text.contains("--version"),
            "help should mention --version flag"
        );
    }

    #[test]
    fn type_subcommand_parses_ok() {
        let cli = Cli::try_parse_from(["jabberwok", "type", "hello world"]).expect("parse failed");
        assert_eq!(
            cli.command,
            Some(Commands::Type {
                text: "hello world".to_string()
            })
        );
    }

    #[test]
    fn type_subcommand_requires_text() {
        let result = Cli::try_parse_from(["jabberwok", "type"]);
        assert!(result.is_err());
    }

    #[test]
    fn record_subcommand_parses_with_defaults() {
        let cli = Cli::try_parse_from(["jabberwok", "record"]).expect("parse failed");
        assert_eq!(
            cli.command,
            Some(Commands::Record {
                output: std::path::PathBuf::from("recording.wav"),
                duration: 5,
            })
        );
    }

    #[test]
    fn record_subcommand_accepts_custom_output() {
        let cli = Cli::try_parse_from(["jabberwok", "record", "--output", "out.wav"])
            .expect("parse failed");
        assert_eq!(
            cli.command,
            Some(Commands::Record {
                output: std::path::PathBuf::from("out.wav"),
                duration: 5,
            })
        );
    }

    #[test]
    fn record_subcommand_accepts_custom_duration() {
        let cli =
            Cli::try_parse_from(["jabberwok", "record", "--duration", "30"]).expect("parse failed");
        assert_eq!(
            cli.command,
            Some(Commands::Record {
                output: std::path::PathBuf::from("recording.wav"),
                duration: 30,
            })
        );
    }

    #[test]
    fn record_subcommand_accepts_both_flags() {
        let cli = Cli::try_parse_from([
            "jabberwok",
            "record",
            "--output",
            "clip.wav",
            "--duration",
            "10",
        ])
        .expect("parse failed");
        assert_eq!(
            cli.command,
            Some(Commands::Record {
                output: std::path::PathBuf::from("clip.wav"),
                duration: 10,
            })
        );
    }

    #[test]
    fn record_subcommand_rejects_non_numeric_duration() {
        let result = Cli::try_parse_from(["jabberwok", "record", "--duration", "abc"]);
        assert!(result.is_err());
    }

    #[test]
    fn download_model_parses_name() {
        let cli = Cli::try_parse_from(["jabberwok", "download-model", "parakeet-v3"])
            .expect("parse failed");
        assert_eq!(
            cli.command,
            Some(Commands::DownloadModel {
                model: Some("parakeet-v3".to_string()),
                config: None,
                models_dir: None,
            })
        );
    }

    #[test]
    fn download_model_accepts_custom_dir() {
        let cli = Cli::try_parse_from([
            "jabberwok",
            "download-model",
            "parakeet-v3",
            "--models-dir",
            "/tmp/models",
        ])
        .expect("parse failed");
        assert_eq!(
            cli.command,
            Some(Commands::DownloadModel {
                model: Some("parakeet-v3".to_string()),
                config: None,
                models_dir: Some(std::path::PathBuf::from("/tmp/models")),
            })
        );
    }

    #[test]
    fn download_model_allows_default_name() {
        let cli = Cli::try_parse_from(["jabberwok", "download-model"]).expect("parse failed");
        assert_eq!(
            cli.command,
            Some(Commands::DownloadModel {
                model: None,
                config: None,
                models_dir: None,
            })
        );
    }

    #[test]
    fn version_flag_triggers_exit() {
        let err = Cli::try_parse_from(["jabberwok", "--version"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
    }

    #[test]
    fn select_device_subcommand_parses_with_input() {
        let cli = Cli::try_parse_from(["jabberwok", "select-device", "--input", "USB"])
            .expect("parse failed");
        assert_eq!(
            cli.command,
            Some(Commands::SelectDevice {
                input: Some("USB".to_string()),
                output: None,
                host: None,
                config: None,
            })
        );
    }

    #[test]
    fn select_device_rejects_no_selector() {
        let parsed = Cli::try_parse_from(["jabberwok", "select-device"]).expect("parse failed");
        match parsed.command.unwrap() {
            Commands::SelectDevice {
                input: None,
                output: None,
                host: _,
                config: _,
            } => {}
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn permissions_subcommand_parses_microphone() {
        let cli =
            Cli::try_parse_from(["jabberwok", "permissions", "microphone"]).expect("parse failed");
        assert_eq!(
            cli.command,
            Some(Commands::Permissions {
                target: PermissionTarget::Microphone,
                remove: false,
            })
        );
    }

    #[test]
    fn doctor_subcommand_parses_ok() {
        let cli = Cli::try_parse_from(["jabberwok", "doctor"]).expect("parse failed");
        assert!(matches!(cli.command, Some(Commands::Doctor { .. })));
    }

    #[test]
    fn setup_is_hidden_from_help() {
        let text = help_text();
        assert!(
            !text.contains("\n  setup"),
            "setup command should be hidden from help"
        );
    }
}
