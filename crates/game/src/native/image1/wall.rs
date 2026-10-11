//! Walls: column scaler, edge slopes, rotation and `DrawWall`.

use super::*;

/// `_scaleColumnS` / `_scaleColumnMaskS`: draw `count` texels top-down, the
/// last one at `dst`.  `cmap` is a 256-aligned colormap, `v`/`step` are 16.16
/// texture coordinates; with `mask`, texel 0 is transparent.
#[inline]
pub(super) fn scale_column_m(c: &mut Cpu, cmap: u32, count: i32, step: i32, mut v: u32, src: u32, dst: u32, mask: bool) {
    let base = cmap & !0xFF;
    let n = count.max(0) as u32;
    let pitch = render_pitch();
    let mut p = dst.wrapping_sub(n.wrapping_sub(1).wrapping_mul(pitch));
    if NATIVE_TARGET.with(|s| s.get().1) != 0 && !c.wlog_on {
        let mem: &mut [u8; MEM_SIZE + 16] = c.mem.as_mut_slice().try_into().unwrap();
        let addr_mask = (MEM_SIZE - 1) as u32;
        for _ in 0..n {
            let texel = mem[(src.wrapping_add(v >> 16) & addr_mask) as usize];
            if !mask || texel != 0 {
                mem[(p & addr_mask) as usize] = mem[((base | texel as u32) & addr_mask) as usize];
            }
            p = p.wrapping_add(pitch); v = v.wrapping_add(step as u32);
        }
    } else {
        for _ in 0..n {
            let texel = c.rd8(src.wrapping_add(v >> 16));
            if !mask || texel != 0 {
                let col = c.rd8(base | texel as u32);
                c.wr8(p, col);
            }
            p = p.wrapping_add(pitch);
            v = v.wrapping_add(step as u32);
        }
    }
}

#[inline]
pub(super) fn scale_column(c: &mut Cpu, cmap: u32, count: i32, step: i32, v: u32, src: u32, dst: u32) {
    scale_column_m(c, cmap, count, step, v, src, dst, false)
}

/// Wall texture kinds: plain, masked (colour 0 transparent), packed (run-length).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WallKind {
    Solid,
    Masked,
    Packed,
}

pub(super) fn h_draw_mask_wall(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, idx) = (c.r[EAX], c.r[EDX]);
    draw_wall_kind(c, this, idx as i32, WallKind::Masked);
    c.ret_near(0);
    Ok(())
}

/// Slope (16.16) of the edge from (x1,y1) to (x2,y2) per screen column.
pub fn yslope(x1: i32, y1: i32, x2: i32, y2: i32) -> i32 {
    let dx = x1.wrapping_sub(x2).wrapping_abs();
    if dx == 0 {
        return 0;
    }
    let dy = y1.wrapping_sub(y2) as i64;
    ((dy << 16) / dx as i64) as i32
}

pub(super) fn h_yslope(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let y2 = c.stack_arg(0) as i32;
    c.r[EAX] = yslope(c.r[EDX] as i32, c.r[EBX] as i32, c.r[ECX] as i32, y2) as u32;
    c.ret_near(4);
    Ok(())
}

#[inline]
pub(crate) fn mul16(a: i32, b: i32) -> i32 {
    ((a as i64 * b as i64) >> 16) as i32
}

/// Rotate (x, y) around (cx, cy) by a 16-bit angle using the trig tables.
pub fn fixed_rotate(c: &Cpu, this: u32, x: i32, y: i32, cx: i32, cy: i32, angle: u16) -> (i32, i32) {
    let a = (angle >> 5) as i32;
    let cos = rdi(c, this, COS_TABLE, a);
    let sin = rdi(c, this, SIN_TABLE, a);
    let dy = y.wrapping_sub(cy);
    let dx = x.wrapping_sub(cx);
    let nx = cx.wrapping_add(mul16(cos, dx)).wrapping_sub(mul16(sin, dy));
    let ny = mul16(sin, dx).wrapping_add(cy).wrapping_add(mul16(cos, dy));
    (nx, ny)
}

