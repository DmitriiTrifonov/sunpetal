//! The sound device. The unit mixes at 32 728.5 Hz; the device wants its own rate: a ring of
//! stereo samples the machine fills each frame and the device's callback drains, resampled
//! linearly, its speed nudged to keep the ring about a tenth of a second full (the machine runs on
//! the wall clock, the device on its own).
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const SOURCE_RATE: f64 = 32_728.5;
/// Stereo frames the ring should hold.
const TARGET: f64 = 3200.0;

pub struct Ring {
    q: Mutex<VecDeque<i16>>,
}

impl Ring {
    fn new() -> Arc<Ring> {
        Arc::new(Ring { q: Mutex::new(VecDeque::new()) })
    }

    /// Interleaved stereo samples from the machine. Past half a second the oldest go (the device
    /// stopped, or the machine runs ahead of it).
    pub fn push(&self, s: &[i16]) {
        let mut q = self.q.lock().unwrap();
        q.extend(s.iter().copied());
        let max = (SOURCE_RATE * 0.5) as usize * 2;
        while q.len() > max {
            q.pop_front();
            q.pop_front();
        }
    }
}

/// Opens the default output device. The stream must be kept alive.
pub fn open() -> Result<(Arc<Ring>, cpal::Stream), String> {
    let host = cpal::default_host();
    let dev = host.default_output_device().ok_or("no sound device")?;
    let cfg = dev.default_output_config().map_err(|e| e.to_string())?;
    let rate = cfg.sample_rate() as f64;
    let channels = cfg.channels() as usize;
    let ring = Ring::new();
    let r = ring.clone();
    let mut pos = 0.0f64;
    let mut last = (0.0f32, 0.0f32);
    let err = |e| eprintln!("sound: {}", e);
    let stream = dev
        .build_output_stream(
            cfg.config(),
            move |out: &mut [f32], _| {
                let mut q = r.q.lock().unwrap();
                let fill = (q.len() / 2) as f64;
                // too empty: slower; too full: faster
                let adjust = 1.0 + ((fill - TARGET) / TARGET).clamp(-1.0, 1.0) * 0.02;
                let step = SOURCE_RATE / rate * adjust;
                for frame in out.chunks_mut(channels) {
                    let (l, rr) = if q.len() >= 4 {
                        let f = pos as f32 - pos.floor() as f32;
                        let (a, b) = ((q[0] as f32, q[1] as f32), (q[2] as f32, q[3] as f32));
                        let v = (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
                        pos += step;
                        while pos >= 1.0 && q.len() >= 4 {
                            q.pop_front();
                            q.pop_front();
                            pos -= 1.0;
                        }
                        last = v;
                        v
                    } else {
                        last
                    };
                    for (i, s) in frame.iter_mut().enumerate() {
                        *s = if i == 0 { l / 32768.0 } else if i == 1 { rr / 32768.0 } else { 0.0 };
                    }
                }
            },
            err,
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((ring, stream))
}
