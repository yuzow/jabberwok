mod audio;
mod cli;
mod config;
mod daemon;
mod devices;
mod input;
mod logs;
mod models;
mod os;
mod setup;
mod state;
mod transcribe;

use anyhow::Context;
use clap::Parser;
use cli::{Cli, Commands, LaunchAgentAction, normalize_args};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn main() {
    let raw_args = match config::prepare_process_args(std::env::args()) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let args = normalize_args(raw_args);
    let logging = load_logging_config_for_args(&args);
    let _log_guard = logs::init_logging(&logging);

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        logging_filter = logging.filter_spec(),
        "jabberwok starting"
    );
    if let Err(e) = devices::log_inventory() {
        tracing::warn!(error = %e, "failed to log audio device inventory");
    }

    if let Err(e) = run(args) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run<I, S>(raw_args: I) -> anyhow::Result<()>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = raw_args.into_iter().map(Into::into).collect();
    let cli = match Cli::try_parse_from(&args) {
        Ok(c) => c,
        Err(e) => {
            e.exit();
        }
    };

    match cli.command {
        None => {
            if config::is_bundled_app() {
                let config = resolve_config_path(None)?;
                return start_or_setup_daemon(config);
            }
            print!("{}", cli::help_text());
        }
        Some(Commands::Setup { config }) => {
            let config = resolve_config_path(config.as_ref())?;
            if cfg!(target_os = "macos") {
                crate::os::setup_window(config.clone())?;
            }
            start_or_setup_daemon(config)?;
        }
        Some(Commands::Tutorial { config }) => {
            let config = resolve_config_path(config.as_ref())?;
            let readiness = setup::evaluate_readiness(&config)?;
            if !readiness.can_start_daemon {
                anyhow::bail!(
                    "jabberwok is not ready to start. Run `jabberwok setup` first.\n{}",
                    setup::doctor_text(&config).unwrap_or_default()
                );
            }
            if let Ok(cfg) = crate::config::JabberwokConfig::load(&config) {
                let mc = crate::config::ModelConfig::from(cfg.models);
                if let Some(path) = mc.default_model_path() {
                    return start_daemon_with_model(config, path.to_path_buf(), false, true);
                }
            }
            anyhow::bail!("no default model configured; run `jabberwok download-model` first");
        }
        Some(Commands::Doctor { config }) => {
            let config = resolve_config_path(config.as_ref())?;
            let report = setup::doctor_report(&config)?;
            println!("{}", report.text);
            if !report.readiness.can_start_daemon {
                anyhow::bail!("jabberwok is not ready");
            }
        }
        Some(Commands::Permissions { target, remove }) => {
            if remove {
                println!("{}", setup::remove_permissions(target.command_name())?);
            } else {
                if setup::permission_already_granted(target.command_name())? {
                    println!(
                        "{} permission is already granted.\nNote: for `cargo run`, this may reflect the current host app (Terminal, WezTerm, etc.) rather than the packaged Jabberwok app.",
                        target.command_name()
                    );
                    return Ok(());
                }
                setup::request_permission(target.command_name())?;
                println!(
                    "Requested {} permission flow. If it did not open automatically, use System Settings manually.\nNote: grant permission to the application (Terminal, Wezterm, etc.) that you're currently running in.",
                    target.command_name()
                );
            }
        }
        Some(Commands::LaunchAgent { action }) => match action {
            LaunchAgentAction::Install => {
                os::install_launch_agent()?;
                println!(
                    "Installed the Jabberwok LaunchAgent. It will start automatically when you sign in."
                );
            }
            LaunchAgentAction::Uninstall => {
                os::uninstall_launch_agent()?;
                println!("Removed the Jabberwok LaunchAgent.");
            }
            LaunchAgentAction::Status => {
                if os::is_launch_agent_installed() {
                    println!("Jabberwok LaunchAgent: installed");
                } else {
                    println!("Jabberwok LaunchAgent: not installed");
                }
            }
        },
        Some(Commands::Reset {
            config,
            wait_for_pid,
            relaunch,
        }) => {
            let config = resolve_config_path(config.as_ref())?;
            if let Some(pid) = wait_for_pid {
                state::wait_for_process_exit(pid, std::time::Duration::from_secs(15))?;
            }

            let paths = state::LocalStatePaths::from_config_path(&config)?;
            let removed = setup::reset_local_data(&paths)?;
            println!("{}", setup::describe_reset_result(&paths, &removed));

            if relaunch {
                setup::relaunch_after_reset()?;
            }
        }
        Some(Commands::Daemon {
            model,
            config,
            save_utterances,
        }) => {
            tracing::info!("command: daemon");
            let config = resolve_config_path(config.as_ref())?;
            let ready = setup::evaluate_readiness(&config)?;

            if !ready.microphone_permission_granted {
                anyhow::bail!(
                    "microphone permission required. run `jabberwok permissions microphone`"
                );
            }
            if !ready.accessibility_permission_granted {
                anyhow::bail!(
                    "accessibility permission required. run `jabberwok permissions accessibility`"
                );
            }

            let loaded = crate::config::JabberwokConfig::load(&config)?;
            let hostname = crate::config::current_hostname();
            let prefs = crate::config::device_prefs_for_current_host(&loaded.devices);
            let device_prefs = Arc::new(Mutex::new((*prefs).clone()));
            let model_path = match model {
                Some(p) => p,
                None => {
                    tracing::info!(config = %config.display(), "loading model config");
                    let mc = crate::config::ModelConfig::from(loaded.models.clone());
                    let path = mc
                        .default_model_path()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "no default model; run `download-model` first or pass --model"
                            )
                        })?
                        .to_path_buf();
                    tracing::info!(model_path = %path.display(), "resolved default model");
                    path
                }
            };
            if !model_path.exists() {
                anyhow::bail!(
                    "default model not installed; run `download-model` first or pass --model"
                );
            }

            tracing::info!(save_utterances, "starting daemon");
            daemon::run(
                &model_path,
                save_utterances,
                device_prefs,
                hostname,
                config,
                false,
            )?;
        }
        Some(Commands::ListDevices { config }) => {
            let config = resolve_config_path(config.as_ref())?;
            let config = crate::config::JabberwokConfig::load(&config)?;
            tracing::info!("command: list-devices");
            let hostname = crate::config::current_hostname();
            let from_fallback = !config.devices.hosts.contains_key(&hostname)
                && config.devices.hosts.contains_key("default");
            let prefs = crate::config::device_prefs_for_current_host(&config.devices);
            devices::list_devices(prefs, &hostname, from_fallback)?;
        }
        Some(Commands::SelectDevice {
            input,
            output,
            host,
            config,
        }) => {
            if input.is_none() && output.is_none() {
                anyhow::bail!("select-device requires --input and/or --output");
            }

            let config_path = resolve_config_path(config.as_ref())?;
            let target_host = host.unwrap_or_else(crate::config::current_hostname);
            let (input_sel, output_sel) = devices::select_device(
                input.as_deref(),
                output.as_deref(),
                &target_host,
                &config_path,
            )?;

            if input.is_some() {
                if let Some(name) = input_sel {
                    println!("[{target_host}] Input device set to \"{name}\"");
                } else {
                    println!("[{target_host}] Input device cleared — using OS default");
                }
            }
            if let Some(name) = output_sel {
                println!("[{target_host}] Output device set to \"{name}\"");
            } else if output.is_some() {
                println!("[{target_host}] Output device cleared — using OS default");
            }
        }
        Some(Commands::Record { output, duration }) => {
            tracing::info!(output = %output.display(), duration_secs = duration, "command: record");
            let config_path = resolve_config_path(None)?;
            let config = crate::config::JabberwokConfig::load(&config_path)?;
            let hostname = crate::config::current_hostname();
            let mut prefs = crate::config::device_prefs_for_current_host(&config.devices).clone();
            let device = crate::config::resolve_input_device(&hostname, &mut prefs, &config_path)?;
            audio::record_with_device(&output, std::time::Duration::from_secs(duration), device)?;
        }
        Some(Commands::DownloadModel {
            model,
            config,
            models_dir,
        }) => {
            let config = resolve_config_path(config.as_ref())?;
            let model = resolve_download_model_name(&config, model)?;
            let models_dir = resolve_models_dir(&config, models_dir);
            tracing::info!(model, config = %config.display(), models_dir = %models_dir.display(), "command: download-model");
            let dest = models::download_model_with_cli_progress(&config, &models_dir, &model)?;
            println!("Downloaded {} to {}", model, dest.display());
            println!("Set as default model.");
        }
        Some(Commands::Transcribe {
            model,
            config,
            file,
            duration,
            inject,
            save_utterances,
        }) => {
            tracing::info!(inject, save_utterances, "command: transcribe");
            let config = resolve_config_path(config.as_ref())?;
            let loaded = crate::config::JabberwokConfig::load(&config)?;
            let model_path = match model {
                Some(p) => p,
                None => {
                    tracing::info!(config = %config.display(), "loading model config");
                    let mc = crate::config::ModelConfig::from(loaded.models.clone());
                    let path = mc
                        .default_model_path()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "no default model; run `download-model` first or pass --model"
                            )
                        })?
                        .to_path_buf();
                    tracing::info!(model_path = %path.display(), "resolved default model");
                    path
                }
            };
            let text = match file {
                Some(path) => transcribe::transcribe_file(&model_path, &path)?,
                None => {
                    let tmp = tempfile::Builder::new()
                        .prefix("jabberwok_")
                        .suffix(".wav")
                        .tempfile()
                        .context("failed to create temporary WAV file")?;
                    let hostname = crate::config::current_hostname();
                    let mut prefs =
                        crate::config::device_prefs_for_current_host(&loaded.devices).clone();
                    let device =
                        crate::config::resolve_input_device(&hostname, &mut prefs, &config)?;
                    audio::record_with_device(
                        tmp.path(),
                        std::time::Duration::from_secs(duration),
                        device,
                    )?;
                    let text = transcribe::transcribe_file(&model_path, tmp.path())?;
                    if save_utterances {
                        transcribe::save_utterance(
                            tmp.path(),
                            &text,
                            std::path::Path::new("utterances"),
                        )?;
                    }
                    text
                }
            };
            if inject {
                os::inject_text(text.trim())?;
            } else {
                println!("{}", text.trim());
            }
        }
        Some(Commands::Type { text }) => {
            tracing::info!(len = text.len(), "command: type");
            os::inject_text(&text)?;
        }
    }

    Ok(())
}

