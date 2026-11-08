//! OWNER: output agent. Speaker output and mouth levels (plan task O3).
//!
//! `CpalOut` opens the default output device lazily, inside `play`, never in a constructor.
//! Samples stay in memory; nothing here writes or logs audio.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{AudioClip, AudioError};

/// One mouth-level window (~30 per second).
pub const LEVEL_WINDOW_SECS: f32 = 0.033;
/// Release time of the level smoother: the mouth closes over ~120 ms.
const RELEASE_SECS: f32 = 0.120;
/// Rising levels move this fraction of the way per window (fast attack).
const ATTACK: f32 = 0.7;
/// Speech RMS after Piper's peak normalisation is ~0.05–0.3; this maps it onto 0–1.
const LEVEL_GAIN: f32 = 3.5;

pub trait AudioOut: Send + Sync {
    /// Play `clip` at `volume` (0–1). Blocks until done or `stop`ped. Calls `on_level` with a
    /// smoothed 0–1 loudness about every 33 ms while playing.
    fn play(
        &self,
        clip: AudioClip,
        volume: f32,
        on_level: Box<dyn Fn(f32) + Send>,
    ) -> Result<(), AudioError>;
    /// Stop whatever is playing now. Takes effect within ~10 ms.
    fn stop(&self);
}

/// Smoothed RMS loudness per `LEVEL_WINDOW_SECS` window, 0–1, for mouth movement.
/// Independent of volume, so the mouth still moves when speech is quiet.
pub fn level_envelope(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    if sample_rate == 0 || samples.is_empty() {
        return Vec::new();
    }
    let win = ((sample_rate as f32 * LEVEL_WINDOW_SECS).round() as usize).max(1);
    let release = (-LEVEL_WINDOW_SECS / RELEASE_SECS).exp();
    let mut level = 0.0f32;
    samples
        .chunks(win)
        .map(|chunk| {
            let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
            let target = (rms * LEVEL_GAIN).clamp(0.0, 1.0);
            level = if target > level {
                level + (target - level) * ATTACK
            } else {
                level * release + target * (1.0 - release)
            };
            level
        })
        .collect()
}

/// Apply output volume (clamped to 0–1).
pub fn scaled(samples: &[f32], volume: f32) -> Vec<f32> {
    let v = if volume.is_finite() {
        volume.clamp(0.0, 1.0)
    } else {
        1.0
    };
    samples.iter().map(|s| s * v).collect()
}

/// Linear-interpolation resample (speech only, so quality is fine).
pub fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || from == 0 || to == 0 || samples.is_empty() {
        return samples.to_vec();
    }
    let out_len = (samples.len() as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * step;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = samples[idx.min(samples.len() - 1)];
            let b = samples[(idx + 1).min(samples.len() - 1)];
            a + (b - a) * frac
        })
        .collect()
}

/// Feed `on_level` from `envelope` as `elapsed` advances; returns the next window index.
fn report_levels(
    envelope: &[f32],
    next: usize,
    elapsed_secs: f32,
    on_level: &dyn Fn(f32),
) -> usize {
    let due = ((elapsed_secs / LEVEL_WINDOW_SECS) as usize + 1).min(envelope.len());
    for &l in envelope.iter().take(due).skip(next) {
        on_level(l);
    }
    due.max(next)
}

// --- Real device ----------------------------------------------------------------------------

/// The system's default output device via cpal. Opened per clip, on the calling thread.
#[derive(Default)]
pub struct CpalOut {
    /// Bumped by `stop`; a `play` that sees a change returns early.
    stops: AtomicU64,
}

impl CpalOut {
    pub fn new() -> Self {
        Self::default()
    }
}

impl AudioOut for CpalOut {
    fn play(
        &self,
        clip: AudioClip,
        volume: f32,
        on_level: Box<dyn Fn(f32) + Send>,
    ) -> Result<(), AudioError> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let started = self.stops.load(Ordering::SeqCst);
        if clip.samples.is_empty() {
            return Ok(());
        }
        let envelope = level_envelope(&clip.samples, clip.sample_rate);
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(AudioError::NoOutputDevice)?;
        let supported = device
            .default_output_config()
            .map_err(|e| AudioError::Device(e.to_string()))?;
        let config = supported.config();
        let channels = config.channels.max(1) as usize;
        let data: Arc<Vec<f32>> = Arc::new(resample(
            &scaled(&clip.samples, volume),
            clip.sample_rate,
            config.sample_rate,
        ));
        let total = data.len();
        let pos = Arc::new(AtomicUsize::new(0));

