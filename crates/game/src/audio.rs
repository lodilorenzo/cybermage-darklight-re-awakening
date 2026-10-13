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
    /// Music synthesizer (None when headless / unavailable)
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

    pub fn is_sc55(&self) -> bool {
        self.midi.as_ref().is_some_and(|m| m.is_sc55())
    }

    /// Open first: failed changes leave the old synthesizer/song untouched.
    pub fn set_music_output(&mut self, sc55: bool, roms: &std::path::Path) -> Result<(), String> {
        let output = if sc55 {
            if !self.output { return Err("SC-55 requires an audio output device".into()); }
            crate::midi_out::MidiOut::Sc55(crate::sc55::Sc55::open(roms)?)
        } else {
            crate::midi_out::MidiOut::open().ok_or("No Windows MIDI output device")?
        };
        if let Some(p) = &mut self.mixer.music {
            let midi = &mut self.midi;
            p.all_notes_off(&mut |m| Audio::midi_send(midi, m));
            if !p.done { p.restart(); }
        }
        self.midi = Some(output);
        self.frac = 0.0;
        self.ring.lock().unwrap().clear();
        Ok(())
    }

    fn sequence(&mut self, dt: f64) {
        if let Some(p) = &mut self.mixer.music {
            let vol = self.mixer.music_vol;
            let midi = &mut self.midi;
            p.advance(dt, vol, &mut |m| Audio::midi_send(midi, m));
        }
    }

    /// Host pages pause sound effects, not the song or SC-55's release/effects tails.
    pub fn advance_music(&mut self, dt: f64) {
        if self.is_sc55() { self.mix(dt, false); }
        else { self.sequence(dt); }
    }

    pub fn advance(&mut self, dt: f64) {
        if !self.is_sc55() { self.sequence(dt); }
        self.mix(dt, true);
    }

    fn mix(&mut self, dt: f64, effects: bool) {
        self.frac += dt * self.mixer.out_rate;
        let frames = self.frac as usize;
        if frames < 64 { return; }
        self.frac -= frames as f64;
        self.tmp.resize(frames * 2, 0.0);
        let mut tmp = std::mem::take(&mut self.tmp);
        let sc55 = self.is_sc55();
        // Keep MIDI scheduling within 64 host frames of the SC-55 PCM timeline.
        for chunk in tmp.chunks_mut(128) {
            if sc55 { self.sequence((chunk.len()/2) as f64 / self.mixer.out_rate); }
            if effects { self.mixer.mix(chunk); } else { chunk.fill(0.0); }
            if sc55 {
                if let Some(midi) = &mut self.midi {
                    if !midi.mix(chunk, self.mixer.out_rate) {
                        eprintln!("SC-55 rendering failed; music disabled");
                        self.midi = None;
                    }
                }
                for s in chunk { *s = s.clamp(-1.0, 1.0); }
            }
        }
        if self.output {
            let mut ring = self.ring.lock().unwrap();
            ring.extend(tmp.iter().copied());
            // keep latency bounded (~150 ms)
            let max = (self.mixer.out_rate * 0.15) as usize * 2;
            while ring.len() > max { ring.pop_front(); }
        }
        self.tmp = tmp;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_sc55_change_preserves_song_samples_and_buffer() {
        let mut audio = Audio::new();
        let mut data = vec![0; 256];
        data[..18].copy_from_slice(b"HMI-MIDISONG061595");
        audio.mixer.music = crate::music::Player::new(data);
        audio.mixer.music.as_mut().unwrap().paused = true;
        audio.mixer.voices[0] = Some(Voice { data: Arc::new(vec![255; 1024]), pos: 0.0,
            rate: 11025.0, vol_l: 1.0, vol_r: 1.0, loops: 0, id: 1 });
        audio.ring.lock().unwrap().push_back(0.5);
        let missing = std::env::temp_dir().join(format!("cybermage-missing-roms-{}", std::process::id()));
        assert!(audio.set_music_output(true, &missing).is_err()); // no audio device
        audio.output = true;
        assert!(audio.set_music_output(true, &missing).is_err()); // no ROMs
        assert!(!audio.is_sc55());
        assert!(audio.mixer.music.as_ref().unwrap().paused);
        assert!(!audio.mixer.music.as_ref().unwrap().done);
        assert_eq!(audio.ring.lock().unwrap().iter().copied().collect::<Vec<_>>(), [0.5]);
        audio.advance_music(1.0 / 60.0);
        assert_eq!(audio.mixer.voices[0].as_ref().unwrap().pos, 0.0);
        audio.advance(1.0 / 60.0);
        assert!(audio.mixer.voices[0].as_ref().unwrap().pos > 0.0);
        assert!(audio.ring.lock().unwrap().iter().any(|&s| s > 0.5));
    }
}
