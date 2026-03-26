use std::path::Path;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc,
};
use std::time::Duration;

use anyhow::{Context, Result};
use audioadapter_buffers::direct::InterleavedSlice;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::{SampleFormat, WavSpec, WavWriter};
use rubato::{Fft, FixedSync, Resampler};
use rustfft::{FftPlanner, num_complex::Complex};

use crate::os::N_BANDS;

static START_SOUND_RAW: &[u8] = include_bytes!("../media/on.raw");
static STOP_SOUND_RAW: &[u8] = include_bytes!("../media/off.raw");
const SOUND_SAMPLE_RATE: u32 = 44_100;
const SOUND_GAIN: f32 = 0.1;

// ---------------------------------------------------------------------------
// FFT-based frequency band analyser
// ---------------------------------------------------------------------------

const FFT_SIZE: usize = 2048;

/// Frequency band edges in Hz.
///
/// All 7 bands sit within the primary speech energy range (80–5 kHz) so every
/// bar reacts noticeably during normal conversation.  Log-spacing is used so
/// each band covers a perceptually equal musical interval.
const BAND_EDGES: [f32; N_BANDS + 1] = [
    80.0, 180.0, 380.0, 750.0, 1_400.0, 2_500.0, 4_000.0, 5_500.0,
];

/// Baseline gain applied to all bands before clamping to [0, 1].
const BAND_GAIN: f32 = 115.5; // 105 × 1.1

/// Per-band gain multiplier.  Speech has a natural high-frequency rolloff
/// (~−6 dB/octave above ~1 kHz), so upper bands need progressively more
/// boost to appear equally animated at the same loudness.
/// Bars 4–6 (indices 3–5) were observed to be too quiet, so their boosts
/// are significantly higher than the natural rolloff alone would suggest.
const PER_BAND_BOOST: [f32; N_BANDS] = [1.0, 1.2, 0.7, 3.5, 10.0, 12.0, 4.2];

/// Holds the FFT planner state and ring buffer between audio callbacks.
struct BandProcessor {
    fft: Arc<dyn rustfft::Fft<f32>>,
    scratch: Vec<Complex<f32>>,
    ring_buffer: Vec<f32>,
    hann: Vec<f32>,
    bin_hz: f32,
    band_levels: Arc<Vec<AtomicU32>>,
}

impl BandProcessor {
    fn new(sample_rate: u32, band_levels: Arc<Vec<AtomicU32>>) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let scratch_len = fft.get_inplace_scratch_len();
        // Hann window to reduce spectral leakage.
        let hann = (0..FFT_SIZE)
            .map(|i| {
                let f = i as f32 / (FFT_SIZE - 1) as f32;
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * f).cos())
            })
            .collect();
        Self {
            fft,
            scratch: vec![Complex::new(0.0, 0.0); scratch_len],
            ring_buffer: Vec::with_capacity(FFT_SIZE * 2),
            hann,
            bin_hz: sample_rate as f32 / FFT_SIZE as f32,
            band_levels,
        }
    }

    /// Accumulate `samples` and run the FFT whenever a full window is ready.
    fn push(&mut self, samples: &[f32]) {
        self.ring_buffer.extend_from_slice(samples);
        while self.ring_buffer.len() >= FFT_SIZE {
            let chunk: Vec<f32> = self.ring_buffer.drain(..FFT_SIZE).collect();
            self.analyse(&chunk);
        }
    }

    fn analyse(&mut self, samples: &[f32]) {
        // Apply Hann window.
        let mut buf: Vec<Complex<f32>> = samples
            .iter()
            .zip(self.hann.iter())
            .map(|(&s, &w)| Complex::new(s * w, 0.0))
            .collect();

        self.fft.process_with_scratch(&mut buf, &mut self.scratch);

        let n_half = FFT_SIZE / 2;

        for (i, atom) in self.band_levels.iter().enumerate() {
            let low = ((BAND_EDGES[i] / self.bin_hz) as usize).clamp(1, n_half);
            let high = ((BAND_EDGES[i + 1] / self.bin_hz) as usize).clamp(1, n_half);
            if high <= low {
                continue;
            }

            // Mean magnitude in the band.
            let sum_sq: f32 = buf[low..high].iter().map(|c| c.norm_sqr()).sum();
            let rms = (sum_sq / (high - low) as f32).sqrt();

            // Normalise by FFT_SIZE and Hann window mean (≈ 0.5).
            let normalised = rms / (FFT_SIZE as f32 * 0.5);
            let level = (normalised * BAND_GAIN * PER_BAND_BOOST[i]).min(1.0);
            atom.store(level.to_bits(), Ordering::Relaxed);
        }
    }
}

