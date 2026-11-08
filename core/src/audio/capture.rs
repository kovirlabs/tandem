//! Microphone capture (M2 Part A).
//!
//! Frames go straight to a callback as 16 kHz mono `f32`. Nothing here buffers, stores, logs
//! or writes audio; the controller keeps the current utterance in memory only.
//!
//! The device opens lazily in `start` (never in a constructor), on a dedicated thread because
//! a cpal `Stream` is not `Send` on every platform.

use std::sync::mpsc;
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::{AudioError, STT_SAMPLE_RATE};

/// Receives 16 kHz mono frames (any length) from the audio thread.
pub type FrameCallback = Box<dyn Fn(&[f32]) + Send>;

/// A microphone. Implemented by [`CpalMic`]; tests use a fake.
pub trait MicSource: Send + Sync {
    /// Open the device and start calling `on_frame`. Calling `start` while running replaces
    /// the running stream.
    fn start(&self, on_frame: FrameCallback) -> Result<(), AudioError>;
    /// Stop and close the device. Safe to call when not running.
    fn stop(&self);
    /// Input device names for the Settings picker. Does not open any device.
    fn devices(&self) -> Vec<String>;
    /// Use this device next time `start` is called (`None` = system default).
    fn set_device(&self, _name: Option<String>) {}
}

/// Average interleaved channels down to mono.
pub fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks(channels)
        .map(|c| c.iter().sum::<f32>() / c.len() as f32)
        .collect()
}

