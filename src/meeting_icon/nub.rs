// Recording pill layout and motion adapted from Hex (MIT). See THIRD_PARTY.md.
use std::{
    f32::consts::TAU,
    time::{Duration, Instant},
};

pub(super) const WIDTH: f32 = 151.0;
pub(super) const HEIGHT: f32 = 36.0;
pub(super) const PAD: f32 = 14.0;
const PITCH: f32 = 2.5;
const ATTACK: f32 = 0.16;
const RELEASE: f32 = 0.55;
const COOL: f32 = 0.45;
pub(super) const FILL: u32 = 0x1d1d1b;
pub(super) const BORDER: u32 = 0x3a3a37;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Look {
    pub level: f32,
    phase: f32,
    pub opacity: f32,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Motion {
    last: Option<Instant>,
    stopped: Option<Instant>,
    recording: bool,
    phase: f32,
    level: f32,
    target: f32,
    velocity: f32,
}

impl Motion {
    pub fn due(self, now: Instant, ms: u32) -> bool {
        self.last.is_some_and(|last| {
            now.saturating_duration_since(last) >= Duration::from_millis(ms.into())
        })
    }

    pub fn advance(&mut self, now: Instant, recording: bool, rms: Option<f32>) -> Option<Look> {
        if recording && !self.recording {
            *self = Self::default();
        } else if !recording && self.recording {
            self.stopped = Some(now);
        }
        self.recording = recording;
        let opacity = match self.stopped {
            Some(stopped) => {
                let t = now.saturating_duration_since(stopped).as_secs_f32() / COOL;
                if t >= 1.0 {
                    *self = Self::default();
                    return None;
                }
                1.0 - t * t * (3.0 - 2.0 * t)
            }
            None if recording => 1.0,
            None => return None,
        };
        let dt = now
            .saturating_duration_since(self.last.replace(now).unwrap_or(now))
            .as_secs_f32();
        let rms = rms
            .filter(|value| recording && value.is_finite())
            .unwrap_or_default();
        let loudness = 20.0 * (rms.max(0.0) + 1e-6).log10();
        let target = ((loudness + 60.0) / 50.0).clamp(0.0, 1.0);
        if target != self.target {
            self.velocity = 0.0;
            self.target = target;
        }
        let omega = TAU / if target > self.level { ATTACK } else { RELEASE };
        let x0 = self.level - target;
        let v0 = self.velocity;
        let exp = (-omega * dt).exp();
        let c = v0 + omega * x0;
        self.level = target + (x0 + c * dt) * exp;
        self.velocity = (v0 - omega * c * dt) * exp;
        if (self.level - target).abs() < 0.001 {
            self.level = target;
            self.velocity = 0.0;
        }
        self.phase = (self.phase + dt * 8.0).rem_euclid(TAU);
        Some(Look {
            level: self.level,
            phase: if self.level > 0.0 { self.phase } else { 0.0 },
            opacity,
        })
    }
}

pub(super) fn draw_field(pixels: &mut [u32], stride: usize, scale: f32, right: f32, look: Look) {
    if look.level == 0.0 {
        return;
    }
    let brightness = look.level.sqrt();
    for col in 0..31 {
        let x = PAD + 1.5 + col as f32 * PITCH;
        if x + 1.0 >= right {
            break;
        }
        let edge = (col.min(30 - col) as f32 / 6.0).min(1.0);
        let trace = (col as f32 * TAU / 17.0 - look.phase).sin() * 10.0 * look.level * edge;
        for row in -4..=4 {
            let y = row as f32 * PITCH;
            let distance = (y - trace).abs().min((y + trace).abs());
            let light = (1.0 - distance / 3.3).max(0.0) * edge * brightness;
            for py in ((HEIGHT / 2.0 + y - 1.5) * scale) as usize
                ..=((HEIGHT / 2.0 + y + 1.5) * scale) as usize
            {
                for px in ((x - 1.5) * scale) as usize..=((x + 1.5) * scale) as usize {
                    let dx = (px as f32 + 0.5) / scale - x;
                    let dy = (py as f32 + 0.5) / scale - HEIGHT / 2.0 - y;
                    let ink = light * ((0.65 - dx.hypot(dy)) * scale + 0.5).clamp(0.0, 1.0);
                    if ink > 0.0 {
                        let channel = |from: f32, to: f32| (from + (to - from) * ink) as u32;
                        pixels[py * stride + px] = channel(29.0, 255.0) << 16
                            | channel(29.0, 159.0) << 8
                            | channel(27.0, 10.0);
                    }
                }
            }
        }
    }
}

#[test]
fn nonfinite_level_is_silent() {
    let frame = Motion::default().advance(Instant::now(), true, Some(f32::NAN));
    assert_eq!(frame.unwrap().level, 0.0);
}