const TARGET_RATE: u32 = 16_000;
const RESAMPLE_CHUNK: usize = 1_024;

// Global stop flag installed once; reset to false at the start of each recording.
// This avoids ctrlc::Error::MultipleHandlers when record() is called more than once
// per process (e.g. in tests or future loop modes), while ensuring Ctrl-C always
// interrupts whichever recording is currently running.
static STOP_FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();

fn global_stop() -> Arc<AtomicBool> {
    STOP_FLAG
        .get_or_init(|| {
            let flag = Arc::new(AtomicBool::new(false));
            let flag_clone = Arc::clone(&flag);
            ctrlc::set_handler(move || {
                eprintln!("\nrecording stopped by user");
                flag_clone.store(true, Ordering::Relaxed);
            })
            .expect("failed to install Ctrl-C handler");
            flag
        })
        .clone()
}

pub fn record_with_device(
    output: &Path,
    duration: Duration,
    input_device: cpal::Device,
) -> Result<()> {
    tracing::info!(output = %output.display(), duration_secs = duration.as_secs(), "starting timed recording");
    let stop = global_stop();
    stop.store(false, Ordering::Relaxed);

    // Timer fires after `duration`
    {
        let stop_timer = Arc::clone(&stop);
        std::thread::spawn(move || {
            std::thread::sleep(duration);
            stop_timer.store(true, Ordering::Relaxed);
        });
    }

    eprintln!("recording… press Ctrl-C to stop early");
    record_inner(output, stop, input_device, None)
}

/// Record until `stop` is set externally — no duration limit.  Used by the
/// push-to-talk daemon so that the caller controls when recording ends.
///
/// `band_levels` is updated in real time with per-frequency-band RMS energy
/// (7 bands, stored as `f32` bits in `AtomicU32`) so the UI overlay can
/// animate a frequency graph while the user speaks.
pub fn record_ptt(
    output: &Path,
    stop: Arc<AtomicBool>,
    band_levels: Arc<Vec<AtomicU32>>,
    input_device: cpal::Device,
) -> Result<()> {
    record_inner(output, stop, input_device, Some(band_levels))
}

pub fn play_start_sound(output_device: Option<cpal::Device>) {
    play_sound(output_device, start_samples());
}

pub fn play_stop_sound(output_device: Option<cpal::Device>) {
    play_sound(output_device, stop_samples());
}

