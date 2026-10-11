//! Native audio: sample mixer + MIDI music (replacement for HMI SOS).
//!
//! Mixing is driven by emulated time (`advance`), so sample completion is
//! deterministic; mixed frames are pushed into a ring buffer that the host
//! audio callback drains.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub const OUT_RATE: u32 = 44100;

#[derive(Clone, PartialEq)]
pub struct Voice {
    pub data: Arc<Vec<u8>>, // unsigned 8-bit mono PCM
    pub pos: f64,
    pub rate: f64,
    pub vol_l: f32,
    pub vol_r: f32,
    pub loops: i32, // -1 = infinite
    pub id: u32,
}

pub struct Mixer {
    pub voices: Vec<Option<Voice>>,
    pub master: f32,
    pub music: Option<crate::music::Player>,
    pub music_vol: f32,
    pub out_rate: f64,
}

impl Mixer {
    pub fn mix(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() {
            *s = 0.0;
        }
        let frames = out.len() / 2;
        let rate = self.out_rate;
        for v in self.voices.iter_mut() {
            let mut done = false;
            if let Some(vc) = v {
                let step = vc.rate / rate;
                for f in 0..frames {
                    if vc.pos as usize >= vc.data.len() {
                        if vc.loops != 0 && !vc.data.is_empty() {
                            if vc.loops > 0 {
                                vc.loops -= 1;
                            }
                            vc.pos -= vc.data.len() as f64;
                        } else {
                            done = true;
                            break;
                        }
                    }
                    let i = vc.pos as usize;
                    let s = (vc.data[i] as f32 - 128.0) / 128.0;
                    out[f * 2] += s * vc.vol_l * self.master;
                    out[f * 2 + 1] += s * vc.vol_r * self.master;
                    vc.pos += step;
                }
                if vc.pos as usize >= vc.data.len() && vc.loops == 0 {
                    done = true;
                }
            }
            if done {
                *v = None;
            }
        }
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

pub struct Audio {
    pub mixer: Mixer,
    /// General MIDI output for music (None when headless / unavailable)
    pub midi: Option<crate::midi_out::MidiOut>,
    pub ring: Arc<Mutex<VecDeque<f32>>>,
    pub enabled: bool,
    /// True when an output device consumes the ring buffer.
    pub output: bool,
    frac: f64,
    tmp: Vec<f32>,
}

impl Audio {
    pub fn new() -> Audio {
        let mixer = Mixer { voices: vec![None; 32], master: 0.8, music: None, music_vol: 1.0, out_rate: OUT_RATE as f64 };
        Audio {
            mixer,
            midi: None,
            ring: Arc::new(Mutex::new(VecDeque::new())),
            enabled: true,
            output: false,
            frac: 0.0,
            tmp: Vec::new(),
        }
    }

    /// Send MIDI messages to the output device (if any).
    pub fn midi_send(midi: &mut Option<crate::midi_out::MidiOut>, msg: &[u8]) {
        if let Some(m) = midi {
            m.send(msg);
        }
    }

    /// Stop the current song (silencing its notes).
    pub fn stop_music(&mut self) {
        if let Some(mut p) = self.mixer.music.take() {
            let midi = &mut self.midi;
            p.all_notes_off(&mut |m| Audio::midi_send(midi, m));
        }
    }

    /// Play `dt` seconds of music only. Used while the guest is paused (host pages,
    /// unfocused window): a frozen sequencer would leave its current notes held.
    pub fn advance_music(&mut self, dt: f64) {
        if let Some(p) = &mut self.mixer.music {
            let vol = self.mixer.music_vol;
            let midi = &mut self.midi;
            p.advance(dt, vol, &mut |m| Audio::midi_send(midi, m));
        }
    }

    /// Mix `dt` seconds of audio.
    pub fn advance(&mut self, dt: f64) {
        self.advance_music(dt);
        self.frac += dt * self.mixer.out_rate;
        let frames = self.frac as usize;
        if frames < 64 {
            return;
        }
        self.frac -= frames as f64;
        self.tmp.resize(frames * 2, 0.0);
        let mut tmp = std::mem::take(&mut self.tmp);
        self.mixer.mix(&mut tmp);
        if self.output {
            let mut ring = self.ring.lock().unwrap();
            ring.extend(tmp.iter().copied());
            // keep latency bounded (~150 ms)
            let max = (self.mixer.out_rate * 0.15) as usize * 2;
            while ring.len() > max {
                ring.pop_front();
            }
        }
        self.tmp = tmp;
    }
}
