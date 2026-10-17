//! Real-time output through the default sound device (cpal; WASAPI on Windows).
//!
//! The [`Mixer`] lives inside the audio callback. Other threads send [`Command`]s through
//! [`Output::send`] and read the latest [`MixerState`] with [`Output::state`]. Audio files must be
//! decoded before they are sent (`Command::Load` carries the decoded [`crate::pcm::Pcm`]).
//! Devices with four or more output channels get the headphone cue on channels 3/4.
//! [`Output::silent`] runs the same mixer in real time without a sound device (tests, demos).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

use crate::mixer::{Command, Mixer, MixerState};
use crate::pcm::Frame;

/// Timing of the audio callback, for a load display and dropout detection.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AudioStats {
    pub callbacks: u64,
    /// Frames of the last callback.
    pub frames: u32,
    /// Render time / buffer duration (smoothed); above 1.0 the mixer is too slow.
    pub load: f32,
    /// Highest single-callback load.
    pub max_load: f32,
    /// Callbacks that took longer than their buffer duration (audible dropout likely).
    pub overloads: u64,
    /// Gaps between callbacks longer than 2× the buffer duration (the device or system stalled).
    pub late: u64,
}

/// Measures the callback; shared between the audio thread and readers.
#[derive(Clone, Default)]
struct StatsMeter {
    shared: Arc<Mutex<AudioStats>>,
    local: AudioStats,
    last_start: Option<std::time::Instant>,
}

impl StatsMeter {
    fn start(&mut self) -> std::time::Instant {
        std::time::Instant::now()
    }

    fn end(&mut self, start: std::time::Instant, frames: usize, sample_rate: u32) {
        let buf_s = frames as f64 / f64::from(sample_rate.max(1));
        let work = start.elapsed().as_secs_f64();
        let load = if buf_s > 0.0 { (work / buf_s) as f32 } else { 0.0 };
        let s = &mut self.local;
        s.callbacks += 1;
        s.frames = frames as u32;
        s.load = if s.callbacks == 1 { load } else { s.load * 0.95 + load * 0.05 };
        // the first callbacks include start-up work
        if s.callbacks > 20 {
            s.max_load = s.max_load.max(load);
            if load > 1.0 {
                s.overloads += 1;
            }
            if let Some(prev) = self.last_start {
                if start.duration_since(prev).as_secs_f64() > 2.0 * buf_s + 0.005 {
                    s.late += 1;
                }
            }
        }
        self.last_start = Some(start);
        if let Ok(mut g) = self.shared.try_lock() {
            *g = *s;
        }
    }
}

pub struct Output {
    _stream: Option<cpal::Stream>,
    stats: Arc<Mutex<AudioStats>>,
    /// Stops the silent render thread when dropped.
    stop: Option<Arc<AtomicBool>>,
    commands: Sender<Command>,
    state: Arc<Mutex<MixerState>>,
    pub sample_rate: u32,
    pub channels: u16,
    pub device_name: String,
}

/// Names of the available output devices of the default host.
pub fn list_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices().map(|it| it.filter_map(|d| d.name().ok()).collect()).unwrap_or_default()
}

impl Output {
    /// Open the default output device (or the first one whose name contains `name`) and start.
    pub fn open(name: Option<&str>) -> Result<Output, String> {
        Output::open_with(name, None)
    }