pub fn request_microphone_access() -> Result<()> {
    let host = cpal::default_host();
    let input_device = host
        .default_input_device()
        .context("no default input device available")?;
    let config = input_device
        .default_input_config()
        .context("failed to get default input config")?;

    let err_fn = |e: cpal::StreamError| tracing::error!("stream error: {e}");
    let stream_config: cpal::StreamConfig = config.clone().into();

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => input_device.build_input_stream(
            &stream_config,
            move |_data: &[f32], _| {},
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => input_device.build_input_stream(
            &stream_config,
            move |_data: &[i16], _| {},
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I32 => input_device.build_input_stream(
            &stream_config,
            move |_data: &[i32], _| {},
            err_fn,
            None,
        )?,
        fmt => anyhow::bail!("unsupported sample format: {fmt:?}"),
    };

    stream.play()?;
    std::thread::sleep(Duration::from_millis(300));
    drop(stream);
    Ok(())
}

fn record_inner(
    output: &Path,
    stop: Arc<AtomicBool>,
    input_device: cpal::Device,
    band_levels: Option<Arc<Vec<AtomicU32>>>,
) -> Result<()> {
    let config = input_device
        .default_input_config()
        .context("failed to get default input config")?;

    let native_rate = config.sample_rate();
    let native_channels = config.channels() as usize;
    let sample_fmt = config.sample_format();

    tracing::info!(
        device = %input_device.description()?.name().to_owned(),
        native_rate,
        native_channels,
        ?sample_fmt,
        "starting recording"
    );

    let (tx, rx) = mpsc::channel::<Vec<f32>>();

    let err_fn = |e: cpal::StreamError| tracing::error!("stream error: {e}");
    let stream_config: cpal::StreamConfig = config.into();

    // One BandProcessor moved into whichever match arm actually executes
    // (Rust understands match arms are mutually exclusive, so this is valid).
    let sample_rate_hz: u32 = native_rate;
    let mut proc = band_levels.map(|bl| BandProcessor::new(sample_rate_hz, bl));

    let stream = match sample_fmt {
        cpal::SampleFormat::F32 => input_device.build_input_stream(
            &stream_config,
            move |data: &[f32], _| {
                if let Some(ref mut p) = proc {
                    p.push(data);
                }
                let _ = tx.send(downmix(data, native_channels));
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => input_device.build_input_stream(
            &stream_config,
            move |data: &[i16], _| {
                let floats: Vec<f32> = data.iter().map(|&s| s as f32 / 32_768.0).collect();
                if let Some(ref mut p) = proc {
                    p.push(&floats);
                }
                let _ = tx.send(downmix(&floats, native_channels));
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I32 => input_device.build_input_stream(
            &stream_config,
            move |data: &[i32], _| {
                let floats: Vec<f32> = data.iter().map(|&s| s as f32 / 2_147_483_648.0).collect();
                if let Some(ref mut p) = proc {
                    p.push(&floats);
                }
                let _ = tx.send(downmix(&floats, native_channels));
            },
            err_fn,
            None,
        )?,
        fmt => anyhow::bail!("unsupported sample format: {fmt:?}"),
    };

    stream.play()?;

    let mut samples: Vec<f32> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        while let Ok(chunk) = rx.try_recv() {
            samples.extend_from_slice(&chunk);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // Drain any samples buffered after the stop flag was set
    while let Ok(chunk) = rx.try_recv() {
        samples.extend_from_slice(&chunk);
    }

    drop(stream);
    tracing::info!(sample_count = samples.len(), "capture complete");
    if samples.is_empty() {
        tracing::warn!("recording captured no samples");
    }

    let resampled = if native_rate != TARGET_RATE {
        tracing::debug!(
            from_rate = native_rate,
            to_rate = TARGET_RATE,
            input_samples = samples.len(),
            "resampling audio"
        );
        let out = resample(samples, native_rate)?;
        tracing::debug!(output_samples = out.len(), "resample complete");
        out
    } else {
        tracing::debug!(
            rate = native_rate,
            "native rate matches target; skipping resample"
        );
        samples
    };

    let spec = WavSpec {
        channels: 1,
        sample_rate: TARGET_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    tracing::debug!(output = %output.display(), sample_count = resampled.len(), sample_rate = TARGET_RATE, "writing WAV file");
    let mut writer = WavWriter::create(output, spec).context("failed to create WAV file")?;
    for &s in &resampled {
        let pcm = (s * 32_767.0).clamp(-32_768.0, 32_767.0) as i16;
        writer.write_sample(pcm)?;
    }
    writer.finalize()?;
    tracing::info!(output = %output.display(), sample_count = resampled.len(), sample_rate = TARGET_RATE, "WAV file written");
    Ok(())
}

fn downmix(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels == 1 {
        return samples.to_vec();
    }
    samples
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

fn decode_raw(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]) * SOUND_GAIN)
        .collect()
}

fn start_samples() -> &'static [f32] {
    static CACHE: OnceLock<Vec<f32>> = OnceLock::new();
    CACHE.get_or_init(|| decode_raw(START_SOUND_RAW))
}

fn stop_samples() -> &'static [f32] {
    static CACHE: OnceLock<Vec<f32>> = OnceLock::new();
    CACHE.get_or_init(|| decode_raw(STOP_SOUND_RAW))
}

fn play_sound(output_device: Option<cpal::Device>, samples: &'static [f32]) {
    std::thread::spawn(move || play_sound_blocking(output_device, samples));
}

fn play_sound_blocking(output_device: Option<cpal::Device>, samples: &'static [f32]) {
    let host = cpal::default_host();
    let Some(device) = output_device.or_else(|| host.default_output_device()) else {
        tracing::warn!("no output device available for start/stop sound");
        return;
    };

    let stream_config = cpal::StreamConfig {
        channels: 1,
        sample_rate: SOUND_SAMPLE_RATE,
        buffer_size: cpal::BufferSize::Default,
    };

    let mut pos = 0usize;
    let err_fn = |error: cpal::StreamError| {
        tracing::error!(error = %error, "output stream error while playing sound");
    };
    let stream = device.build_output_stream(
        &stream_config,
        move |data: &mut [f32], _| {
            for out in data.iter_mut() {
                *out = if pos < samples.len() {
                    let sample = samples[pos];
                    pos += 1;
                    sample
                } else {
                    0.0
                };
            }
        },
        err_fn,
        None,
    );

    let Ok(stream) = stream else {
        tracing::warn!("failed to build output stream for sound");
        return;
    };

    if let Err(error) = stream.play() {
        tracing::warn!(error = %error, "failed to play sound");
        return;
    }

    let duration = Duration::from_secs_f32(samples.len() as f32 / SOUND_SAMPLE_RATE as f32);
    std::thread::sleep(duration);
}

fn resample(samples: Vec<f32>, from_rate: u32) -> Result<Vec<f32>> {
    let mut resampler = Fft::<f32>::new(
        from_rate as usize,
        TARGET_RATE as usize,
        RESAMPLE_CHUNK,
        2,
        1,
        FixedSync::Input,
    )?;

    // Pad input to a multiple of RESAMPLE_CHUNK
    let mut padded = samples;
    let rem = padded.len() % RESAMPLE_CHUNK;
    if rem != 0 {
        padded.resize(padded.len() + (RESAMPLE_CHUNK - rem), 0.0);
    }

    let mut output = Vec::new();
    for chunk in padded.chunks(RESAMPLE_CHUNK) {
        let adapter = InterleavedSlice::new(chunk, 1, chunk.len())
            .map_err(|e| anyhow::anyhow!("resampler buffer error: {e:?}"))?;
        let out = resampler.process(&adapter, 0, None)?;
        output.extend_from_slice(&out.take_data());
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cpal::traits::HostTrait;

    // ── downmix ────────────────────────────────────────────────────────────

    #[test]
    fn downmix_mono_passthrough() {
        let samples = vec![0.1f32, 0.2, 0.3, 0.4];
        assert_eq!(downmix(&samples, 1), samples);
    }

    #[test]
    fn downmix_stereo_averages_pairs() {
        // Two frames: [0.0, 1.0] and [−0.5, 0.5] → [0.5, 0.0]
        let samples = vec![0.0f32, 1.0, -0.5, 0.5];
        let result = downmix(&samples, 2);
        assert_eq!(result.len(), 2);
        assert!((result[0] - 0.5).abs() < 1e-6, "first frame: {}", result[0]);
        assert!(
            (result[1] - 0.0).abs() < 1e-6,
            "second frame: {}",
            result[1]
        );
    }

    #[test]
    fn downmix_quad_averages_four_channels() {
        // One frame: [1.0, 0.0, 0.0, 0.0] → 0.25
        let samples = vec![1.0f32, 0.0, 0.0, 0.0];
        let result = downmix(&samples, 4);
        assert_eq!(result.len(), 1);
        assert!((result[0] - 0.25).abs() < 1e-6, "frame: {}", result[0]);
    }

    #[test]
    fn downmix_empty_input_returns_empty() {
        assert!(downmix(&[], 2).is_empty());
    }

    // ── resample ───────────────────────────────────────────────────────────

    // Acceptable output length: within 10% of the theoretical target.
    // The small overshoot comes from silence-padding the last chunk.
    fn assert_len_near(got: usize, expected: usize) {
        let tolerance = expected / 10;
        assert!(
            got >= expected.saturating_sub(tolerance) && got <= expected + tolerance,
            "expected ~{expected} samples (±10%), got {got}"
        );
    }

    #[test]
    fn resample_48k_to_16k_output_length() {
        let samples = vec![0.0f32; 48_000]; // 1 second at 48 kHz
        let result = resample(samples, 48_000).unwrap();
        assert_len_near(result.len(), TARGET_RATE as usize);
    }

    #[test]
    fn resample_44100_to_16k_output_length() {
        let samples = vec![0.0f32; 44_100]; // 1 second at 44.1 kHz
        let result = resample(samples, 44_100).unwrap();
        assert_len_near(result.len(), TARGET_RATE as usize);
    }

    #[test]
    fn resample_output_is_non_empty_for_nonempty_input() {
        let samples = vec![0.0f32; RESAMPLE_CHUNK]; // one chunk
        let result = resample(samples, 48_000).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn resample_already_at_target_rate_succeeds() {
        let samples = vec![0.0f32; TARGET_RATE as usize];
        let result = resample(samples, TARGET_RATE).unwrap();
        assert!(!result.is_empty());
    }

    // ── record (requires hardware + a working default input device) ────────

    #[test]
    #[ignore = "requires audio hardware; run with: cargo test record -- --ignored"]
    fn record_creates_wav_with_correct_spec() {
        let path = std::env::temp_dir().join("jabberwok_test_record.wav");
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .expect("requires a default input device");
        record_with_device(&path, Duration::from_secs(1), device).unwrap();

        let reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.sample_rate, TARGET_RATE);
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(spec.sample_format, hound::SampleFormat::Int);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    #[ignore = "requires audio hardware; run with: cargo test record_with_device -- --ignored"]
    fn record_with_device_uses_specified_device() {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .expect("requires a default input device");
        let path = std::env::temp_dir().join("jabberwok_test_record_device.wav");
        record_with_device(&path, Duration::from_secs(1), device).unwrap();

        let reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.sample_rate, TARGET_RATE);
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(spec.sample_format, hound::SampleFormat::Int);

        let _ = std::fs::remove_file(path);
    }
}
