//! HMI song sequencer (replacement for the HMI SOS MIDI library).
//!
//! Format (HMI-MIDISONG061595): song header with tick rate (Hz) at 0xD4,
//! track count at 0xE4 and a track offset table at 0xE8.  Each track
//! ("HMI-MIDITRACK") has its data offset at +0x57 and MIDI channel at +0x7B.
//! Events are standard MIDI except that note-ons carry a variable-length
//! duration; SOS schedules the matching note-off itself.
//!
//! Timing follows `sosMIDIProcessSong`: on every tick pending note-offs are
//! processed first, then every track counts down its delta and processes
//! events while the delta is zero.

/// A pending note-off (SOS "dependency list" entry).
struct NoteOff {
    chan: u8,
    key: u8,
    count: u32,
}

struct Track {
    pos: usize,
    delta: u32,
    running: u8,
    chan: u8,
    done: bool,
}

pub struct Player {
    data: Vec<u8>,
    tracks: Vec<Track>,
    offs: Vec<NoteOff>,
    pub rate: f64,
    pub done: bool,
    pub paused: bool,
    acc: f64,
    /// Raw CC7 per channel (scaled by the music volume on output)
    chan_vol: [u8; 16],
    /// Last music volume applied (0..1)
    applied_vol: f32,
}

fn vlq(d: &[u8], p: &mut usize) -> u32 {
    let mut v = 0u32;
    loop {
        let b = *d.get(*p).unwrap_or(&0);
        *p += 1;
        v = (v << 7) | (b & 0x7F) as u32;
        if b & 0x80 == 0 || *p >= d.len() {
            return v;
        }
    }
}

fn rd16(d: &[u8], o: usize) -> i16 {
    i16::from_le_bytes([d[o], d[o + 1]])
}
fn rd32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

impl Player {
    /// Parse an HMI song from a byte slice.
    pub fn new(data: Vec<u8>) -> Option<Player> {
        if data.len() < 0x100 || !data.starts_with(b"HMI-MIDISONG061595") {
            return None;
        }
        let rate = rd16(&data, 0xD4).max(1) as f64;
        let ntracks = rd16(&data, 0xE4).max(0) as usize;
        let table = rd32(&data, 0xE8) as usize;
        let mut tracks = Vec::new();
        for i in 0..ntracks {
            let to = table + i * 4;
            if to + 4 > data.len() {
                break;
            }
            let off = rd32(&data, to) as usize;
            if off + 0x80 > data.len() {
                continue;
            }
            let mut pos = off + rd32(&data, off + 0x57) as usize;
            let chan = (rd16(&data, off + 0x7B) & 15) as u8;
            if pos >= data.len() {
                continue;
            }
            let delta = vlq(&data, &mut pos);
            tracks.push(Track { pos, delta, running: 0, chan, done: false });
        }
        Some(Player {
            data,
            tracks,
            offs: Vec::new(),
            rate,
            done: false,
            paused: false,
            acc: 0.0,
            chan_vol: [100; 16],
            applied_vol: -1.0,
        })
    }

    /// Copy a song out of guest memory.
    pub fn from_guest(cpu: &x86::Cpu, ptr: u32) -> Option<Player> {
        let len = song_len(cpu, ptr);
        Player::new(cpu.read_bytes(ptr, len as usize).to_vec())
    }

    /// Advance by `dt` seconds, emitting MIDI messages to `out`.
    pub fn advance(&mut self, dt: f64, vol: f32, out: &mut dyn FnMut(&[u8])) {
        if (vol - self.applied_vol).abs() > 1e-4 {
            self.applied_vol = vol;
            for ch in 0..16u8 {
                if ch != 9 {
                    let v = self.scaled(self.chan_vol[ch as usize]);
                    out(&[0xB0 | ch, 7, v]);
                }
            }
        }
        if self.done || self.paused {
            return;
        }
        self.acc += dt * self.rate;
        while self.acc >= 1.0 && !self.done {
            self.acc -= 1.0;
            self.tick(out);
        }
    }

    fn scaled(&self, v: u8) -> u8 {
        ((v as f32) * self.applied_vol.clamp(0.0, 1.0)).round().min(127.0) as u8
    }

    /// Silence all sounding notes (pause / stop).
    pub fn all_notes_off(&mut self, out: &mut dyn FnMut(&[u8])) {
        for n in self.offs.drain(..) {
            out(&[0x90 | n.chan, n.key, 0]);
        }
        for ch in 0..16u8 {
            out(&[0xB0 | ch, 123, 0]);
        }
    }

