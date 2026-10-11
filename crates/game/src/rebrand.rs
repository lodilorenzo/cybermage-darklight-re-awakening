//! Project branding.  The title art (HIPAK.RSC/FSC/GSC entry 0, raw 640x480)
//! says "DARKLIGHT AWAKENING"; when it is read, the line is re-set as
//! "DARKLIGHT RE-AWAKENING" from the art's own glyphs (R, E and E's middle bar
//! as the hyphen), condensed to the original width.  The title screen (below
//! the menu) and the logo-headed menu screens (key commands, keyboard, options,
//! personal log) also get "DARKLIGHT RE-AWAKENING" in the game's small font 0.
//! Nothing is embedded: the patch is derived from the user's files and skipped
//! where the art differs from the English v1.01F layout.

use crate::le::rt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use x86::Cpu;

pub const NAME: &str = "CyberMage: Darklight Re-Awakening";

const Y0: usize = 163; // band holding the wordmark and its outline
const Y1: usize = 193;
const H: usize = Y1 - Y0;
const RED: [u8; 6] = [19, 34, 43, 84, 117, 23];
const DARK: [u8; 3] = [0, 1, 3];
const MASK_HASH: u64 = 0xc82e8c1e5a595e00; // FNV-1a of the red mask, rows 165..191, cols 284..637

const SUBTITLE: &str = "DARKLIGHT RE-AWAKENING";
const SCREEN: u64 = 640 * 480;
const MENUS: [usize; 4] = [37, 52, 53, 54]; // HIPAK entries with the logo + label header

/// Read-time substitutions for a HIPAK resource (file offsets).
#[derive(Clone)]
pub struct Patch {
    band: (u64, Vec<u8>),
    title: u64,
    menus: Vec<u64>,
}

pub fn title_patch(name: &str, f: &mut File) -> Option<Patch> {
    let base = name.rsplit(['\\', '/', ':']).next()?.to_ascii_uppercase();
    if !matches!(base.as_str(), "HIPAK.RSC" | "HIPAK.FSC" | "HIPAK.GSC") { return None; }
    let mut hdr = [0u8; 8 + 56 * 4];
    let ok = f.read_exact(&mut hdr).is_ok();
    let entry = |i: usize| u32::from_le_bytes(hdr[8 + i * 4..12 + i * 4].try_into().unwrap()) as u64;
    let screen = |i: usize| entry(i + 1).checked_sub(entry(i)) == Some(SCREEN);
    let mut img = vec![0u8; SCREEN as usize];
    let ok = ok && screen(0) && f.seek(SeekFrom::Start(entry(0))).is_ok() && f.read_exact(&mut img).is_ok();
    f.seek(SeekFrom::Start(0)).ok()?;
    if !ok { return None; }
    let band = wordmark(&img)?;
    let menus = MENUS.iter().filter(|&&i| screen(i)).map(|&i| entry(i)).collect();
    Some(Patch { band: (entry(0) + (Y0 * 640) as u64, band), title: entry(0), menus })
}

/// Apply `p` to `buf`, read from file offset `pos`.  Text needs a whole screen in one read.
pub fn apply(p: &Patch, c: &Cpu, pos: u64, buf: &mut [u8]) {
    let end = pos + buf.len() as u64;
    let (at, data) = &p.band;
    let (a, b) = (pos.max(*at), end.min(at + data.len() as u64));
    if a < b {
        buf[(a - pos) as usize..(b - pos) as usize].copy_from_slice(&data[(a - at) as usize..(b - at) as usize]);
    }
    let whole = |at: u64| at >= pos && at + SCREEN <= end;
    let screen = |at: u64| (at - pos) as usize..(at - pos + SCREEN) as usize;
    if whole(p.title) { stamp_title(&mut buf[screen(p.title)], c); }
    for &at in p.menus.iter().filter(|&&at| whole(at)) { stamp_menu(&mut buf[screen(at)], c); }
}