fn start_or_setup_daemon(config_path: PathBuf) -> anyhow::Result<()> {
    let readiness = setup::evaluate_readiness(&config_path)?;
    if !readiness.can_start_daemon {
        if let Ok(info) = setup::doctor_text(&config_path) {
            println!("{}", info);
        }

        if cfg!(target_os = "macos") {
            crate::os::setup_window(config_path.clone())?;
            return start_or_setup_daemon(config_path);
        }

        return Ok(());
    }

    if let Ok(cfg) = crate::config::JabberwokConfig::load(&config_path) {
        let mc = crate::config::ModelConfig::from(cfg.models);
        if let Some(path) = mc.default_model_path() {
            return start_daemon_with_model(config_path, path.to_path_buf(), false, false);
        }
    }

    Err(anyhow::anyhow!(
        "no default model configured; run `jabberwok download-model <name>`"
    ))
}

fn start_daemon_with_model(
    config_path: PathBuf,
    model_path: PathBuf,
    save_utterances: bool,
    force_tutorial: bool,
) -> anyhow::Result<()> {
    let cfg = crate::config::JabberwokConfig::load(&config_path)?;
    let show_tutorial = force_tutorial || !cfg.tutorial.has_seen_tutorial;
    let hostname = crate::config::current_hostname();
    let prefs = crate::config::device_prefs_for_current_host(&cfg.devices);
    daemon::run(
        &model_path,
        save_utterances,
        Arc::new(Mutex::new(prefs.clone())),
        hostname,
        config_path,
        show_tutorial,
    )
}

