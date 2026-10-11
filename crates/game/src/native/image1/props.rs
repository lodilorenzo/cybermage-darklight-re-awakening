//! Sprite (prop) drawers: `DrawMaskProp`, `DrawPackProp`, `DrawPackInvis`
//! (cloaked: darkens the background in a checkerboard) and `DrawPackAura`
//! (checkerboard of two colormaps).

use super::*;

const CLIP_LEFT: u32 = 0xAF258;
/// Crosshair target collection: enabled flag, view x offset, count, list (u16)
const TARGET_ON: u32 = 0x4B008;
const TARGET_X0: u32 = 0x4B154;
const TARGET_COUNT: u32 = 0x4B00C;
const TARGET_LIST: u32 = 0x4B010;
const PROP_TARGETABLE: u32 = 0x7D028; // u8 per prop
const FRAME_PARITY: u32 = 0x4B398;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PropMode {
    Mask,
    Pack,
    Invis,
    Aura(u32),
}

pub(super) fn h_draw_mask_prop(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    draw_prop(c, c.r[EAX], c.r[EDX] as i32, c.r[EBX] as i32, PropMode::Mask);
    c.ret_near(0);
    Ok(())
}
pub(super) fn h_draw_pack_prop(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    draw_prop(c, c.r[EAX], c.r[EDX] as i32, c.r[EBX] as i32, PropMode::Pack);
    c.ret_near(0);
    Ok(())
}
pub(super) fn h_draw_pack_invis(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    draw_prop(c, c.r[EAX], c.r[EDX] as i32, c.r[EBX] as i32, PropMode::Invis);
    c.ret_near(0);
    Ok(())
}
pub(super) fn h_draw_pack_aura(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    draw_prop(c, c.r[EAX], c.r[EDX] as i32, c.r[EBX] as i32, PropMode::Aura(c.r[ECX]));
    c.ret_near(0);
    Ok(())
}

/// Distance of prop `p` along the view direction (for shading).
fn prop_view_dist(c: &Cpu, this: u32, p: i32) -> i32 {
    let (cu, cv) = (rd(c, this, CAM_U), rd(c, this, CAM_V));
    let cam = c.rd16(this.wrapping_add(CAM_ANGLE));
    let (x, _) = fixed_rotate(c, this, rdi(c, this, WALL_X, p), rdi(c, this, WALL_Y, p), cu, cv, cam.wrapping_neg());
    x.wrapping_sub(cu)
}