/// Below the title menu (its boxes end at row 444) and above the copyright line.
fn stamp_title(img: &mut [u8], c: &Cpu) {
    if let Some(f) = Font::load(c) { f.draw(img, 320 - f.width(SUBTITLE) / 2, 449, SUBTITLE, true); }
}

/// Under the logo: the label (rows 102..111) moves down 8 rows into the blank
/// strip above the frame, and the subtitle takes its place.
pub fn stamp_menu(img: &mut [u8], c: &Cpu) {
    let Some(f) = Font::load(c) else { return };
    let rows = |y0: usize, y1: usize| (y0..y1).flat_map(|y| img[y * 640..y * 640 + 215].iter().copied()).collect::<Vec<u8>>();
    let label = rows(102, 112);
    if rows(112, 122).iter().any(|&v| v != 1) || !label.contains(&116)
        || label.iter().any(|v| ![1, 23, 34, 116].contains(v)) { return; }
    for y in 102..112 { img[y * 640..y * 640 + 215].fill(1); }
    for y in 0..10 { img[(110 + y) * 640..(110 + y) * 640 + 215].copy_from_slice(&label[y * 215..(y + 1) * 215]); }
    f.draw(img, 112 - f.width(SUBTITLE) / 2, 97, SUBTITLE, false);
}

/// The game's font 0 (the menu header labels), from `_font_table` in guest memory.
struct Font<'a> { data: &'a [u8], height: usize, stride: usize, spacing: isize }

impl<'a> Font<'a> {
    fn load(c: &'a Cpu) -> Option<Self> {
        let table = rt(0x20732F);
        let (ptr, height, stride) = (c.rd32(table), c.rd16(table + 20) as usize, c.rd16(table + 30) as usize);
        if ptr == 0 || height == 0 || height > 64 || stride < 2 || stride > 4096 { return None; }
        Some(Font { data: c.read_bytes(ptr + 14, stride * 256), height, stride, spacing: c.rd16(table + 26) as i16 as isize })
    }
    fn glyph(&self, ch: u8) -> Option<(usize, &[u8])> {
        let g = &self.data[ch as usize * self.stride..(ch as usize + 1) * self.stride];
        let w = u16::from_le_bytes([g[0], g[1]]) as usize;
        (w * self.height + 2 <= g.len()).then(|| (w, &g[2..2 + w * self.height]))
    }
    fn width(&self, text: &str) -> usize {
        text.bytes().filter_map(|ch| self.glyph(ch)).map(|(w, _)| w as isize + self.spacing).sum::<isize>().max(0) as usize
    }
    /// The wordmark's red ramp, dark at the top; optional black drop shadow.
    fn draw(&self, img: &mut [u8], mut x: usize, y: usize, text: &str, shadow: bool) {
        const RAMP: [u8; 6] = [43, 43, 84, 84, 117, 117];
        for ch in text.bytes() {
            let Some((w, px)) = self.glyph(ch) else { continue };
            for pass in (0..=shadow as usize).rev() {
                for yy in 0..self.height { for xx in 0..w {
                    let (tx, ty) = (x + xx + pass, y + yy + pass);
                    if px[yy * w + xx] < 0xFE && tx < 640 && ty < 480 {
                        img[ty * 640 + tx] = if pass == 1 { 0 } else { RAMP[yy.min(RAMP.len() - 1)] };
                    }
                } }
            }
            x = (x as isize + w as isize + self.spacing).max(0) as usize;
        }
    }
}