fn resolve_config_path(explicit: Option<&PathBuf>) -> anyhow::Result<PathBuf> {
    match explicit {
        Some(explicit) => Ok(explicit.to_path_buf()),
        None => config::config_file(),
    }
}

fn load_logging_config_for_args(args: &[String]) -> crate::config::LoggingConfig {
    let config_path = extract_config_arg(args)
        .map(Ok)
        .unwrap_or_else(config::config_file);

    let Ok(config_path) = config_path else {
        return crate::config::LoggingConfig::default();
    };

    crate::config::JabberwokConfig::load(&config_path)
        .map(|config| config.logging)
        .unwrap_or_default()
}

fn extract_config_arg(args: &[String]) -> Option<PathBuf> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if let Some(path) = arg.strip_prefix("--config=") {
            return Some(PathBuf::from(path));
        }
        if arg == "--config" {
            return iter.next().map(PathBuf::from);
        }
    }
    None
}

fn resolve_download_model_name(
    config_path: &std::path::Path,
    requested: Option<String>,
) -> anyhow::Result<String> {
    if let Some(requested) = requested {
        return Ok(requested);
    }

    let configured_default = crate::config::ModelConfig::load(config_path)
        .ok()
        .and_then(|config| config.default);

    Ok(configured_default.unwrap_or_else(|| "parakeet-v3".to_string()))
}