/// Draw a projected sprite quad (corners in WALL_SX/SY).
pub fn draw_prop(c: &mut Cpu, this: u32, p: i32, tex: i32, mode: PropMode) {
    for k in 0..4 {
        let v = rd(c, this, HORIZON).wrapping_sub(rdi(c, this, WALL_SY, k));
        c.wr32(this.wrapping_add(WALL_SY).wrapping_add(4 * k as u32), v as u32);
    }
    let (a, _, cc, _, mut xl, mut xr) = wall_corners(c, this);
    let view_w = rd(c, this, VIEW_W);
    let view_h = rd(c, this, VIEW_H);
    if xr < 0 || xl > view_w - 1 {
        return;
    }
    let mut y_hi = rdi(c, this, WALL_SY, a);
    let mut y_lo = rdi(c, this, WALL_SY, cc);
    let h = y_hi - y_lo + 1;
    if y_lo > view_h - 1 || y_hi < 0 || h <= 0 {
        return;
    }
    let texptr = rdi(c, this, TEXTURE_PTR, tex) as u32;
    let th17 = rdi(c, this, TEX_H, tex) << 17;
    let tw = rdi(c, this, TEX_W, tex);
    let ustep = th17.wrapping_div(xr - xl + 1);
    let mut u = th17.wrapping_sub(ustep / 2);
    if xl < 0 {
        let k = xl.wrapping_abs();
        wr(c, this, CLIP_LEFT, k);
        u = u.wrapping_sub(k.wrapping_mul(ustep));
        xl = 0;
    }
    if view_w - 1 < xr {
        xr = view_w - 1;
    }
    if mode != PropMode::Mask
        && rd(c, this, TARGET_ON) != 0
        && c.rd8(this.wrapping_add(PROP_TARGETABLE).wrapping_add(p as u32)) == 1
        && rd(c, this, TARGET_X0) + xl <= 0x140
        && rd(c, this, TARGET_X0) + xr >= 0x140
    {
        let n = rd(c, this, TARGET_COUNT);
        c.wr16(this.wrapping_add(TARGET_LIST).wrapping_add(2 * n as u32), p as u16);
        wr(c, this, TARGET_COUNT, n + 1);
    }

    // colormaps
    let mut cmap = 0u32;
    let mut cmap_b = 0u32;
    match mode {
        PropMode::Mask | PropMode::Pack => {
            let light = rd(c, this, FLAT_LIGHT);
            if light == 0 {
                if rd(c, this, CUR_CURVE) != 0 {
                    select_curve(c, this, 0);
                }
                let d = prop_view_dist(c, this, p);
                cmap = distance_colormap(c, this, rd(c, this, FOG_ON), d);
            } else {
                let ty = c.rd16(this.wrapping_add(LIGHT_TYPE).wrapping_add(2 * light as u32));
                let param = (c.rd32(this.wrapping_add(LIGHT_PARAM).wrapping_add(2 * light as u32)) as i32) >> 16;
                match ty {
                    0 => cmap = rdi(c, this, SHADE_MAPS, param) as u32,
                    1 => cmap = rdi(c, this, FOG_MAPS, param) as u32,
                    2..=5 => cmap = rdi(c, this, FIXED_MAPS, ty as i32 - 2) as u32,
                    6 => {
                        if light != rd(c, this, CUR_CURVE) {
                            select_curve(c, this, light);
                        }
                        let d = prop_view_dist(c, this, p);
                        cmap = rdi(c, this, SHADE_MAPS, shade_distance(c, this, d)) as u32;
                    }
                    _ => {} // original leaves the colormap uninitialised
                }
            }
        }
        PropMode::Invis => {
            let d = prop_view_dist(c, this, p);
            let s = (shade_distance(c, this, d) + 0x10).min(0x3F);
            cmap = rdi(c, this, SHADE_MAPS, s) as u32;
        }
        PropMode::Aura(k) => {
            cmap = rd(c, this, SHADE_MAPS) as u32;
            cmap_b = rdi(c, this, FIXED_MAPS, k as i32) as u32;
        }
    }

    let clip = if y_lo < 0 {
        let k = y_lo.wrapping_abs();
        y_lo = 0;
        k
    } else {
        0
    };
    if view_h - 1 < y_hi {
        y_hi = view_h - 1;
    }
    let vnum = if mode != PropMode::Mask && flag_byte(c, this, p, 1) & 0x10 != 0 { tw << 16 } else { tw << 17 };
    let vstep = vnum.wrapping_div(h);
    let v0 = clip.wrapping_mul(vstep).wrapping_add(((vstep as u32) >> 1) as i32) as u32;
    let count = y_hi - y_lo + 1;
    let fb = this.wrapping_add(rd(c, this, FB_OFFSET) as u32);
    let pitch = render_pitch();
    let tw2 = tw.wrapping_mul(2);
    let mut par = (c.rd32(this.wrapping_add(FRAME_PARITY)) as i32).wrapping_add(xl) % 2;
    for x in xl..=xr {
        let col = u >> 16;
        match mode {
            PropMode::Mask => {
                let src = texptr.wrapping_add(col.wrapping_mul(tw2) as u32);
                let dst = fb.wrapping_add((y_hi as u32).wrapping_mul(pitch)).wrapping_add(x as u32);
                scale_column_m(c, cmap, count, vstep, v0, src, dst, true);
            }
            _ => {
                let data = texptr.wrapping_add(c.rd32(texptr.wrapping_add((col as u32).wrapping_mul(4))));
                let dst = fb.wrapping_add((y_lo as u32).wrapping_mul(pitch)).wrapping_add(x as u32);
                par ^= 1;
                let par_col = par;
                packed_runs(c, data, count, vstep as u32, v0, dst, |c, pos, p_dst, y0, k| {
                    let texel = c.rd8(data.wrapping_add(pos >> 16));
                    let pp = ((par_col.wrapping_add(y0)) % 2) ^ (k & 1);
                    match mode {
                        PropMode::Pack => {
                            let v = c.rd8(cmap.wrapping_add(texel as u32));
                            c.wr8(p_dst, v);
                        }
                        PropMode::Invis => {
                            if pp != 0 {
                                let bg = c.rd8(p_dst);
                                let v = c.rd8(cmap.wrapping_add(bg as u32));
                                c.wr8(p_dst, v);
                            }
                        }
                        _ => {
                            let m = if pp != 0 { cmap_b } else { cmap };
                            let v = c.rd8(m.wrapping_add(texel as u32));
                            c.wr8(p_dst, v);
                        }
                    }
                });
            }
        }
        u = u.wrapping_sub(ustep);
    }
}

#[inline]
fn flag_byte(c: &Cpu, this: u32, obj: i32, byte: u32) -> u8 {
    c.rd8(this.wrapping_add(WALL_FLAGS + byte).wrapping_add((obj as u32).wrapping_mul(4)))
}

/// Walk a packed column like `draw_packed`, calling `px(cpu, pos, dst, y0, k)`
/// for every opaque pixel (`y0` = rows done before the run, `k` = index in run).
pub(super) fn packed_runs(
    c: &mut Cpu,
    data: u32,
    count: i32,
    step: u32,
    mut pos: u32,
    mut dst: u32,
    mut px: impl FnMut(&mut Cpu, u32, u32, i32, i32),
) {
    let pitch = render_pitch();
    let mut y = 0i32;
    let mut off = 0u32;
    while y < count {
        let hdr = c.rd16(data.wrapping_add(off)) as i16;
        let data_start = off.wrapping_add(2) << 16;
        pos = pos.wrapping_add(0x20000);
        let (len, opaque) = if hdr > 0 {
            off = off.wrapping_add(hdr as u32 + 2);
            (hdr as i32, true)
        } else {
            off = off.wrapping_add(2);
            ((hdr as i32).wrapping_abs() as i16 as i32, false)
        };
        let run = len.wrapping_shl(16);
        let rem = run.wrapping_sub(pos.wrapping_sub(data_start) as i32);
        if rem <= 0 {
            if !opaque {
                pos = pos.wrapping_sub(run as u32);
            }
            continue;
        }
        let mut n = ((rem - 1) as u32 / step) as i32 + 1;
        if y + n > count {
            n = count - y;
        }
        let y0 = y;
        y += n;
        if opaque {
            for k in 0..n {
                px(c, pos, dst, y0, k);
                dst = dst.wrapping_add(pitch);
                pos = pos.wrapping_add(step);
            }
        } else {
            pos = pos.wrapping_add(step.wrapping_mul(n as u32)).wrapping_sub(run as u32);
            dst = dst.wrapping_add((n as u32).wrapping_mul(pitch));
        }
    }
}
