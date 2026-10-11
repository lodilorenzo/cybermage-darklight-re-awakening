//! VGA / VESA display adapter emulation.
//!
//! Banked VESA modes are emulated by keeping the 64 KiB window at 0xA0000 in
//! CPU memory and swapping it with the large `vram` on every bank switch.

use x86::Cpu;

pub const WINDOW: u32 = 0xA0000;
pub const WIN_SIZE: usize = 0x10000;
pub const VRAM_SIZE: usize = 4 << 20;

pub struct Vga {
    pub vram: Vec<u8>,
    pub bank: usize,
    pub mode: u16,
    pub width: u32,
    pub height: u32,
    /// Bytes per scanline
    pub pitch: u32,
    pub display_start: u32,
    pub dac: [[u8; 3]; 256],
    pub dac_w: usize,
    pub dac_r: usize,
    pub dac_c: usize,
    pub status_toggle: u32,
    pub seq_idx: u8,
    pub seq: [u8; 8],
    pub crtc_idx: u8,
    pub crtc: [u8; 32],
    pub gc_idx: u8,
    pub gc: [u8; 16],
    /// Mode X planar layout is active (unchained 256 colour)
    pub planar: bool,
    pub retrace: bool,
    pub frames: u64,
}

impl Vga {
    pub fn new() -> Vga {
        Vga {
            vram: vec![0; VRAM_SIZE],
            bank: 0,
            mode: 3,
            width: 320,
            height: 200,
            pitch: 320,
            display_start: 0,
            dac: [[0; 3]; 256],
            dac_w: 0,
            dac_r: 0,
            dac_c: 0,
            status_toggle: 0,
            seq_idx: 0,
            seq: [0; 8],
            crtc_idx: 0,
            crtc: [0; 32],
            gc_idx: 0,
            gc: [0; 16],
            planar: false,
            retrace: false,
            frames: 0,
        }
    }

    pub fn is_vesa(&self) -> bool {
        self.mode >= 0x100
    }

    /// Copy the CPU-visible window back into VRAM (call before reading vram).
    pub fn flush_window(&mut self, cpu: &Cpu) {
        if self.is_vesa() {
            let off = self.bank * WIN_SIZE;
            if off + WIN_SIZE <= self.vram.len() {
                self.vram[off..off + WIN_SIZE].copy_from_slice(cpu.read_bytes(WINDOW, WIN_SIZE));
            }
        }
    }

    pub fn set_bank(&mut self, cpu: &mut Cpu, bank: usize) {
        if !self.is_vesa() || bank == self.bank {
            self.bank = bank;
            return;
        }
        self.flush_window(cpu);
        self.bank = bank;
        let off = bank * WIN_SIZE;
        if off + WIN_SIZE <= self.vram.len() {
            cpu.write_bytes(WINDOW, &self.vram[off..off + WIN_SIZE]);
        }
    }

    pub fn set_mode(&mut self, cpu: &mut Cpu, mode: u16, clear: bool) -> bool {
        let m = mode & 0x1FF;
        let (w, h) = match m {
            0x13 => (320, 200),
            0x03 | 0x02 => (80, 25),
            0x100 => (640, 400),
            0x101 => (640, 480),
            0x103 => (800, 600),
            0x105 => (1024, 768),
            _ => return false,
        };
        self.mode = m;
        self.width = w;
        self.height = h;
        self.pitch = w;
        self.display_start = 0;
        self.bank = 0;
        self.planar = false;
        if clear || true {
            self.vram.iter_mut().for_each(|b| *b = 0);
            if m != 3 {
                cpu.mem[WINDOW as usize..WINDOW as usize + WIN_SIZE].fill(0);
            }
        }
        if m == 0x13 || m >= 0x100 {
            // default VGA palette-ish: grayscale ramp until program sets one
            for i in 0..256 {
                let v = (i >> 2) as u8;
                self.dac[i] = [v, v, v];
            }
        }
        true
    }

    pub fn port_in(&mut self, port: u16) -> u8 {
        match port {
            0x3DA | 0x3BA => {
                // Alternate display-enable / vertical-retrace bits so polling loops terminate.
                self.status_toggle = self.status_toggle.wrapping_add(1);
                let vr = self.retrace;
                let de = self.status_toggle & 1;
                (if vr { 0x08 } else { 0 }) | de as u8
            }
            0x3C7 => 0x03,
            0x3C8 => self.dac_w as u8,
            0x3C9 => {
                let v = self.dac[self.dac_r][self.dac_c];
                self.dac_c += 1;
                if self.dac_c == 3 {
                    self.dac_c = 0;
                    self.dac_r = (self.dac_r + 1) & 255;
                }
                v
            }
            0x3C4 => self.seq_idx,
            0x3C5 => self.seq[(self.seq_idx & 7) as usize],
            0x3D4 => self.crtc_idx,
            0x3D5 => self.crtc[(self.crtc_idx & 31) as usize],
            0x3CE => self.gc_idx,
            0x3CF => self.gc[(self.gc_idx & 15) as usize],
            0x3CC => 0x63,
            _ => 0xFF,
        }
    }