fn resolve_models_dir(config_path: &std::path::Path, requested: Option<PathBuf>) -> PathBuf {
    requested.unwrap_or_else(|| setup::models_dir_for_config(config_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_no_subcommand_succeeds() {
        assert!(run(["jabberwok"]).is_ok());
    }

    #[test]
    fn run_reset_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config").join("jabberwok.toml");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, "").unwrap();
        assert!(run(["jabberwok", "reset", "--config", config.to_str().unwrap(),]).is_ok());
    }

    #[test]
    fn run_list_devices_succeeds() {
        assert!(run(["jabberwok", "list-devices"]).is_ok());
    }

    #[test]
    fn run_daemon_no_default_model_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("jabberwok.toml");
        std::fs::write(&config, "").unwrap();
        let err = run(["jabberwok", "daemon", "--config", config.to_str().unwrap()]).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("no default model") || msg.contains("required"),
            "error was: {msg}"
        );
    }

    #[test]
    fn run_download_model_unknown_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("jabberwok.toml");
        std::fs::write(&config, "").unwrap();
        let err = run([
            "jabberwok",
            "download-model",
            "nonexistent",
            "--config",
            config.to_str().unwrap(),
        ])
        .unwrap_err();
        assert!(
            err.to_string().contains("unknown model"),
            "error was: {err}"
        );
    }

    #[test]
    fn run_transcribe_no_default_model_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("jabberwok.toml");
        std::fs::write(&config, "").unwrap();
        let err = run([
            "jabberwok",
            "transcribe",
            "--config",
            config.to_str().unwrap(),
        ])
        .unwrap_err();
        assert!(
            err.to_string().contains("no default model"),
            "error was: {err}"
        );
    }

    #[test]
    fn run_transcribe_explicit_bad_model_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("ghost.bin");
        let wav = dir.path().join("audio.wav");
        std::fs::write(&wav, b"not a real wav").unwrap();
        let err = run([
            "jabberwok",
            "transcribe",
            "--model",
            model.to_str().unwrap(),
            "--file",
            wav.to_str().unwrap(),
        ])
        .unwrap_err();
        assert!(
            err.to_string().contains("model not found"),
            "error was: {err}"
        );
    }

    #[test]
    fn extract_config_arg_supports_split_flag() {
        let args = vec![
            "jabberwok".to_string(),
            "daemon".to_string(),
            "--config".to_string(),
            "/tmp/jabberwok.toml".to_string(),
        ];
        assert_eq!(
            extract_config_arg(&args),
            Some(PathBuf::from("/tmp/jabberwok.toml"))
        );
    }

    #[test]
    fn extract_config_arg_supports_equals_syntax() {
        let args = vec![
            "jabberwok".to_string(),
            "daemon".to_string(),
            "--config=/tmp/jabberwok.toml".to_string(),
        ];
        assert_eq!(
            extract_config_arg(&args),
            Some(PathBuf::from("/tmp/jabberwok.toml"))
        );
    }
}