pub(super) fn h_fixed_rotate(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, px, py, cx) = (c.r[EAX], c.r[EDX], c.r[EBX], c.r[ECX] as i32);
    let (cy, angle) = (c.stack_arg(0) as i32, c.stack_arg(1) as u16);
    let (nx, ny) = fixed_rotate(c, this, c.rd32(px) as i32, c.rd32(py) as i32, cx, cy, angle);
    c.wr32(px, nx as u32);
    c.wr32(py, ny as u32);
    c.ret_near(8);
    Ok(())
}

pub(super) fn h_draw_wall(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, idx) = (c.r[EAX], c.r[EDX]);
    draw_wall(c, this, idx as i32, false);
    c.ret_near(0);
    Ok(())
}

pub(super) fn h_draw_pack_wall(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, idx) = (c.r[EAX], c.r[EDX]);
    draw_wall(c, this, idx as i32, true);
    c.ret_near(0);
    Ok(())
}

/// Draw a packed (run-length) texture column top-down from `dst`.
///
/// Column data: i16 run headers; n > 0 is followed by n opaque bytes, n <= 0
/// is a transparent run of |n| pixels.  `pos` is a 16.16 byte offset into the
/// column data (headers included) advanced by `step` per screen pixel.
pub(super) fn draw_packed(c: &mut Cpu, data: u32, count: i32, step: u32, mut pos: u32, mut dst: u32, cmap: u32) {
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
        y += n;
        if opaque {
            for _ in 0..n {
                let texel = c.rd8(data.wrapping_add(pos >> 16));
                let col = c.rd8(cmap.wrapping_add(texel as u32));
                c.wr8(dst, col);
                dst = dst.wrapping_add(pitch);
                pos = pos.wrapping_add(step);
            }
        } else {
            pos = pos.wrapping_add(step.wrapping_mul(n as u32)).wrapping_sub(run as u32);
            dst = dst.wrapping_add((n as u32).wrapping_mul(pitch));
        }
    }
}

/// Wall corner selection shared by the wall drawers: (a, b, cc, d, xl, xr)
/// where a/b are the upper edge corners and cc/d the lower edge corners.
pub(super) fn wall_corners(c: &Cpu, this: u32) -> (i32, i32, i32, i32, i32, i32) {
    let sx = |k: i32| rdi(c, this, WALL_SX, k);
    let sy = |k: i32| rdi(c, this, WALL_SY, k);
    if sx(0) < sx(1) {
        (0, 1, 3, 2, sx(0), sx(1))
    } else if sx(0) == sx(1) {
        let (a, b) = if sy(0) > sy(1) { (0, 1) } else { (1, 0) };
        let (cc, d) = if sy(3) < sy(2) { (3, 2) } else { (2, 3) };
        (a, b, cc, d, sx(a), sx(b))
    } else {
        (1, 0, 2, 3, sx(1), sx(0))
    }
}

/// Draw a textured wall quad (4 projected corners in WALL_SX/WALL_SY).
/// `packed` selects run-length (transparent) textures (`DrawPackWall`).
pub fn draw_wall(c: &mut Cpu, this: u32, idx: i32, packed: bool) {
    draw_wall_kind(c, this, idx, if packed { WallKind::Packed } else { WallKind::Solid })
}

