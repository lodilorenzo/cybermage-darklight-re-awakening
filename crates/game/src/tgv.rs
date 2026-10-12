//! EA TGV movie decoder and player (replacement for PLAYTGV.EXE).
//!
//! Container: chunks of `tag(4) size(u32 LE, incl. header)`:
//!   `1SNh` EACS audio header, `1SNd` audio data (signed 8-bit PCM),
//!   `kVGT` key frame (LZ77-style), `fVGT` inter frame (4x4 blocks: motion
//!   vectors, raw blocks and 4-colour codebook blocks).  15 frames/second.

use crate::Emu;
use std::time::{Duration, Instant};

pub struct Tgv {
    pub width: usize,
    pub height: usize,
    pub frame: Vec<u8>,
    pub palette: [u32; 256],
    mvs: Vec<(i32, i32)>,
}

/// Little-endian bit reader (LSB first).
struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    fn get(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for i in 0..n {
            let byte = *self.d.get(self.pos >> 3).unwrap_or(&0);
            let bit = (byte >> (self.pos & 7)) & 1;
            v |= (bit as u32) << i;
            self.pos += 1;
        }
        v
    }
    fn get_signed(&mut self, n: u32) -> i32 {
        let v = self.get(n) as i32;
        (v << (32 - n)) >> (32 - n)
    }
}

impl Tgv {
    pub fn new() -> Tgv {
        Tgv { width: 0, height: 0, frame: Vec::new(), palette: [0; 256], mvs: Vec::new() }
    }

    /// Decode one video chunk payload (after the 8-byte chunk header).
    pub fn decode(&mut self, key: bool, buf: &[u8]) -> bool {
        if key {
            if buf.len() < 12 {
                return false;
            }
            let w = u16::from_le_bytes([buf[0], buf[1]]) as usize;
            let h = u16::from_le_bytes([buf[2], buf[3]]) as usize;
            if w != self.width || h != self.height {
                self.width = w;
                self.height = h;
                self.frame = vec![0; w * h];
            }
            let pal_count = u16::from_le_bytes([buf[6], buf[7]]) as usize;
            let mut p = 12;
            for i in 0..pal_count.min(256) {
                if p + 3 > buf.len() {
                    break;
                }
                self.palette[i] = ((buf[p] as u32) << 16) | ((buf[p + 1] as u32) << 8) | buf[p + 2] as u32;
                p += 3;
            }
            unpack(&buf[p..], &mut self.frame);
            true
        } else {
            if self.frame.is_empty() || buf.len() < 12 {
                return false;
            }
            self.decode_inter(buf)
        }
    }

    fn decode_inter(&mut self, buf: &[u8]) -> bool {
        let rd = |o: usize| u16::from_le_bytes([buf[o], buf[o + 1]]) as usize;
        let num_mvs = rd(0);
        let num_raw = rd(2);
        let num_packed = rd(4);
        let vector_bits = rd(6) as u32;
        if vector_bits == 0 || vector_bits > 25 {
            return false;
        }
        let mut p = 12;
        let mvbits = (num_mvs * 2 * 10 + 31) & !31;
        let mut bits = Bits { d: &buf[p..], pos: 0 };
        self.mvs.clear();
        for _ in 0..num_mvs {
            let x = bits.get_signed(10);
            let y = bits.get_signed(10);
            self.mvs.push((x, y));
        }
        p += mvbits / 8;
        if p + num_raw * 16 > buf.len() {
            return false;
        }
        let raw = &buf[p..p + num_raw * 16];
        p += num_raw * 16;
        let mut bits = Bits { d: &buf[p..], pos: 0 };
        let mut packed: Vec<[u8; 16]> = Vec::with_capacity(num_packed);
        for _ in 0..num_packed {
            let mut tmp = [0u8; 4];
            for t in tmp.iter_mut() {
                *t = bits.get(8) as u8;
            }
            let mut b = [0u8; 16];
            for j in 0..16 {
                b[15 - j] = tmp[bits.get(2) as usize];
            }
            packed.push(b);
        }
        let (w, h) = (self.width, self.height);
        let last = self.frame.clone();
        for by in 0..h / 4 {
            for bx in 0..w / 4 {
                let v = bits.get(vector_bits) as usize;
                if v < num_mvs {
                    let mx = (bx * 4) as i32 + self.mvs[v].0;
                    let my = (by * 4) as i32 + self.mvs[v].1;
                    if mx < 0 || mx + 4 > w as i32 || my < 0 || my + 4 > h as i32 {
                        continue;
                    }
                    for j in 0..4 {
                        for i in 0..4 {
                            self.frame[(by * 4 + j) * w + bx * 4 + i] = last[(my as usize + j) * w + mx as usize + i];
                        }
                    }
                } else {
                    let o = v - num_mvs;
                    let src: &[u8] = if o < num_raw {
                        &raw[o * 16..o * 16 + 16]
                    } else if o - num_raw < num_packed {
                        &packed[o - num_raw]
                    } else {
                        continue;
                    };
                    for j in 0..4 {
                        for i in 0..4 {
                            self.frame[(by * 4 + j) * w + bx * 4 + i] = src[j * 4 + i];
                        }
                    }
                }
            }
        }
        true
    }
}