        fn build<T>(
            device: &cpal::Device,
            config: cpal::StreamConfig,
            channels: usize,
            data: Arc<Vec<f32>>,
            pos: Arc<AtomicUsize>,
        ) -> Result<cpal::Stream, AudioError>
        where
            T: cpal::SizedSample + cpal::FromSample<f32>,
        {
            device
                .build_output_stream::<T, _, _>(
                    config,
                    move |out: &mut [T], _| {
                        let mut p = pos.load(Ordering::Relaxed);
                        for frame in out.chunks_mut(channels) {
                            let s = data.get(p).copied().unwrap_or(0.0);
                            p += 1;
                            for o in frame {
                                *o = T::from_sample(s);
                            }
                        }
                        pos.store(p, Ordering::Relaxed);
                    },
                    |e| tracing::warn!("speaker stream error: {e}"),
                    None,
                )
                .map_err(|e| AudioError::Device(e.to_string()))
        }

        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, config, channels, data, pos.clone()),
            cpal::SampleFormat::I16 => build::<i16>(&device, config, channels, data, pos.clone()),
            cpal::SampleFormat::U16 => build::<u16>(&device, config, channels, data, pos.clone()),
            cpal::SampleFormat::I32 => build::<i32>(&device, config, channels, data, pos.clone()),
            other => Err(AudioError::Device(format!(
                "unsupported sample format {other}"
            ))),
        }?;
        stream
            .play()
            .map_err(|e| AudioError::Device(e.to_string()))?;

        let rate = config.sample_rate.max(1) as f32;
        let mut next = 0;
        let deadline = Instant::now() + Duration::from_secs_f32(total as f32 / rate + 2.0);
        loop {
            if self.stops.load(Ordering::SeqCst) != started {
                break;
            }
            let played = pos.load(Ordering::Relaxed);
            next = report_levels(&envelope, next, played as f32 / rate, &*on_level);
            // Let the device drain its last buffer before closing.
            if played >= total + (rate as usize / 20) || Instant::now() > deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(stream);
        on_level(0.0);
        Ok(())
    }

    fn stop(&self) {
        self.stops.fetch_add(1, Ordering::SeqCst);
    }
}

// --- Test double ----------------------------------------------------------------------------

/// Records clips and simulates playback time (real time × `time_scale`), emitting levels.
#[cfg(test)]
pub struct FakeOut {
    pub played: std::sync::Mutex<Vec<(AudioClip, f32)>>,
    /// Clips that were cut short by `stop`.
    pub interrupted: AtomicUsize,
    stops: AtomicU64,
    time_scale: f32,
}

#[cfg(test)]
impl FakeOut {
    pub fn new(time_scale: f32) -> Self {
        Self {
            played: std::sync::Mutex::new(Vec::new()),
            interrupted: AtomicUsize::new(0),
            stops: AtomicU64::new(0),
            time_scale,
        }
    }

    pub fn played_count(&self) -> usize {
        self.played.lock().map(|g| g.len()).unwrap_or(0)
    }
}