/// `DrawWall` / `DrawMaskWall` / `DrawPackWall`.
pub fn draw_wall_kind(c: &mut Cpu, this: u32, idx: i32, kind: WallKind) {
    let packed = kind == WallKind::Packed;
    let mask = kind == WallKind::Masked;
    let mut fog = rd(c, this, FOG_ON);
    for k in 0..4 {
        let v = rd(c, this, HORIZON).wrapping_sub(rdi(c, this, WALL_SY, k));
        c.wr32(this.wrapping_add(WALL_SY).wrapping_add(4 * k as u32), v as u32);
    }
    let (a, b, cc, d, xl, mut xr) = wall_corners(c, this);
    let sx = |c: &Cpu, k: i32| rdi(c, this, WALL_SX, k);
    let sy = |c: &Cpu, k: i32| rdi(c, this, WALL_SY, k);
    let view_w = rd(c, this, VIEW_W);
    if xr < 0 || xl > view_w - 1 {
        return;
    }
    let mut y_lo = (sy(c, cc) << 16).wrapping_add(0x8000);
    let mut y_hi = (sy(c, a) << 16).wrapping_add(0x8000);
    let s_lo = yslope(sx(c, cc), sy(c, cc), sx(c, d), sy(c, d));
    let s_hi = yslope(sx(c, a), sy(c, a), sx(c, b), sy(c, b));
    let mut xl = xl;
    if xl < 0 {
        let n = xl.wrapping_abs();
        y_lo = y_lo.wrapping_sub(s_lo.wrapping_mul(n));
        y_hi = y_hi.wrapping_sub(s_hi.wrapping_mul(n));
        xl = 0;
    }
    if view_w - 1 < xr {
        xr = view_w - 1;
    }

    let wall_ang = c.rd16(this.wrapping_add(WALL_ANGLE).wrapping_add(2 * idx as u32));
    let t = (c.rd32(this.wrapping_add(FLAT_TEXTURE).wrapping_add(2 * idx as u32)) as i32) >> 16;
    let tex = rdi(c, this, TEXTURE_PTR, t) as u32;
    let tex_h = rdi(c, this, TEX_H, t);
    let tex_w = rdi(c, this, TEX_W, t);
    let cam_u = rd(c, this, CAM_U);
    let cam_v = rd(c, this, CAM_V);
    let (wall_x, wall_y) = fixed_rotate(
        c,
        this,
        rdi(c, this, WALL_X, idx),
        rdi(c, this, WALL_Y, idx),
        cam_u,
        cam_v,
        wall_ang.wrapping_neg(),
    );

    // vertical texture extent above/below the eye (16.16)
    let bottom = rdi(c, this, WALL_BOTTOM, idx).wrapping_sub(rd(c, this, CAM_Z));
    let height = rdi(c, this, WALL_HEIGHT, idx);
    let below_h = height.wrapping_sub(bottom);
    let sum = height.wrapping_add(bottom);
    let h2 = height.wrapping_mul(2);
    let k = height.wrapping_div(tex_w);
    let (v_above, v_below) = if sum >= h2 {
        (h2.wrapping_div(k) << 16, 0)
    } else if sum <= 0 {
        (0, h2.wrapping_div(k) << 16)
    } else {
        (sum.wrapping_div(k) << 16, below_h.wrapping_div(k) << 16)
    };

    let cam_ang = c.rd16(this.wrapping_add(CAM_ANGLE));
    let rel = (cam_ang.wrapping_sub(wall_ang) >> 5) as i32;
    let dxw = wall_x.wrapping_sub(cam_u);
    let d_cos = mul16(rdi(c, this, COS_TABLE, rel), dxw);
    let sin_r = rdi(c, this, SIN_TABLE, rel);

    let mut light = rd(c, this, FLAT_LIGHT);
    let mut colormap = setup_light(c, this, &mut light, &mut fog);

    let tc_bias = rdi(c, this, WALL_LENGTH, idx).wrapping_div(tex_h) >> 1;
    if xl > xr {
        return;
    }
    let tw17 = tex_w << 17;
    let tw2 = tex_w.wrapping_mul(2);
    let th2 = tex_h.wrapping_mul(2);
    let mirrored = c.rd8(this.wrapping_add(WALL_FLAGS + 3).wrapping_add(4 * idx as u32)) & 0x80 != 0;
    let col_angles = rd(c, this, COL_ANGLES) as u32;
    let fb = this.wrapping_add(rd(c, this, FB_OFFSET) as u32);
    let view_h = rd(c, this, VIEW_H);
    let horizon = rd(c, this, HORIZON);
    let pitch = render_pitch();

    for x in xl..=xr {
        let y1 = y_lo >> 16;
        let y2 = y_hi >> 16;
        let h = y2 - y1 + 1;
        if !(y1 > view_h - 1 || y2 < 0 || h <= 0) {
            // texture column from the ray angle through this screen column
            let col_ang = c.rd16(col_angles.wrapping_add(2 * x as u32));
            let ba = cam_ang.wrapping_sub(wall_ang).wrapping_add(col_ang) as i32;
            let tanv = tan_lookup(c, this, ba);
            let uoff = mul16(dxw.wrapping_abs(), tanv);
            let len = rdi(c, this, WALL_LENGTH, idx);
            let tpos = cam_v.wrapping_add(uoff).wrapping_sub(wall_y).wrapping_add(len);
            let mut col = tc_bias.wrapping_add(tpos).wrapping_mul(tex_h).wrapping_div(len);
            if mirrored {
                col = th2 - 1 - col;
            }
            if col > th2 - 1 {
                col = th2 - 1;
            } else if col < 0 {
                col = 0;
            }
            let above = horizon - y1 + 1;
            let below = y2 - horizon;
            let (y1c, clip) = if y1 < 0 { (0, y1.wrapping_abs()) } else { (y1, 0) };
            let y2c = y2.min(view_h - 1);
            let colptr = if packed {
                tex.wrapping_add(c.rd32(tex.wrapping_add((col as u32).wrapping_mul(4))))
            } else {
                tex.wrapping_add(col.wrapping_mul(tw2) as u32)
            };
            if light == 0 {
                let dist = d_cos.wrapping_add(mul16(sin_r, uoff));
                colormap = distance_colormap(c, this, fog, dist);
            }
            let xpix = x as u32;
            let row = |y: i32| fb.wrapping_add((y as u32).wrapping_mul(pitch)).wrapping_add(xpix);
            if above <= 0 || below <= 0 || v_above == 0 || v_below == 0 {
                let step = tw17.wrapping_div(h);
                let v = clip.wrapping_mul(step).wrapping_add(((step as u32) >> 1) as i32);
                if packed {
                    draw_packed(c, colptr, y2c - y1c + 1, step as u32, v as u32, row(y1c), colormap);
                } else {
                    scale_column_m(c, colormap, y2c - y1c + 1, step, v as u32, colptr, row(y2c), mask);
                }
            } else {
                let step1 = v_above.wrapping_div(above);
                let v1 = v_above
                    .wrapping_sub(((step1 as u32) >> 1) as i32)
                    .wrapping_sub((horizon - y1c).wrapping_mul(step1));
                let step2 = v_below.wrapping_div(below);
                let v2 = (((step2 as u32) >> 1) as i32).wrapping_add(v_above);
                if packed {
                    draw_packed(c, colptr, horizon - y1c + 1, step1 as u32, v1 as u32, row(y1c), colormap);
                    draw_packed(c, colptr, y2c - horizon, step2 as u32, v2 as u32, row(horizon + 1), colormap);
                } else {
                    scale_column_m(c, colormap, horizon - y1c + 1, step1, v1 as u32, colptr, row(horizon), mask);
                    scale_column_m(c, colormap, y2c - horizon, step2, v2 as u32, colptr, row(y2c), mask);
                }
            }
        }
        y_lo = y_lo.wrapping_sub(s_lo);
        y_hi = y_hi.wrapping_sub(s_hi);
    }
}

/// Tangent of a 16-bit angle from the quarter-wave table (sign by quadrant).
#[inline]
pub(super) fn tan_lookup(c: &Cpu, this: u32, ba: i32) -> i32 {
    let q = (ba % 0x4000) >> 3;
    if ba < 0x4000 {
        rdi(c, this, TAN_TABLE, q)
    } else if ba < 0x8000 {
        rdi(c, this, TAN_TABLE, 0x7FF - q)
    } else if ba < 0xC000 {
        rdi(c, this, TAN_TABLE, q).wrapping_neg()
    } else {
        rdi(c, this, TAN_TABLE, 0x7FF - q).wrapping_neg()
    }
}
