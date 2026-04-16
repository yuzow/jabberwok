use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc,
};
use std::time::Duration;

use anyhow::{Context, Result};
use fs2::FileExt;

use crate::{config, os, transcribe};

enum Msg {
    PttDown,
    PttUp,
}

struct DaemonLock {
    #[allow(dead_code)]
    file: File,
}

impl DaemonLock {
    fn acquire() -> Result<Self> {
        let lock_path = daemon_lock_path()?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("failed to open daemon lock file {}", lock_path.display()))?;

        file.try_lock_exclusive().map_err(|_| {
            anyhow::anyhow!(
                "jabberwok daemon is already running.\nlock file: {}\nIf no daemon is running, remove that file and try again.",
                lock_path.display()
            )
        })?;

        Ok(Self { file })
    }
}

impl Drop for DaemonLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Run the push-to-talk daemon.
///
/// Spawns the key-listener and recording/transcription logic on background
/// threads, then calls [`crate::os::run_overlay`] which blocks on the active
/// platform's overlay/event-loop implementation.
///
/// Requires Accessibility permission (System Settings → Privacy & Security →
/// Accessibility).
pub fn run(
    model_path: &Path,
    save_utterances: bool,
    device_prefs: Arc<Mutex<crate::config::DevicePrefs>>,
    hostname: String,
    config_path: PathBuf,
    show_tutorial: bool,
) -> Result<()> {
    tracing::info!("checking daemon runtime support");
    os::ensure_daemon_runtime_support()?;
    tracing::info!("checking accessibility permissions");
    os::ensure_accessibility_permission_for_daemon()?;
    let _lock = DaemonLock::acquire()?;

    // True while the PTT key is held — the overlay uses this for show/hide.
    let recording = Arc::new(AtomicBool::new(false));
    // One AtomicU32 (f32 bits) per frequency band, used only for animation.
    let band_levels: Arc<Vec<AtomicU32>> = Arc::new(
        (0..os::N_BANDS)
            .map(|_| AtomicU32::new(0_f32.to_bits()))
            .collect(),
    );

    let recording_bg = Arc::clone(&recording);
    let band_levels_bg = Arc::clone(&band_levels);
    let model_path = model_path.to_path_buf();
    let device_prefs_bg = Arc::clone(&device_prefs);
    let config_path_bg = config_path.clone();
    let inventory = crate::devices::inventory()?;

    tracing::info!("spawning daemon loop thread");
    // Spawn the daemon loop onto a background thread so the main thread is
    // free to run the Cocoa event loop (required by macOS for UI).
    std::thread::spawn(move || {
        if let Err(e) = daemon_loop(
            &model_path,
            save_utterances,
            recording_bg,
            band_levels_bg,
            device_prefs_bg,
            hostname,
            config_path_bg,
        ) {
            tracing::error!(error = %e, "daemon loop exited with error");
        }
    });

    // Show the tutorial window while the daemon loop is already running in the
    // background. The text boxes in the tutorial receive injected text from the
    // daemon naturally — no special wiring needed. The tutorial blocks this
    // thread until the user dismisses it, then we proceed to the overlay.
    if show_tutorial && let Err(e) = os::tutorial_window(config_path.clone()) {
        tracing::warn!(error = %e, "tutorial window failed; continuing to overlay");
    }

    os::run_overlay(
        recording,
        band_levels,
        inventory,
        Arc::clone(&device_prefs),
        config_path,
    );

    Ok(())
}

fn daemon_lock_path() -> Result<PathBuf> {
    let base = config::logs_dir()?;
    std::fs::create_dir_all(&base)
        .with_context(|| format!("failed to create {}", base.display()))?;
    Ok(base.join("daemon.lock"))
}