    pub fn port_out(&mut self, port: u16, v: u8) {
        match port {
            0x3C7 => {
                self.dac_r = v as usize;
                self.dac_c = 0;
            }
            0x3C8 => {
                self.dac_w = v as usize;
                self.dac_c = 0;
            }
            0x3C9 => {
                self.dac[self.dac_w][self.dac_c] = v & 0x3F;
                self.dac_c += 1;
                if self.dac_c == 3 {
                    self.dac_c = 0;
                    self.dac_w = (self.dac_w + 1) & 255;
                }
            }
            0x3C4 => self.seq_idx = v,
            0x3C5 => {
                self.seq[(self.seq_idx & 7) as usize] = v;
                if self.seq_idx == 4 && self.mode == 0x13 {
                    // chain-4 disabled => Mode X
                    self.planar = v & 0x08 == 0;
                }
            }
            0x3D4 => self.crtc_idx = v,
            0x3D5 => {
                self.crtc[(self.crtc_idx & 31) as usize] = v;
                if self.crtc_idx == 0x0C || self.crtc_idx == 0x0D {
                    let s = ((self.crtc[0x0C] as u32) << 8) | self.crtc[0x0D] as u32;
                    if self.planar {
                        self.display_start = s * 4;
                    }
                }
            }
            0x3CE => self.gc_idx = v,
            0x3CF => self.gc[(self.gc_idx & 15) as usize] = v,
            _ => {}
        }
    }

    /// Render current screen into an RGB buffer; returns (w, h).
    pub fn render(&mut self, cpu: &Cpu, out: &mut Vec<u32>) -> (usize, usize) {
        self.render_masked(cpu, out, &[])
    }

    /// A full-screen native world only needs the DOS pixels marked as HUD.
    /// An empty/wrong-size mask retains ordinary full-frame conversion.
    pub fn render_masked(&mut self, cpu: &Cpu, out: &mut Vec<u32>, mask: &[bool]) -> (usize, usize) {
        let pal: [u32;256] = std::array::from_fn(|i| {
            let c = self.dac[i];
            let e = |x: u8| ((x as u32) << 2) | ((x as u32) >> 4);
            (e(c[0]) << 16) | (e(c[1]) << 8) | e(c[2])
        });
        if self.mode == 3 {
            // text mode: render 80x25 characters as blocks (debug only)
            let (w, h) = (640usize, 400usize);
            out.resize(w * h, 0);
            let tm = cpu.read_bytes(0xB8000, 4000);
            for y in 0..h {
                for x in 0..w {
                    let cx = x / 8;
                    let cy = y / 16;
                    let ch = tm[(cy * 80 + cx) * 2];
                    let at = tm[(cy * 80 + cx) * 2 + 1];
                    let on = ch > b' ' && font_pixel(ch, (x % 8) as u32, (y % 16) as u32);
                    out[y * w + x] = if on { text_color(at & 15) } else { text_color(at >> 4) };
                }
            }
            return (w, h);
        }
        self.flush_window(cpu);
        let (w, h) = (self.width as usize, self.height as usize);
        out.resize(w * h, 0);
        if self.mode == 0x13 && !self.planar {
            let src = cpu.read_bytes(WINDOW, 64000);
            for i in 0..w * h {
                out[i] = pal[src[i] as usize];
            }
        } else if self.mode == 0x13 {
            // Mode X: planar memory kept in vram by the window handler
            let start = self.display_start as usize;
            for y in 0..h {
                for x in 0..w {
                    let addr = start + y * w + x;
                    out[y * w + x] = pal[self.vram[addr % self.vram.len()] as usize];
                }
            }
        } else {
            let start = self.display_start as usize;
            let pitch = self.pitch as usize;
            for y in 0..h {
                let row = start + y * pitch;
                if row + w > self.vram.len() {
                    break;
                }
                let dst = &mut out[y*w..(y+1)*w];
                let src = &self.vram[row..row+w];
                if mask.len() == w*h {
                    for ((dst,&pixel),&opaque) in dst.iter_mut().zip(src).zip(&mask[y*w..(y+1)*w]) {
                        if opaque { *dst = pal[pixel as usize]; }
                    }
                } else {
                    for (dst,&pixel) in dst.iter_mut().zip(src) { *dst = pal[pixel as usize]; }
                }
            }
        }
        (w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_overlay_conversion_keeps_every_opaque_pixel_and_full_mode_fallback() {
        let mut vga = Vga::new();
        let mut cpu = Cpu::new();
        vga.set_mode(&mut cpu, 0x101, true);
        for (i,p) in vga.vram.iter_mut().take(640*480).enumerate() { *p = i as u8; }
        cpu.write_bytes(WINDOW, &vga.vram[..WIN_SIZE]);
        let mut expected = Vec::new();
        vga.render(&cpu, &mut expected);
        let mask: Vec<bool> = (0..640*480).map(|i| i%11 == 0 || i/640 > 425).collect();
        let mut actual = vec![0xDEADBEEF; expected.len()];
        assert_eq!(vga.render_masked(&cpu, &mut actual, &mask), (640,480));
        for i in 0..actual.len() { assert_eq!(actual[i], if mask[i] {expected[i]} else {0xDEADBEEF}); }
        vga.render_masked(&cpu, &mut actual, &mask[..1]);
        assert_eq!(actual, expected);
        vga.render(&cpu, &mut actual);
        assert_eq!(actual, expected);
    }
}

fn text_color(c: u8) -> u32 {
    const T: [u32; 16] = [
        0x000000, 0x0000AA, 0x00AA00, 0x00AAAA, 0xAA0000, 0xAA00AA, 0xAA5500, 0xAAAAAA, 0x555555, 0x5555FF, 0x55FF55,
        0x55FFFF, 0xFF5555, 0xFF55FF, 0xFFFF55, 0xFFFFFF,
    ];
    T[(c & 15) as usize]
}

/// Tiny procedural "font": draws a box glyph so text mode output is at least visible.
fn font_pixel(_ch: u8, x: u32, y: u32) -> bool {
    (2..14).contains(&y) && (1..7).contains(&x)
}
