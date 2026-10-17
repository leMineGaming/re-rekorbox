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

pub struct Output {
    _stream: Option<cpal::Stream>,
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
        let config: cpal::StreamConfig = supported.into();
        let sample_rate = config.sample_rate.0;
        let channels = config.channels;
        let (tx, rx) = channel::<Command>();
        let state = Arc::new(Mutex::new(MixerState::default()));
        let mut mixer = Mixer::new(sample_rate);
        let shared = state.clone();
        let fill = move |buf: &mut [Frame], cue: &mut [Frame]| {
            for cmd in rx.try_iter() {
                mixer.apply(cmd);
            }
            mixer.render_with_cue(buf, cue);
            if let Ok(mut s) = shared.try_lock() {
                *s = mixer.state();
            }
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
        Ok(Output { _stream: Some(stream), stop: None, commands: tx, state, sample_rate, channels, device_name })
    }

    /// The mixer in real time on a background thread, output discarded.
    pub fn silent(sample_rate: u32) -> Output {
        let (tx, rx) = channel::<Command>();
        let state = Arc::new(Mutex::new(MixerState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, halt) = (state.clone(), stop.clone());
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
                    mixer.render_with_cue(&mut buf, &mut cue);
                    rendered += block as u64;
                }
                if let Ok(mut s) = shared.try_lock() {
                    *s = mixer.state();
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        });
        Output { _stream: None, stop: Some(stop), commands: tx, state, sample_rate, channels: 2, device_name: "(silent)".into() }
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