/// Key-frame LZ decompression.
fn unpack(src: &[u8], dst: &mut [u8]) {
    let mut s = if src.first().map(|b| b & 1 != 0).unwrap_or(false) { 5 } else { 2 };
    if s + 3 > src.len() {
        return;
    }
    let mut size = ((src[s] as i64) << 16) | ((src[s + 1] as i64) << 8) | src[s + 2] as i64;
    s += 3;
    let mut d = 0usize;
    let b = |i: usize| *src.get(i).unwrap_or(&0) as usize;
    while size > 0 && s < src.len() {
        let c = src[s];
        let mut size1 = (c & 3) as usize;
        let mut size2 = 0usize;
        let mut offset = 0usize;
        if c & 0x80 != 0 {
            if c & 0x40 != 0 {
                if c & 0x20 != 0 {
                    if c < 0xFC {
                        size1 = (((c & 31) as usize) + 1) << 2;
                    }
                    s += 1;
                } else {
                    offset = (((c & 0x10) as usize) << 12) + ((b(s + 1) << 8) | b(s + 2)) + 1;
                    size2 = (((c & 0xC) as usize) << 6) + b(s + 3) + 5;
                    s += 4;
                }
            } else {
                size1 = (b(s + 1) & 0xC0) >> 6;
                offset = (((b(s + 1) << 8) | b(s + 2)) & 0x3FFF) + 1;
                size2 = ((c & 0x3F) as usize) + 4;
                s += 3;
            }
        } else {
            offset = (((c & 0x60) as usize) << 3) + b(s + 1) + 1;
            size2 = (((c & 0x1C) as usize) >> 2) + 3;
            s += 2;
        }
        if size1 > src.len() - s.min(src.len()) {
            break;
        }
        if size1 > 0 {
            size -= size1 as i64;
            let run = size1.min(dst.len() - d);
            dst[d..d + run].copy_from_slice(&src[s..s + run]);
            d += run;
            s += size1;
        }
        if size2 > 0 {
            if d < offset {
                return;
            }
            size -= size2 as i64;
            let run = size2.min(dst.len() - d);
            for k in 0..run {
                dst[d + k] = dst[d + k - offset];
            }
            d += run;
        }
    }
}

/// A parsed movie: chunk list.
pub struct Movie {
    pub data: Vec<u8>,
    pub chunks: Vec<(u32, usize, usize)>, // (tag, payload start, payload len)
    pub audio_rate: u32,
    pub audio_channels: u32,
}

const TAG_KVGT: u32 = u32::from_le_bytes(*b"kVGT");
const TAG_FVGT: u32 = u32::from_le_bytes(*b"fVGT");
const TAG_SND_D: u32 = u32::from_le_bytes(*b"1SNd");
const TAG_SND_H: u32 = u32::from_le_bytes(*b"1SNh");

impl Movie {
    pub fn open(path: &std::path::Path) -> Option<Movie> {
        let data = std::fs::read(path).ok()?;
        let mut chunks = Vec::new();
        let mut p = 0usize;
        let (mut rate, mut chans) = (22050, 2);
        while p + 8 <= data.len() {
            let tag = u32::from_le_bytes(data[p..p + 4].try_into().unwrap());
            let size = u32::from_le_bytes(data[p + 4..p + 8].try_into().unwrap()) as usize;
            if size < 8 || p + size > data.len() {
                break;
            }
            if tag == TAG_SND_H && &data[p + 8..p + 12] == b"EACS" {
                rate = u32::from_le_bytes(data[p + 12..p + 16].try_into().unwrap());
                chans = data[p + 17] as u32;
            }
            chunks.push((tag, p + 8, size - 8));
            p += size;
        }
        Some(Movie { data, chunks, audio_rate: rate, audio_channels: chans.max(1) })
    }
}