    fn tick(&mut self, out: &mut dyn FnMut(&[u8])) {
        // pending note-offs fire when their counter was already zero
        let mut i = 0;
        while i < self.offs.len() {
            if self.offs[i].count == 0 {
                let n = self.offs.remove(i);
                out(&[0x90 | n.chan, n.key, 0]);
            } else {
                self.offs[i].count -= 1;
                i += 1;
            }
        }
        for t in 0..self.tracks.len() {
            if self.tracks[t].done {
                continue;
            }
            if self.tracks[t].delta != 0 {
                self.tracks[t].delta -= 1;
                continue;
            }
            loop {
                if !self.event(t, out) {
                    break;
                }
                let tr = &mut self.tracks[t];
                tr.delta = vlq(&self.data, &mut tr.pos);
                if tr.delta != 0 {
                    // SOS decrements the fresh delta once in the same tick
                    tr.delta -= 1;
                    break;
                }
            }
        }
        if self.tracks.iter().all(|t| t.done) {
            // sosMIDIStopSong: the song ends with its last track
            self.all_notes_off(out);
            self.done = true;
        }
    }

    /// Process one event of track `t`. Returns false when the track ended.
    fn event(&mut self, t: usize, out: &mut dyn FnMut(&[u8])) -> bool {
        let d = &self.data;
        let tr = &mut self.tracks[t];
        if tr.pos >= d.len() {
            tr.done = true;
            return false;
        }
        let mut st = d[tr.pos];
        if st >= 0x80 {
            tr.pos += 1;
            tr.running = st;
        } else {
            st = tr.running;
        }
        let ch = tr.chan;
        let byte = |p: usize| *d.get(p).unwrap_or(&0);
        match st & 0xF0 {
            0x90 => {
                let (key, mut vel) = (byte(tr.pos), byte(tr.pos + 1));
                tr.pos += 2;
                let dur = vlq(d, &mut tr.pos);
                self.offs.push(NoteOff { chan: ch, key, count: dur });
                if ch == 9 {
                    // drum volume is applied to the velocity (SOS does not send CC7 on 9)
                    vel = ((vel as u32 * self.chan_vol[9] as u32) / 127) as u8;
                    vel = self.scaled_vel(vel);
                }
                out(&[0x90 | ch, key, vel]);
            }
            0x80 => {
                let key = byte(tr.pos);
                tr.pos += 2;
                out(&[0x90 | ch, key, 0]);
            }
            0xB0 => {
                let (cc, val) = (byte(tr.pos), byte(tr.pos + 1));
                tr.pos += 2;
                match cc {
                    7 => {
                        self.chan_vol[ch as usize] = val;
                        if ch != 9 {
                            let v = self.scaled(val);
                            out(&[0xB0 | ch, 7, v]);
                        }
                    }
                    // HMI channel management controllers (not sent to the device)
                    0x67..=0x6B | 0x77 => {}
                    0x6C => {
                        // all notes off for this track's channel
                        let mut i = 0;
                        while i < self.offs.len() {
                            if self.offs[i].chan == ch {
                                let n = self.offs.remove(i);
                                out(&[0x90 | n.chan, n.key, 0]);
                            } else {
                                i += 1;
                            }
                        }
                    }
                    _ => out(&[0xB0 | ch, cc, val]),
                }
            }
            0xA0 | 0xE0 => {
                let (a, b) = (byte(tr.pos), byte(tr.pos + 1));
                tr.pos += 2;
                out(&[st & 0xF0 | ch, a, b]);
            }
            0xC0 | 0xD0 => {
                let a = byte(tr.pos);
                tr.pos += 1;
                out(&[st & 0xF0 | ch, a]);
            }
            _ => match st {
                0xFF => {
                    let typ = byte(tr.pos);
                    let len = byte(tr.pos + 1) as usize;
                    tr.pos += 2 + len;
                    if typ == 0x2F {
                        tr.done = true;
                        return false;
                    }
                }
                0xF0 => {
                    let len = rd32(d, tr.pos) as usize;
                    tr.pos += 4 + len;
                }
                0xFE => {
                    // HMI meta events (text/branch/loop): skip (unused by CyberMage songs)
                    let sub = byte(tr.pos);
                    tr.pos += match sub {
                        0x10 => byte(tr.pos + 3) as usize + 9,
                        0x12 | 0x14 => 4,
                        0x13 | 0x15 => 12,
                        _ => 8,
                    } - 1;
                }
                _ => {
                    tr.done = true;
                    return false;
                }
            },
        }
        true
    }

    fn scaled_vel(&self, v: u8) -> u8 {
        self.scaled(v)
    }
}

/// Size of an HMI song in memory: end of the last track (best effort).
pub fn song_len(cpu: &x86::Cpu, ptr: u32) -> u32 {
    let ntracks = cpu.rd16(ptr + 0xE4) as u32;
    let table = cpu.rd32(ptr + 0xE8);
    let mut end = 0x200;
    for i in 0..ntracks.min(64) {
        let off = cpu.rd32(ptr + table + i * 4);
        end = end.max(off + 0x80);
    }
    // the last track's data runs to its end-of-track event
    let mut p = end as usize;
    let max = 0x10000u32;
    while (p as u32) < max {
        if cpu.rd8(ptr + p as u32) == 0xFF && cpu.rd8(ptr + p as u32 + 1) == 0x2F {
            return p as u32 + 3;
        }
        p += 1;
    }
    max
}