fn wordmark(img: &[u8]) -> Option<Vec<u8>> {
    let red = |y: usize, x: usize| RED.contains(&img[y * 640 + x]);
    let mut h = 0xcbf29ce484222325u64;
    for y in 165..191 { for x in 284..637 { h = (h ^ red(y, x) as u64).wrapping_mul(0x100000001b3); } }
    if h != MASK_HASH { return None; }
    let band = &img[Y0 * 640..Y1 * 640];
    // Red glyph pixels in columns x0..=x1 plus the dark outline within 2px.
    let mask = |x0: usize, x1: usize| {
        let mut m = vec![false; H * 640];
        for y in 165..191 { for x in x0..=x1 { m[(y - Y0) * 640 + x] = red(y, x); } }
        let r = m.clone();
        for y in 0..H { for x in 0..640 {
            if !DARK.contains(&band[y * 640 + x]) { continue; }
            m[y * 640 + x] |= (y.saturating_sub(2)..(y + 3).min(H))
                .any(|yy| (x.saturating_sub(2)..(x + 3).min(640)).any(|xx| r[yy * 640 + xx]));
        } }
        m
    };
    // Natural-width line: DARKLIGHT, R, E, hyphen, AWAKENING; (red x0, x1, gap after).
    const W: usize = 512;
    let (mut line, mut lmask) = (vec![0u8; H * W], vec![false; H * W]);
    let mut cursor = 2;
    for (x0, x1, gap) in [(286, 443, 13), (330, 342, 1), (542, 555, 2), (0, 0, 2), (453, 632, 0)] {
        if x0 == 0 {
            // Hyphen: E's middle bar (rows 176-177, cols 546..552) inside a black outline.
            for y in 175 - Y0..179 - Y0 { for x in 0..8 {
                let i = y * W + cursor - 1 + x;
                lmask[i] = true;
                line[i] = if (1..7).contains(&x) && (176 - Y0..178 - Y0).contains(&y) {
                    // The bar's top-left pixel is background; reuse the one below.
                    let sy = if x == 1 { 177 - Y0 } else { y };
                    band[sy * 640 + 545 + x]
                } else { 0 };
            } }
            cursor += 6 + gap;
            continue;
        }
        let m = mask(x0, x1);
        for y in 0..H { for x in x0 - 2..=x1 + 2 {
            let i = y * W + cursor + x - x0;
            if m[y * 640 + x] { line[i] = band[y * 640 + x]; lmask[i] = true; }
        } }
        cursor += x1 - x0 + 1 + gap;
    }
    let nat = cursor + 2;
    // Remove the old text (onion-peel mode fill from the surrounding art).
    let mut out = band.to_vec();
    let (a, b) = (mask(286, 443), mask(453, 632));
    let mut hole: Vec<bool> = a.iter().zip(&b).map(|(a, b)| *a || *b).collect();
    while hole.iter().any(|h| *h) {
        let (mut next, mut nh) = (out.clone(), hole.clone());
        for y in 0..H { for x in 0..640 {
            if !hole[y * 640 + x] { continue; }
            let filled = |yy: usize, xx: usize| !hole[yy * 640 + xx];
            let edge = (y > 0 && filled(y - 1, x)) || (y + 1 < H && filled(y + 1, x))
                || (x > 0 && filled(y, x - 1)) || (x + 1 < 640 && filled(y, x + 1));
            if !edge { continue; }
            let mut count = [0u8; 256];
            for yy in y.saturating_sub(1)..(y + 2).min(H) { for xx in x.saturating_sub(1)..(x + 2).min(640) {
                if filled(yy, xx) { count[out[yy * 640 + xx] as usize] += 1; }
            } }
            // Most frequent neighbour; ties go to the lowest index.
            next[y * 640 + x] = (0..256).max_by_key(|&v| (count[v], std::cmp::Reverse(v))).unwrap() as u8;
            nh[y * 640 + x] = false;
        } }
        out = next; hole = nh;
    }
    // Condense horizontally onto cols 284..=636 (nearest column, palette exact).
    let (x0, w) = (284, 353);
    for j in 0..w {
        let s = ((j as f64 + 0.5) * nat as f64 / w as f64) as usize;
        for y in 0..H { if lmask[y * W + s] { out[y * 640 + x0 + j] = line[y * W + s]; } }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_matching_title_art_is_patched() {
        assert!(super::wordmark(&vec![0u8; 640 * 480]).is_none());
        let mut f = std::fs::File::open(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
        assert!(super::title_patch(r"D:\CYBERMAG\LOPAK.RSC", &mut f).is_none());
    }
}