#[cfg(test)]
impl AudioOut for FakeOut {
    fn play(
        &self,
        clip: AudioClip,
        volume: f32,
        on_level: Box<dyn Fn(f32) + Send>,
    ) -> Result<(), AudioError> {
        let started = self.stops.load(Ordering::SeqCst);
        let envelope = level_envelope(&clip.samples, clip.sample_rate);
        let duration = clip.duration_secs() * self.time_scale;
        if let Ok(mut g) = self.played.lock() {
            g.push((clip, volume));
        }
        let t0 = Instant::now();
        let mut next = 0;
        loop {
            if self.stops.load(Ordering::SeqCst) != started {
                self.interrupted.fetch_add(1, Ordering::SeqCst);
                break;
            }
            let elapsed = t0.elapsed().as_secs_f32();
            let scale = self.time_scale.max(1e-6);
            next = report_levels(&envelope, next, elapsed / scale, &*on_level);
            if elapsed >= duration {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        on_level(0.0);
        Ok(())
    }

    fn stop(&self) {
        self.stops.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
pub(crate) fn tone(secs: f32, amp: f32, rate: u32) -> AudioClip {
    let n = (secs * rate as f32) as usize;
    AudioClip {
        samples: (0..n)
            .map(|i| amp * (i as f32 * 220.0 * std::f32::consts::TAU / rate as f32).sin())
            .collect(),
        sample_rate: rate,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn levels_follow_envelope() {
        // 0.3 s loud, 0.3 s silent.
        let mut clip = tone(0.3, 0.3, 22_050);
        clip.samples.extend(std::iter::repeat_n(0.0, 6_615));
        let env = level_envelope(&clip.samples, clip.sample_rate);
        assert!((17..=19).contains(&env.len()), "{} windows", env.len());
        assert!(env.iter().all(|l| (0.0..=1.0).contains(l)));
        // Fast attack: loud within two windows.
        assert!(env[1] > 0.5, "attack {}", env[1]);
        let peak = env[..9].iter().cloned().fold(0.0, f32::max);
        // Release ~120 ms: still partly open one window into silence, near closed after 300 ms.
        assert!(env[10] > 0.2 && env[10] < peak);
        assert!(*env.last().unwrap_or(&1.0) < 0.1, "tail {:?}", env.last());
        // Silence is zero.
        assert!(level_envelope(&[0.0; 2000], 22_050)
            .iter()
            .all(|l| *l == 0.0));
    }

    #[test]
    fn stop_interrupts_play() {
        let out = Arc::new(FakeOut::new(1.0));
        let o = out.clone();
        let t0 = Instant::now();
        let h = std::thread::spawn(move || o.play(tone(2.0, 0.3, 16_000), 1.0, Box::new(|_| {})));
        std::thread::sleep(Duration::from_millis(50));
        out.stop();
        assert!(h.join().ok().is_some_and(|r| r.is_ok()));
        assert!(
            t0.elapsed() < Duration::from_millis(500),
            "{:?}",
            t0.elapsed()
        );
        assert_eq!(out.interrupted.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn volume_scales() {
        let s = scaled(&[0.5, -1.0, 0.25], 0.5);
        assert_eq!(s, vec![0.25, -0.5, 0.125]);
        assert_eq!(scaled(&[0.5], 3.0), vec![0.5]); // clamped to 1
        assert_eq!(scaled(&[0.5], -1.0), vec![0.0]);
        // Mouth levels don't depend on volume.
        let clip = tone(0.2, 0.3, 22_050);
        assert_eq!(
            level_envelope(&clip.samples, 22_050),
            level_envelope(&clip.samples, 22_050)
        );
        // The fake records the volume it was asked for.
        let out = FakeOut::new(0.0);
        let levels = Arc::new(Mutex::new(Vec::new()));
        let l = levels.clone();
        let r = out.play(
            tone(0.1, 0.3, 22_050),
            0.4,
            Box::new(move |v| {
                if let Ok(mut g) = l.lock() {
                    g.push(v);
                }
            }),
        );
        assert!(r.is_ok());
        let played = out.played.lock().map(|g| g[0].1).unwrap_or(0.0);
        assert!((played - 0.4).abs() < 1e-6);
        assert_eq!(
            levels.lock().map(|g| g.last().copied()).ok().flatten(),
            Some(0.0)
        );
    }

    #[test]
    fn resample_lengths() {
        assert_eq!(resample(&vec![0.0; 22_050], 22_050, 48_000).len(), 48_000);
        assert_eq!(resample(&[0.1, 0.2], 16_000, 16_000), vec![0.1, 0.2]);
        assert!(resample(&[], 22_050, 48_000).is_empty());
    }

    /// Plays a short tone on the real default speaker. Needs a speaker; never writes audio.
    #[test]
    #[ignore]
    fn cpal_plays_tone() {
        let out = CpalOut::new();
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let r = out.play(
            tone(0.5, 0.2, 22_050),
            0.5,
            Box::new(move |_| {
                c.fetch_add(1, Ordering::SeqCst);
            }),
        );
        assert!(r.is_ok(), "{r:?}");
        assert!(count.load(Ordering::SeqCst) > 5);
    }
}