fn daemon_loop(
    model_path: &Path,
    save_utterances: bool,
    recording: Arc<AtomicBool>,
    band_levels: Arc<Vec<AtomicU32>>,
    device_prefs: Arc<Mutex<crate::config::DevicePrefs>>,
    hostname: String,
    config_path: PathBuf,
) -> Result<()> {
    let (tx, rx) = mpsc::channel::<Msg>();
    let (ptt_tx, ptt_rx) = mpsc::channel::<bool>();
    let ptt_key_label = os::default_push_to_talk_key_label();
    tracing::debug!(ptt_key_label, "configured push-to-talk key");

    std::thread::spawn(move || {
        for pressed in ptt_rx {
            let msg = if pressed { Msg::PttDown } else { Msg::PttUp };
            let _ = tx.send(msg);
        }
    });

    // The platform listener blocks its thread, so run it on a dedicated thread.
    std::thread::spawn(move || {
        loop {
            let tx = ptt_tx.clone();
            match os::listen_for_ptt_events(tx) {
                Ok(()) => {
                    tracing::warn!("global key listener stopped unexpectedly; retrying");
                }
                Err(error) => {
                    tracing::error!(?error, "failed to start global key listener; retrying");
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });

    tracing::info!(ptt_key_label, "daemon ready");
    eprintln!("Hold {ptt_key_label} to record, release to transcribe.");

    // Tracks an in-progress recording: (temp WAV file, stop flag, thread handle).
    let mut active: Option<(
        tempfile::NamedTempFile,
        Arc<AtomicBool>,
        std::thread::JoinHandle<Result<()>>,
    )> = None;

    for msg in rx {
        match msg {
            Msg::PttDown => {
                if active.is_none() {
                    let tmp = tempfile::Builder::new()
                        .prefix("jabberwok_")
                        .suffix(".wav")
                        .tempfile()
                        .context("failed to create temporary WAV file")?;
                    let stop = Arc::new(AtomicBool::new(false));
                    let wav_path = tmp.path().to_path_buf();
                    let stop_clone = Arc::clone(&stop);
                    let bands_clone = Arc::clone(&band_levels);
                    let device = {
                        let mut prefs = device_prefs
                            .lock()
                            .map_err(|_| anyhow::anyhow!("device preferences lock is poisoned"))?;
                        crate::config::resolve_input_device(&hostname, &mut prefs, &config_path)?
                    };
                    let output_device = match device_prefs
                        .lock()
                        .map_err(|_| anyhow::anyhow!("device preferences lock is poisoned"))
                        .and_then(|mut prefs| {
                            crate::config::resolve_output_device(
                                &hostname,
                                &mut prefs,
                                &config_path,
                            )
                            .map_err(|e| anyhow::anyhow!(e))
                        }) {
                        Ok(device) => Some(device),
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "configured output device unavailable, falling back to system default for startup sound"
                            );
                            None
                        }
                    };

                    os::play_start_sound(output_device);
                    tracing::debug!(wav_path = %wav_path.display(), "created temp WAV file for recording");
                    let handle = std::thread::spawn(move || {
                        crate::audio::record_ptt(&wav_path, stop_clone, bands_clone, device)
                    });
                    active = Some((tmp, stop, handle));
                    recording.store(true, Ordering::Relaxed);
                    tracing::info!("PTT key down {ptt_key_label} — recording started");
                    eprintln!("recording…");
                }
            }
            Msg::PttUp => {
                if let Some((tmp, stop, handle)) = active.take() {
                    tracing::info!("PTT key up {ptt_key_label} — stopping recording");
                    recording.store(false, Ordering::Relaxed);
                    let output_device = match device_prefs
                        .lock()
                        .map_err(|_| anyhow::anyhow!("device preferences lock is poisoned"))
                        .and_then(|mut prefs| {
                            crate::config::resolve_output_device(
                                &hostname,
                                &mut prefs,
                                &config_path,
                            )
                            .map_err(|e| anyhow::anyhow!(e))
                        }) {
                        Ok(device) => Some(device),
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "configured output device unavailable, falling back to system default for stop sound"
                            );
                            None
                        }
                    };
                    os::play_stop_sound(output_device);
                    stop.store(true, Ordering::Relaxed);
                    // Zero out all bands so the animation resets.
                    for band in band_levels.iter() {
                        band.store(0_f32.to_bits(), Ordering::Relaxed);
                    }

                    match handle.join() {
                        Err(_) => {
                            tracing::error!("recording thread panicked");
                            continue;
                        }
                        Ok(Err(e)) => {
                            tracing::error!(error = %e, "recording failed");
                            continue;
                        }
                        Ok(Ok(())) => {
                            tracing::debug!("recording thread finished successfully");
                        }
                    }

                    eprintln!("transcribing…");
                    tracing::info!(wav = %tmp.path().display(), "starting transcription");
                    match transcribe::transcribe_file(model_path, tmp.path()) {
                        Err(e) => {
                            eprintln!("transcription error: {e}");
                            tracing::error!(error = %e, "transcription failed");
                        }
                        Ok(text) => {
                            let trimmed = text.trim().to_string();
                            if trimmed.is_empty() {
                                tracing::warn!(
                                    "empty capture or empty transcript; skipping text injection"
                                );
                                eprintln!("→");
                                continue;
                            }
                            tracing::info!(len = trimmed.len(), "transcription complete");
                            tracing::info!(text = %trimmed, "transcription result");
                            eprintln!("→ {trimmed}");
                            if save_utterances {
                                let _ = transcribe::save_utterance(
                                    tmp.path(),
                                    &trimmed,
                                    Path::new("utterances"),
                                );
                            }
                            if let Err(e) = os::inject_text(&trimmed) {
                                eprintln!("inject error: {e}");
                                tracing::error!(error = %e, "text injection failed");
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquiring_daemon_lock_twice_fails() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("daemon.lock");

        let first = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        first.try_lock_exclusive().unwrap();

        let second = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        assert!(second.try_lock_exclusive().is_err());
    }
}