    /// Like [`Output::open`] with a fixed buffer size in frames (`None` = device default).
    pub fn open_with(name: Option<&str>, buffer_frames: Option<u32>) -> Result<Output, String> {
        let host = cpal::default_host();
        let device = match name {
            None => host.default_output_device().ok_or("no default output device")?,
            Some(n) => host
                .output_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().is_ok_and(|x| x.to_lowercase().contains(&n.to_lowercase())))
                .ok_or(format!("no output device matching {n:?}"))?,
        };
        let device_name = device.name().unwrap_or_else(|_| "?".into());
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let format = supported.sample_format();
        let mut config: cpal::StreamConfig = supported.into();
        if let Some(n) = buffer_frames {
            config.buffer_size = cpal::BufferSize::Fixed(n);
        }
        let sample_rate = config.sample_rate.0;
        let channels = config.channels;
        let (tx, rx) = channel::<Command>();
        let state = Arc::new(Mutex::new(MixerState::default()));
        let mut mixer = Mixer::new(sample_rate);
        let shared = state.clone();
        let mut meter = StatsMeter::default();
        let stats = meter.shared.clone();
        let fill = move |buf: &mut [Frame], cue: &mut [Frame]| {
            let t = meter.start();
            for cmd in rx.try_iter() {
                mixer.apply(cmd);
            }
            mixer.render_with_cue(buf, cue);
            if let Ok(mut s) = shared.try_lock() {
                *s = mixer.state();
            }
            meter.end(t, buf.len(), sample_rate);
        };
        let err = |e| eprintln!("audio stream error: {e}");
        let stream = match format {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, fill, err),
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, fill, err),
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, fill, err),
            cpal::SampleFormat::I32 => build::<i32>(&device, &config, fill, err),
            other => return Err(format!("unsupported sample format {other:?}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Output { _stream: Some(stream), stats, stop: None, commands: tx, state, sample_rate, channels, device_name })
    }

    /// The mixer in real time on a background thread, output discarded.
    pub fn silent(sample_rate: u32) -> Output {
        let (tx, rx) = channel::<Command>();
        let state = Arc::new(Mutex::new(MixerState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, halt) = (state.clone(), stop.clone());
        let mut meter = StatsMeter::default();
        let stats = meter.shared.clone();
        std::thread::spawn(move || {
            let mut mixer = Mixer::new(sample_rate);
            let block = (sample_rate / 100).max(1) as usize;
            let (mut buf, mut cue) = (vec![[0.0f32; 2]; block], vec![[0.0f32; 2]; block]);
            let start = std::time::Instant::now();
            let mut rendered = 0u64;
            while !halt.load(Ordering::Relaxed) {
                for cmd in rx.try_iter() {
                    mixer.apply(cmd);
                }
                // keep pace with the clock
                let due = (start.elapsed().as_secs_f64() * f64::from(sample_rate)) as u64;
                while rendered < due {
                    let t = meter.start();
                    mixer.render_with_cue(&mut buf, &mut cue);
                    meter.end(t, block, sample_rate);
                    rendered += block as u64;
                }
                if let Ok(mut s) = shared.try_lock() {
                    *s = mixer.state();
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        });
        Output { _stream: None, stats, stop: Some(stop), commands: tx, state, sample_rate, channels: 2, device_name: "(silent)".into() }
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.commands.send(cmd);
    }

    /// A sender that can be moved to another thread (e.g. a MIDI callback).
    pub fn sender(&self) -> Sender<Command> {
        self.commands.clone()
    }

    /// Shared handle to the latest state (can be moved to another thread).
    pub fn state_handle(&self) -> Arc<Mutex<MixerState>> {
        self.state.clone()
    }

    pub fn state(&self) -> MixerState {
        *self.state.lock().unwrap()
    }

    /// Callback timing (load, overloads, stalls).
    pub fn stats(&self) -> AudioStats {
        *self.stats.lock().unwrap()
    }

    /// True when no sound device is used.
    pub fn is_silent(&self) -> bool {
        self._stream.is_none()
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        if let Some(s) = &self.stop {
            s.store(true, Ordering::Relaxed);
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut fill: impl FnMut(&mut [Frame], &mut [Frame]) + Send + 'static,
    err: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels);
    let mut frames: Vec<Frame> = Vec::with_capacity(8192);
    let mut cue: Vec<Frame> = Vec::with_capacity(8192);
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let n = data.len() / channels.max(1);
                frames.resize(n, [0.0; 2]);
                cue.resize(n, [0.0; 2]);
                fill(&mut frames, &mut cue);
                for ((out, f), c) in data.chunks_mut(channels).zip(&frames).zip(&cue) {
                    match out.len() {
                        1 => out[0] = T::from_sample(0.5 * (f[0] + f[1])),
                        _ => {
                            out[0] = T::from_sample(f[0]);
                            out[1] = T::from_sample(f[1]);
                            for (i, s) in out[2..].iter_mut().enumerate() {
                                // channels 3/4: headphone cue
                                *s = T::from_sample(if i < 2 { c[i] } else { 0.0 });
                            }
                        }
                    }
                }
            },
            err,
            None,
        )
        .map_err(|e| e.to_string())
}
