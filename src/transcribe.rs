use std::path::Path;

use anyhow::{Context, Result};
use hound::WavReader;
use transcribe_rs::onnx::Quantization;
use transcribe_rs::onnx::parakeet::{ParakeetModel, ParakeetParams};
use transcribe_rs::whisper_cpp::WhisperEngine;
use transcribe_rs::{SpeechModel, TranscribeOptions};

/// Transcribe a 16 kHz mono 16-bit WAV file using the given model.
///
/// If `model_path` is a directory, Parakeet (ONNX) is used.
/// If it is a file, Whisper (whisper.cpp) is used.
pub fn transcribe_file(model_path: &Path, wav_path: &Path) -> Result<String> {
    if !model_path.exists() {
        anyhow::bail!("model not found: {}", model_path.display());
    }
    if wav_is_empty(wav_path)? {
        tracing::warn!(wav = %wav_path.display(), "skipping transcription for empty WAV");
        return Ok(String::new());
    }
    if model_path.is_dir() {
        tracing::info!(model = %model_path.display(), "transcribing with Parakeet (ONNX) engine");
        parakeet(model_path, wav_path)
    } else {
        tracing::info!(model = %model_path.display(), "transcribing with Whisper (whisper.cpp) engine");
        whisper(model_path, wav_path)
    }
}

fn parakeet(model_path: &Path, wav_path: &Path) -> Result<String> {
    tracing::debug!("loading Parakeet model");
    let mut model = ParakeetModel::load(model_path, &Quantization::Int8)
        .map_err(|e| anyhow::anyhow!("failed to load parakeet model: {e}"))?;
    tracing::debug!("Parakeet model loaded; reading WAV samples");

    let samples = transcribe_rs::audio::read_wav_samples(wav_path)
        .map_err(|e| anyhow::anyhow!("failed to read WAV: {e}"))?;
    tracing::debug!(
        sample_count = samples.len(),
        "WAV samples read; running inference"
    );

    let result = model
        .transcribe_with(&samples, &ParakeetParams::default())
        .map_err(|e| anyhow::anyhow!("transcription failed: {e}"))?;

    tracing::debug!(text_len = result.text.len(), "Parakeet inference complete");
    Ok(result.text)
}

fn whisper(model_path: &Path, wav_path: &Path) -> Result<String> {
    tracing::debug!("loading Whisper model");
    let mut engine = WhisperEngine::load(model_path)
        .map_err(|e| anyhow::anyhow!("failed to load whisper model: {e}"))?;
    tracing::debug!("Whisper model loaded; running inference");

    let opts = TranscribeOptions {
        language: None,
        translate: false,
    };

    let result = engine
        .transcribe_file(wav_path, &opts)
        .map_err(|e| anyhow::anyhow!("transcription failed: {e}"))?;

    tracing::debug!(text_len = result.text.len(), "Whisper inference complete");
    Ok(result.text)
}

fn wav_is_empty(wav_path: &Path) -> Result<bool> {
    let reader = WavReader::open(wav_path).context("failed to open WAV for inspection")?;
    Ok(reader.duration() == 0)
}

/// Save a microphone utterance as a WAV + TXT pair in `utterances_dir`.
///
/// The files are named by the current local timestamp (`YYYYMMDDTHHmmss`).
/// TODO: add `--save-utterances` flag (off by default) when needed;
/// for now callers always invoke this after a microphone transcription.
pub fn save_utterance(wav_src: &Path, text: &str, utterances_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(utterances_dir).context("failed to create utterances directory")?;
    let stamp = chrono::Local::now().format("%Y%m%dT%H%M%S").to_string();
    let wav_dest = utterances_dir.join(format!("{stamp}.wav"));
    let txt_dest = utterances_dir.join(format!("{stamp}.txt"));
    std::fs::copy(wav_src, &wav_dest)
        .with_context(|| format!("failed to copy WAV to {}", wav_dest.display()))?;
    std::fs::write(&txt_dest, text.trim())
        .with_context(|| format!("failed to write transcript to {}", txt_dest.display()))?;
    tracing::info!(wav = %wav_dest.display(), txt = %txt_dest.display(), "utterance saved");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn save_utterance_creates_wav_and_txt() {
        let src_dir = tempfile::tempdir().unwrap();
        let wav_src = src_dir.path().join("input.wav");
        std::fs::write(&wav_src, b"RIFF....").unwrap();

        let out_dir = tempfile::tempdir().unwrap();
        let utterances_dir = out_dir.path().join("utterances");

        save_utterance(&wav_src, "  hello world  ", &utterances_dir).unwrap();

        let entries: Vec<_> = std::fs::read_dir(&utterances_dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        let wavs: Vec<_> = entries
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e == "wav"))
            .collect();
        let txts: Vec<_> = entries
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e == "txt"))
            .collect();
        assert_eq!(wavs.len(), 1, "expected one .wav file");
        assert_eq!(txts.len(), 1, "expected one .txt file");

        let wav_bytes = std::fs::read(wavs[0]).unwrap();
        assert_eq!(wav_bytes, b"RIFF....");

        let txt_content = std::fs::read_to_string(txts[0]).unwrap();
        assert_eq!(txt_content, "hello world", "text should be trimmed");
    }

    #[test]
    fn save_utterance_creates_dir_if_absent() {
        let src_dir = tempfile::tempdir().unwrap();
        let wav_src = src_dir.path().join("input.wav");
        std::fs::write(&wav_src, b"data").unwrap();

        let out_dir = tempfile::tempdir().unwrap();
        let utterances_dir = out_dir.path().join("a").join("b").join("utterances");
        assert!(!utterances_dir.exists());

        save_utterance(&wav_src, "text", &utterances_dir).unwrap();
        assert!(utterances_dir.is_dir());
    }

    #[test]
    fn save_utterance_returns_error_when_src_missing() {
        let out_dir = tempfile::tempdir().unwrap();
        let missing_wav = out_dir.path().join("nonexistent.wav");
        let utterances_dir = out_dir.path().join("utterances");

        let err = save_utterance(&missing_wav, "text", &utterances_dir).unwrap_err();
        assert!(
            err.to_string().contains("failed to copy WAV"),
            "error was: {err}"
        );
    }

    #[test]
    fn transcribe_file_nonexistent_model_returns_error() {
        let tmp = tempdir().unwrap();
        let model = tmp.path().join("ghost.bin"); // does not exist
        let wav = tmp.path().join("audio.wav");
        let err = transcribe_file(&model, &wav).unwrap_err();
        assert!(
            err.to_string().contains("model not found"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn transcribe_file_empty_dir_returns_parakeet_error() {
        let tmp = tempdir().unwrap();
        let wav = tmp.path().join("audio.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&wav, spec).unwrap();
        writer.write_sample::<i16>(1).unwrap();
        writer.finalize().unwrap();

        // tmp.path() is_dir() == true → parakeet path
        let err = transcribe_file(tmp.path(), &wav).unwrap_err();
        assert!(
            err.to_string().contains("failed to load parakeet model"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn transcribe_file_empty_wav_returns_empty_text() {
        let tmp = tempdir().unwrap();
        let wav = tmp.path().join("audio.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        hound::WavWriter::create(&wav, spec)
            .unwrap()
            .finalize()
            .unwrap();

        let text = transcribe_file(tmp.path(), &wav).unwrap();
        assert!(text.is_empty(), "expected empty transcript, got: {text:?}");
    }
}