/// Root-mean-square loudness of a frame, clamped to 0–1 (a full-scale sine is about 0.707).
pub fn rms_level(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: f32 = frame.iter().map(|s| s * s).sum();
    let rms = (sum / frame.len() as f32).sqrt();
    if rms.is_finite() {
        rms.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Streaming sample-rate converter. Downsampling averages the source samples under each
/// output sample (a cheap low-pass that keeps aliasing down for speech); upsampling
/// interpolates linearly. Good enough for speech recognition and voice activity detection.
pub struct Resampler {
    step: f64,
    pos: f64,
    carry: Vec<f32>,
}

impl Resampler {
    pub fn new(from_hz: u32, to_hz: u32) -> Self {
        let step = if to_hz == 0 {
            1.0
        } else {
            f64::from(from_hz.max(1)) / f64::from(to_hz)
        };
        Self {
            step,
            pos: 0.0,
            carry: Vec::new(),
        }
    }

    pub fn is_passthrough(&self) -> bool {
        (self.step - 1.0).abs() < f64::EPSILON
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if self.is_passthrough() {
            return input.to_vec();
        }
        self.carry.extend_from_slice(input);
        let mut out = Vec::with_capacity((input.len() as f64 / self.step) as usize + 1);
        let len = self.carry.len() as f64;
        if self.step > 1.0 {
            while self.pos + self.step <= len {
                let a = self.pos as usize;
                let b = ((self.pos + self.step) as usize).max(a + 1);
                let window = &self.carry[a..b];
                out.push(window.iter().sum::<f32>() / window.len() as f32);
                self.pos += self.step;
            }
        } else {
            while self.pos + 1.0 < len {
                let i = self.pos as usize;
                let t = (self.pos - i as f64) as f32;
                out.push(self.carry[i] * (1.0 - t) + self.carry[i + 1] * t);
                self.pos += self.step;
            }
        }
        let used = (self.pos as usize).min(self.carry.len());
        self.carry.drain(..used);
        self.pos -= used as f64;
        out
    }
}

/// Map a cpal error to the problems the UI knows how to explain.
pub fn map_cpal_error(e: &cpal::Error) -> AudioError {
    match e.kind() {
        cpal::ErrorKind::PermissionDenied => AudioError::PermissionDenied,
        cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::HostUnavailable => {
            AudioError::NoInputDevice
        }
        _ => AudioError::Device(e.to_string()),
    }
}

fn device_name(d: &cpal::Device) -> Option<String> {
    d.description().ok().map(|desc| desc.name().to_string())
}

struct Running {
    stop: mpsc::Sender<()>,
    thread: JoinHandle<()>,
}

/// The system microphone via cpal.
pub struct CpalMic {
    device: Mutex<Option<String>>,
    running: Mutex<Option<Running>>,
}

impl CpalMic {
    /// Does not touch any device.
    pub fn new(device_name: Option<String>) -> Self {
        Self {
            device: Mutex::new(device_name.filter(|n| !n.trim().is_empty())),
            running: Mutex::new(None),
        }
    }

    fn pick_device(host: &cpal::Host, wanted: Option<&str>) -> Result<cpal::Device, AudioError> {
        if let Some(name) = wanted {
            let found = host
                .input_devices()
                .map_err(|e| map_cpal_error(&e))?
                .find(|d| device_name(d).as_deref() == Some(name));
            if let Some(d) = found {
                return Ok(d);
            }
            tracing::info!("chosen microphone not found; using the system default");
        }
        host.default_input_device().ok_or(AudioError::NoInputDevice)
    }

    /// Runs on the capture thread: open, play, wait for stop, close.
    fn run(
        wanted: Option<String>,
        on_frame: FrameCallback,
        ready: mpsc::Sender<Result<(), AudioError>>,
        stop: mpsc::Receiver<()>,
    ) {
        let host = cpal::default_host();
        let opened = Self::pick_device(&host, wanted.as_deref()).and_then(|device| {
            let supported = device
                .default_input_config()
                .map_err(|e| map_cpal_error(&e))?;
            let format = supported.sample_format();
            let config = supported.config();
            let channels = usize::from(config.channels.max(1));
            let resampler = Mutex::new(Resampler::new(config.sample_rate, STT_SAMPLE_RATE));
            let deliver = move |mono: Vec<f32>| {
                let out = match resampler.lock() {
                    Ok(mut r) => r.process(&mono),
                    Err(_) => return,
                };
                if !out.is_empty() {
                    on_frame(&out);
                }
            };
            let on_err = |e: cpal::Error| tracing::warn!("microphone stream error: {}", e.kind());
            let stream = match format {
                cpal::SampleFormat::F32 => device.build_input_stream::<f32, _, _>(
                    config,
                    move |data, _| deliver(downmix(data, channels)),
                    on_err,
                    None,
                ),
                cpal::SampleFormat::I16 => device.build_input_stream::<i16, _, _>(
                    config,
                    move |data, _| {
                        let f: Vec<f32> = data.iter().map(|s| f32::from(*s) / 32_768.0).collect();
                        deliver(downmix(&f, channels))
                    },
                    on_err,
                    None,
                ),
                cpal::SampleFormat::I32 => device.build_input_stream::<i32, _, _>(
                    config,
                    move |data, _| {
                        let f: Vec<f32> =
                            data.iter().map(|s| *s as f32 / 2_147_483_648.0).collect();
                        deliver(downmix(&f, channels))
                    },
                    on_err,
                    None,
                ),
                cpal::SampleFormat::U16 => device.build_input_stream::<u16, _, _>(
                    config,
                    move |data, _| {
                        let f: Vec<f32> = data
                            .iter()
                            .map(|s| (f32::from(*s) - 32_768.0) / 32_768.0)
                            .collect();
                        deliver(downmix(&f, channels))
                    },
                    on_err,
                    None,
                ),
                other => {
                    return Err(AudioError::Device(format!(
                        "unsupported microphone sample format {other:?}"
                    )))
                }
            }
            .map_err(|e| map_cpal_error(&e))?;
            stream.play().map_err(|e| map_cpal_error(&e))?;
            Ok(stream)
        });
        match opened {
            Ok(stream) => {
                let _ = ready.send(Ok(()));
                // Block until asked to stop (or the controller is gone).
                let _ = stop.recv();
                drop(stream);
            }
            Err(e) => {
                let _ = ready.send(Err(e));
            }
        }
    }
}

impl MicSource for CpalMic {
    fn start(&self, on_frame: FrameCallback) -> Result<(), AudioError> {
        self.stop();
        let wanted = self.device.lock().ok().and_then(|g| g.clone());
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("tandem-mic".into())
            .spawn(move || Self::run(wanted, on_frame, ready_tx, stop_rx))
            .map_err(|e| AudioError::Device(e.to_string()))?;
        let result = ready_rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|_| Err(AudioError::Device("the microphone didn't start".into())));
        match result {
            Ok(()) => {
                if let Ok(mut g) = self.running.lock() {
                    *g = Some(Running {
                        stop: stop_tx,
                        thread,
                    });
                }
                Ok(())
            }
            Err(e) => {
                drop(stop_tx);
                let _ = thread.join();
                Err(e)
            }
        }
    }

    fn stop(&self) {
        let running = self.running.lock().ok().and_then(|mut g| g.take());
        if let Some(r) = running {
            let _ = r.stop.send(());
            let _ = r.thread.join();
        }
    }

    fn devices(&self) -> Vec<String> {
        let host = cpal::default_host();
        match host.input_devices() {
            Ok(devs) => devs.filter_map(|d| device_name(&d)).collect(),
            Err(e) => {
                tracing::info!("can't list microphones: {}", e.kind());
                Vec::new()
            }
        }
    }

    fn set_device(&self, name: Option<String>) {
        if let Ok(mut g) = self.device.lock() {
            *g = name.filter(|n| !n.trim().is_empty());
        }
    }
}

impl Drop for CpalMic {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Scripted microphone for tests: frames are pushed by the test with [`FakeMic::feed`].
#[cfg(test)]
pub(crate) mod fake {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::{FrameCallback, MicSource};
    use crate::audio::AudioError;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FakeFailure {
        NoDevice,
        Denied,
    }

    #[derive(Default)]
    pub struct FakeMic {
        callback: Mutex<Option<FrameCallback>>,
        fail: Mutex<Option<FakeFailure>>,
        pub starts: AtomicUsize,
        pub stops: AtomicUsize,
    }

    impl FakeMic {
        pub fn failing(f: FakeFailure) -> Self {
            let m = Self::default();
            if let Ok(mut g) = m.fail.lock() {
                *g = Some(f);
            }
            m
        }

        pub fn is_running(&self) -> bool {
            self.callback.lock().map(|g| g.is_some()).unwrap_or(false)
        }

        /// Deliver samples as if the mic captured them (no-op when stopped).
        pub fn feed(&self, samples: &[f32]) {
            if let Ok(g) = self.callback.lock() {
                if let Some(cb) = g.as_ref() {
                    for chunk in samples.chunks(320) {
                        cb(chunk);
                    }
                }
            }
        }
    }

    impl MicSource for FakeMic {
        fn start(&self, on_frame: FrameCallback) -> Result<(), AudioError> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            match self.fail.lock().ok().and_then(|g| *g) {
                Some(FakeFailure::NoDevice) => return Err(AudioError::NoInputDevice),
                Some(FakeFailure::Denied) => return Err(AudioError::PermissionDenied),
                None => {}
            }
            if let Ok(mut g) = self.callback.lock() {
                *g = Some(on_frame);
            }
            Ok(())
        }

        fn stop(&self) {
            self.stops.fetch_add(1, Ordering::SeqCst);
            if let Ok(mut g) = self.callback.lock() {
                *g = None;
            }
        }

        fn devices(&self) -> Vec<String> {
            vec!["Fake mic".into()]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_48k_to_16k_length() {
        let mut r = Resampler::new(48_000, 16_000);
        let input = vec![0.25f32; 48_000];
        // Feed in uneven chunks like a real device would.
        let mut out = Vec::new();
        for chunk in input.chunks(441) {
            out.extend(r.process(chunk));
        }
        assert!((out.len() as i64 - 16_000).abs() <= 1, "got {}", out.len());
        assert!(out.iter().all(|s| (s - 0.25).abs() < 1e-6));
    }

    #[test]
    fn resample_44k1_and_upsample_lengths() {
        let mut r = Resampler::new(44_100, 16_000);
        let n: usize = r.process(&vec![0.0; 44_100]).len();
        assert!((n as i64 - 16_000).abs() <= 1, "got {n}");
        let mut up = Resampler::new(8_000, 16_000);
        let n = up.process(&vec![0.0; 8_000]).len();
        assert!((n as i64 - 16_000).abs() <= 2, "got {n}");
        let mut same = Resampler::new(16_000, 16_000);
        assert_eq!(same.process(&[0.1, 0.2]), vec![0.1, 0.2]);
    }

    #[test]
    fn stereo_downmix_averages() {
        assert_eq!(
            downmix(&[1.0, 0.0, 0.5, 0.5, -1.0, 1.0], 2),
            vec![0.5, 0.5, 0.0]
        );
        assert_eq!(downmix(&[0.3, 0.6], 1), vec![0.3, 0.6]);
    }

    #[test]
    fn rms_of_silence_is_zero() {
        assert_eq!(rms_level(&[0.0; 512]), 0.0);
        assert_eq!(rms_level(&[]), 0.0);
    }

    #[test]
    fn rms_of_full_scale_sine_is_about_0_7() {
        let sine: Vec<f32> = (0..16_000)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16_000.0).sin())
            .collect();
        assert!((rms_level(&sine) - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01);
    }

    #[test]
    fn cpal_errors_map_to_problems() {
        let denied = cpal::Error::new(cpal::ErrorKind::PermissionDenied);
        assert!(matches!(
            map_cpal_error(&denied),
            AudioError::PermissionDenied
        ));
        let gone = cpal::Error::new(cpal::ErrorKind::DeviceNotAvailable);
        assert!(matches!(map_cpal_error(&gone), AudioError::NoInputDevice));
        let other = cpal::Error::new(cpal::ErrorKind::Xrun);
        assert!(matches!(map_cpal_error(&other), AudioError::Device(_)));
    }

    #[test]
    fn constructor_opens_nothing() {
        let mic = CpalMic::new(Some(String::new()));
        assert!(mic.device.lock().map(|g| g.is_none()).unwrap_or(false));
        mic.stop(); // safe when not running
    }

    #[test]
    #[ignore = "needs a real microphone and permission"]
    fn real_mic_delivers_frames() {
        let mic = CpalMic::new(None);
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = count.clone();
        let started = mic.start(Box::new(move |f| {
            c.fetch_add(f.len(), std::sync::atomic::Ordering::SeqCst);
        }));
        assert!(started.is_ok(), "{started:?}");
        std::thread::sleep(Duration::from_millis(500));
        mic.stop();
        assert!(count.load(std::sync::atomic::Ordering::SeqCst) > 4_000);
    }
}