/// Play a movie requested via `spawnl(playtgv.exe, file, ...)`.
pub fn play_spawned(window: &mut minifb::Window, emu: &Emu, prog: &str, args: &[String]) {
    if !prog.to_ascii_lowercase().contains("playtgv") {
        eprintln!("[exec] unsupported program {}", prog);
        return;
    }
    let Some(file) = args.iter().find(|a| !a.starts_with('-')) else { return };
    let Some(path) = emu.m.vfs.resolve_read(file) else {
        eprintln!("[tgv] not found: {}", file);
        return;
    };
    let Some(movie) = Movie::open(&path) else { return };
    play(window, emu, &movie);
}

fn play(window: &mut minifb::Window, emu: &Emu, movie: &Movie) {
    let mut dec = Tgv::new();
    let mut rgb: Vec<u32> = Vec::new();
    let start = Instant::now();
    let mut frame_no = 0u32;
    let out_rate = emu.m.audio.mixer.out_rate;
    let ring = emu.m.audio.ring.clone();
    let mut resample_pos = 0.0f64;
    let step = movie.audio_rate as f64 / out_rate;
    for &(tag, off, len) in &movie.chunks {
        if !window.is_open() {
            return;
        }
        let payload = &movie.data[off..off + len];
        if tag == TAG_SND_D {
            // signed 8-bit interleaved PCM -> device rate stereo f32
            let ch = movie.audio_channels as usize;
            let frames = payload.len() / ch;
            let mut out = Vec::new();
            while (resample_pos as usize) < frames {
                let i = resample_pos as usize;
                let l = payload[i * ch] as i8 as f32 / 128.0;
                let r = if ch > 1 { payload[i * ch + 1] as i8 as f32 / 128.0 } else { l };
                out.push(l * 0.9);
                out.push(r * 0.9);
                resample_pos += step;
            }
            resample_pos -= frames as f64;
            ring.lock().unwrap().extend(out);
        } else if tag == TAG_KVGT || tag == TAG_FVGT {
            dec.decode(tag == TAG_KVGT, payload);
            frame_no += 1;
            let due = start + Duration::from_secs_f64(frame_no as f64 / 15.0);
            let now = Instant::now();
            if due > now {
                std::thread::sleep(due - now);
            }
            rgb.resize(dec.width * dec.height, 0);
            for (o, &p) in rgb.iter_mut().zip(dec.frame.iter()) {
                *o = dec.palette[p as usize];
            }
            if window.update_with_buffer(&rgb, dec.width, dec.height).is_err() { return; }
            crate::frontend::sync_display(emu.m.display.vsync && window.is_active());
            let pad = if window.is_active() { crate::gamepad::poll(emu.m.display.deadzone) }
                else { crate::gamepad::Pad::default() };
            let keyboard_skip = window.is_active() && [minifb::Key::Escape,minifb::Key::Enter,minifb::Key::NumPadEnter,minifb::Key::Space]
                .iter().any(|&key| window.is_key_pressed(key,minifb::KeyRepeat::No));
            if keyboard_skip || pad.down(crate::gamepad::A | crate::gamepad::B | crate::gamepad::START) {
                ring.lock().unwrap().clear();
                return;
            }
        }
    }
}

/// Decode the first `n` frames of a movie to BMP files (debug tool).
pub fn dump_frames(path: &std::path::Path, n: usize, out_dir: &std::path::Path) {
    let Some(movie) = Movie::open(path) else {
        eprintln!("cannot open {}", path.display());
        return;
    };
    eprintln!("{} chunks, audio {} Hz x{}", movie.chunks.len(), movie.audio_rate, movie.audio_channels);
    let mut dec = Tgv::new();
    let mut k = 0;
    for &(tag, off, len) in &movie.chunks {
        if tag == TAG_KVGT || tag == TAG_FVGT {
            dec.decode(tag == TAG_KVGT, &movie.data[off..off + len]);
            if k % 15 == 0 {
                let rgb: Vec<u32> = dec.frame.iter().map(|&p| dec.palette[p as usize]).collect();
                crate::write_bmp(&out_dir.join(format!("tgv_{:04}.bmp", k)), dec.width, dec.height, &rgb);
            }
            k += 1;
            if k >= n {
                break;
            }
        }
    }
}
