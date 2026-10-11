//! Camera setup, projection and per-object render dispatch.

use super::*;
use crate::le::rt;
use crate::native::misc;

// ---- camera / view state
const PITCH_POS: u32 = 0xE33F8; // view pitch (horizon offset, +-240)
const HALF_H: u32 = 0xE34B0;
const CAM_COS: u32 = 0xE34BC; // cos/sin of -camera angle (camera-space rotation)
const CAM_SIN: u32 = 0xE34C0;
const ANG_FWD: u32 = 0xE34DC; // u16 table indices: angle, angle+90, angle-90
const ANG_LEFT: u32 = 0xE34DE;
const ANG_RIGHT: u32 = 0xE34E0;
/// Per-view setup (36 bytes per view): framebuffer offset, width, height
const VIEW_SETUP: u32 = 0x4B15C;
const COL_ANGLE_TABLES: u32 = 0xAF278; // 0x500 bytes per view
const SKY_TYPE: u32 = 0x5DFA0;
const SKY_FOG_COLOUR: u32 = 0xBE058;
const FRAME_PARITY: u32 = 0x4B398;
/// Visible cell list (sorted): head, per-entry next / cell / info
const CELL_HEAD: u32 = 0x5BEFC;
const CELL_NEXT: u32 = 0x5CF98;
const CELL_INDEX: u32 = 0x5D798;
const CELL_INFO: u32 = 0x5BF98;
const CELL_LIGHT_MAP: u32 = 0xCB09C; // pointer: light index per cell
const CELL_FIRST_OBJ: u32 = 0x4B3A4; // first object per cell (-1 = none)
const OBJ_ID: u32 = 0x58C74;
const OBJ_NEXT: u32 = 0x5600C;
// ---- per-object (wall/flat/prop) data
const OBJ_COUNT: u32 = 0x76124; // u16 per object: remaining cells before drawing
const PROP_LEVEL: u32 = 0x77758; // u16 per object
const OBJ_TOP: u32 = 0x78D8C;
const WALL_PX: u32 = 0x84A48; // 4 world x per wall (16 bytes per wall)
const WALL_PY: u32 = 0x8FBE8; // 4 world y per wall
// ---- flat corner scratch
const VISIBLE: u32 = 0xE3420; // 4 x RWCPlot result
const CORNER_X: u32 = 0xE3430; // 4 world x
const CORNER_Y: u32 = 0xE3440; // 4 world y

// ---- callees still running as original code
const ATAN2: u32 = 0x06CAD2;

fn call(m: &mut Machine, c: &mut Cpu, link: u32, args: &[u32]) -> Result<u32, Fault> {
    m.call_guest(c, rt(link), args)
}
#[inline]
fn wri(c: &mut Cpu, this: u32, off: u32, i: i32, v: i32) {
    c.wr32(this.wrapping_add(off).wrapping_add((i as u32).wrapping_mul(4)), v as u32)
}
#[inline]
fn flag(c: &Cpu, this: u32, obj: i32, byte: u32) -> u8 {
    c.rd8(this.wrapping_add(WALL_FLAGS + byte).wrapping_add((obj as u32).wrapping_mul(4)))
}

// ============================================================== camera

pub(super) fn h_camera_init(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let this = c.r[EAX];
    let x = (c.r[EDX] & !0xFF).wrapping_add(0x80) as i32;
    let y = (c.r[EBX] & !0xFF).wrapping_add(0x80) as i32;
    let ang = c.stack_arg(0) as u16;
    wr(c, this, CAM_U, x);
    c.wr16(this.wrapping_add(CAM_ANGLE), ang);
    wr(c, this, CAM_V, y);
    wr(c, this, CAM_FRAC_L_, x & 0xFFFF);
    wr(c, this, CAM_FRAC_B_, y & 0xFFFF);
    wr(c, this, CAM_FRAC_R_, 0xFFFF - (x & 0xFFFF));
    wr(c, this, CAM_Z, c.r[ECX] as i32);
    wr(c, this, CAM_FRAC_T_, 0xFFFF - (y & 0xFFFF));
    c.ret_near(4);
    Ok(())
}
const CAM_FRAC_L_: u32 = 0xAF260;
const CAM_FRAC_B_: u32 = 0xAF264;
const CAM_FRAC_R_: u32 = 0xAF268;
const CAM_FRAC_T_: u32 = 0xAF26C;

/// Select view `view` (0 = main, 1 = secondary): framebuffer and size.
pub fn render_init(c: &mut Cpu, this: u32, view: u32) {
    let ca = this.wrapping_add(COL_ANGLE_TABLES).wrapping_add(view.wrapping_mul(0x500));
    c.wr32(this.wrapping_add(COL_ANGLES), ca);
    let p = this.wrapping_add(view.wrapping_mul(36));
    let fb = rd(c, p, VIEW_SETUP);
    let w = rd(c, p, VIEW_SETUP + 4);
    let h = rd(c, p, VIEW_SETUP + 8);
    wr(c, this, FB_OFFSET, fb);
    wr(c, this, VIEW_W, w);
    wr(c, this, HALF_W, w / 2);
    wr(c, this, VIEW_H, h);
    wr(c, this, HALF_H, h / 2);
}

pub(super) fn h_render_init(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let this = c.r[EAX];
    m.display.current_view = c.r[EDX];
    render_init(c, this, c.r[EDX]);
    if m.display.current_view == crate::display::primary_view(c, this) && m.display.mode != 0 && !c.wlog_on {
        if m.display.guest_angles == 0 {
            m.display.guest_angles = m.heap_alloc((crate::display::MODES.iter().map(|&(w,_)| w).max().unwrap() * 2) as u32).expect("native ray angles");
        }
        // Legacy-width rays spanning the native view, so visibility lists include it.
        let w = rd(c, this, VIEW_W);
        let ((_, _, vw, _), f) = native_view(m, c, this);
        ray_angles(c, m.display.guest_angles, w, f as f64 * w as f64 / vw as f64);
        c.wr32(this + COL_ANGLES, m.display.guest_angles);
    }
    c.ret_near(0);
    Ok(())
}

pub fn set_view_horizon(c: &mut Cpu, this: u32) {
    let hh = rd(c, this, HALF_H);
    let v = hh.wrapping_sub(rd(c, this, PITCH_POS).wrapping_mul(hh).wrapping_div(0xF0));
    wr(c, this, HORIZON, v);
}

pub(super) fn h_set_view_horizon(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    set_view_horizon(c, c.r[EAX]);
    c.ret_near(0);
    Ok(())
}

/// Rotate (x, y) around (cx, cy) by the current camera rotation.
pub fn cv_rotate(c: &Cpu, this: u32, x: i32, y: i32, cx: i32, cy: i32) -> (i32, i32) {
    let dx = x.wrapping_sub(cx);
    let dy = y.wrapping_sub(cy);
    let (co, si) = (rd(c, this, CAM_COS), rd(c, this, CAM_SIN));
    let nx = cx.wrapping_add(mul16(co, dx)).wrapping_sub(mul16(si, dy));
    let ny = mul16(si, dx).wrapping_add(cy).wrapping_add(mul16(co, dy));
    (nx, ny)
}

pub(super) fn h_cv_rotate(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, px, py, cx) = (c.r[EAX], c.r[EDX], c.r[EBX], c.r[ECX] as i32);
    let cy = c.stack_arg(0) as i32;
    let (nx, ny) = cv_rotate(c, this, c.rd32(px) as i32, c.rd32(py) as i32, cx, cy);
    c.wr32(px, nx as u32);
    c.wr32(py, ny as u32);
    c.ret_near(4);
    Ok(())
}

/// 64K-unit angle of the vector (x1,y1)->(x2,y2).
pub fn atan64k(m: &mut Machine, c: &mut Cpu, x1: i32, y1: i32, x2: i32, y2: i32) -> Result<i32, Fault> {
    let (dx, dy) = (x2.wrapping_sub(x1), y2.wrapping_sub(y1));
    if dx == 0 && dy == 0 {
        return Ok(0);
    }
    // the CRT atan2 runs as guest code so results match bit-for-bit
    let _ = (m, c);
    let a = (dy as f64).atan2(dx as f64) * 10430.37835047045;
    Ok(fistp_chop(a))
}

/// `__CHP` + `fistp`: truncate toward zero, integer-indefinite on overflow.
fn fistp_chop(v: f64) -> i32 {
    let t = v.trunc();
    if t.is_nan() || t < -2147483648.0 || t > 2147483647.0 {
        i32::MIN
    } else {
        t as i32
    }
}

pub(super) fn h_atan64k(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let y2 = c.stack_arg(0) as i32;
    let r = atan64k(m, c, c.r[EDX] as i32, c.r[EBX] as i32, c.r[ECX] as i32, y2)?;
    c.r[EAX] = r as u32;
    c.ret_near(4);
    Ok(())
}

/// Euclidean distance (truncated).
pub fn distance(x1: i32, y1: i32, x2: i32, y2: i32) -> i32 {
    let (dx, dy) = (x2.wrapping_sub(x1) as f64, y2.wrapping_sub(y1) as f64);
    fistp_chop((dx * dx + dy * dy).sqrt())
}

pub(super) fn h_distance(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let y2 = c.stack_arg(0) as i32;
    c.r[EAX] = distance(c.r[EDX] as i32, c.r[EBX] as i32, c.r[ECX] as i32, y2) as u32;
    c.ret_near(4);
    Ok(())
}

/// Slope (16.16) of x per screen row along the edge (x1,y1)->(x2,y2).
pub fn xslope(x1: i32, y1: i32, x2: i32, y2: i32) -> i32 {
    let dy = y1.wrapping_sub(y2).wrapping_abs();
    if dy == 0 {
        return 0;
    }
    ((x1.wrapping_sub(x2) as i64) << 16).wrapping_div(dy as i64) as i32
}

pub(super) fn h_xslope(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let y2 = c.stack_arg(0) as i32;
    c.r[EAX] = xslope(c.r[EDX] as i32, c.r[EBX] as i32, c.r[ECX] as i32, y2) as u32;
    c.ret_near(4);
    Ok(())
}

// ============================================================== background / scene

/// Sky / background for the current view.
pub fn render_background(c: &mut Cpu, this: u32) {
    let view_h = rd(c, this, VIEW_H);
    let horizon = rd(c, this, HORIZON);
    let mut fill = ((view_h - horizon) / 4).wrapping_mul((rd(c, this, CAM_Z) >> 16) + 1);
    if fill < 0 {
        fill = 0;
    } else if fill > view_h {
        fill = view_h;
    }
    if NATIVE_TARGET.with(|s| s.get().1) != 0 { fill = fill.min((view_h - horizon).max(0)); }
    let fb = this.wrapping_add(rd(c, this, FB_OFFSET) as u32);
    let view_w = rd(c, this, VIEW_W) as u32;
    let angle = c.rd16(this.wrapping_add(CAM_ANGLE));
    let pitch = render_pitch();
    let row = |y: i32| fb.wrapping_add((y as u32).wrapping_mul(pitch));
    match rd(c, this, SKY_TYPE) {
        4 => {
            let mut sky = horizon + view_h / 4;
            if sky > view_h - 1 {
                sky = view_h - 1;
            } else if sky < 1 {
                sky = 1;
            }
            draw_backdrop(c, this, angle, rd(c, this, FB_OFFSET) as u32, sky);
            misc::memset2(c, row(sky), 0, view_w, (view_h - sky) as u32, pitch);
        }
        3 => {
            if fill != 0 {
                misc::memset2(c, fb, 0, view_w, (horizon + fill) as u32, pitch);
            }
        }
        2 if rd(c, this, FOG_ON) != 0 => {
            if fill != 0 {
                let col = c.rd8(this.wrapping_add(SKY_FOG_COLOUR));
                misc::memset2(c, fb, col, view_w, (horizon + fill) as u32, pitch);
            }
        }
        _ => {
            draw_backdrop(c, this, angle, rd(c, this, FB_OFFSET) as u32, horizon);
            if fill != 0 {
                misc::memset2(c, row(horizon), 0, view_w, fill as u32, pitch);
            }
        }
    }
}

pub(super) fn h_render_background(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    render_background(c, c.r[EAX]);
    c.ret_near(0);
    Ok(())
}

/// Set up camera-space trig and render all visible cells front to back.
pub fn render_scene(m: &mut Machine, c: &mut Cpu, this: u32) -> Result<(), Fault> {
    let cam = c.rd16(this.wrapping_add(CAM_ANGLE));
    let a = (cam.wrapping_neg() >> 5) as i32;
    wr(c, this, CAM_COS, rdi(c, this, COS_TABLE, a));
    wr(c, this, CAM_SIN, rdi(c, this, SIN_TABLE, a));
    let fwd = cam >> 5;
    let left = cam.wrapping_add(0x4000) >> 5;
    let right = cam.wrapping_sub(0x4000) >> 5;
    c.wr16(this.wrapping_add(ANG_FWD), fwd);
    c.wr16(this.wrapping_add(ANG_LEFT), left);
    c.wr16(this.wrapping_add(ANG_RIGHT), right);
    wr(c, this, U_COS, rdi(c, this, COS_TABLE, fwd as i32));
    wr(c, this, V_COS, rdi(c, this, SIN_TABLE, fwd as i32));
    wr(c, this, U_SIN, rdi(c, this, COS_TABLE, left as i32));
    wr(c, this, V_SIN, rdi(c, this, SIN_TABLE, left as i32));
    wr(c, this, U_STEP, rdi(c, this, COS_TABLE, right as i32));
    wr(c, this, V_STEP, rdi(c, this, SIN_TABLE, right as i32));
    let par = c.rd8(this.wrapping_add(FRAME_PARITY)) ^ 1;
    c.wr8(this.wrapping_add(FRAME_PARITY), par);
    let mut k = rd(c, this, CELL_HEAD);
    while k != -1 {
        let cell = rdi(c, this, CELL_INDEX, k);
        let lm = (rd(c, this, CELL_LIGHT_MAP) as u32).wrapping_add(cell as u32);
        let light = c.rd8(lm) as i32;
        wr(c, this, FLAT_LIGHT, light);
        let mut o = rdi(c, this, CELL_FIRST_OBJ, cell);
        while o != -1 {
            let info = rdi(c, this, CELL_INFO, k) as u32;
            render_object(m, c, this, rdi(c, this, OBJ_ID, o), info)?;
            o = rdi(c, this, OBJ_NEXT, o);
        }
        k = rdi(c, this, CELL_NEXT, k);
    }
    Ok(())
}

pub(super) fn h_render_scene(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let this = c.r[EAX];
    if m.display.current_view == crate::display::primary_view(c, this) {
        m.display.pending = true;
        if m.display.mode != 0 && !c.wlog_on { render_native(m, c, this)?; }
        // Target detection and the guest HUD use the original view projection.
        c.wr32(this + COL_ANGLES, this + COL_ANGLE_TABLES + m.display.current_view * 0x500);
    }
    render_scene(m, c, this)?;
    c.ret_near(0);
    Ok(())
}

/// Render one wall / flat / prop (objects spanning several cells are drawn
/// once, when their cell countdown reaches zero).
pub fn render_object(m: &mut Machine, c: &mut Cpu, this: u32, obj: i32, info: u32) -> Result<(), Fault> {
    let f3 = flag(c, this, obj, 3);
    let cnt_addr = this.wrapping_add(OBJ_COUNT).wrapping_add((obj as u32).wrapping_mul(2));
    if f3 & 4 != 0 {
        return render_wall(m, c, this, obj);
    }
    if f3 & 8 != 0 {
        let n = c.rd16(cnt_addr).wrapping_sub(1);
        c.wr16(cnt_addr, n);
        if n == 0 {
            render_wall(m, c, this, obj)?;
        }
        return Ok(());
    }
    if f3 & 0x20 != 0 {
        if rdi(c, this, OBJ_TOP, obj) > rd(c, this, CAM_Z) && flag(c, this, obj, 1) & 8 != 0 {
            if c.rd16(cnt_addr) == 0 {
                return Ok(());
            }
            let lvl = c.rd16(this.wrapping_add(PROP_LEVEL).wrapping_add((obj as u32).wrapping_mul(2)));
            let hi = info >> 24;
            let mid = (info >> 16) & 0xFF;
            if hi as i32 > (lvl >> 8) as i32 + 1 || mid != (lvl & 0xFF) as u32 {
                return Ok(());
            }
            render_prop(m, c, this, obj)?;
            c.wr16(cnt_addr, 0);
        } else {
            let n = c.rd16(cnt_addr).wrapping_sub(1);
            c.wr16(cnt_addr, n);
            if n == 0 {
                render_prop(m, c, this, obj)?;
            }
        }
        return Ok(());
    }
    if f3 & 2 != 0 {
        let z = rdi(c, this, WALL_BOTTOM, obj);
        let cz = rd(c, this, CAM_Z);
        if z < cz {
            if flag(c, this, obj, 2) & 0x40 != 0 {
                render_flat(m, c, this, obj, false)?;
            }
        } else if z > cz && flag(c, this, obj, 2) & 0x80 != 0 {
            render_flat(m, c, this, obj, true)?;
        }
    }
    Ok(())
}

pub(super) fn h_render_object(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, obj, info) = (c.r[EAX], c.r[EDX] as i32, c.r[EBX]);
    render_object(m, c, this, obj, info)?;
    c.ret_near(0);
    Ok(())
}

// ============================================================== projection

/// Project world point (x, y) with vertical extent `h` to screen; writes the
/// screen x / y to `ox` / `oy`.  Fails when the point is not in front of
/// the camera by more than `min_dist`.
pub fn rwc_plot(c: &mut Cpu, this: u32, x: i32, y: i32, h: i32, ox: u32, oy: u32, min_dist: i32) -> bool {
    let (cu, cv) = (rd(c, this, CAM_U), rd(c, this, CAM_V));
    let (x, y) = cv_rotate(c, this, x, y, cu, cv);
    let rx = x.wrapping_sub(cu);
    let ry = y.wrapping_sub(cv);
    if rx <= min_dist {
        return false;
    }
    let hw = rd(c, this, HALF_W);
    let f = focal(c, this);
    let projected = if NATIVE_TARGET.with(|s| s.get().1) != 0 {
        (ry as i64 * f as i64 / rx as i64) as i32
    } else { ry.wrapping_mul(f).wrapping_div(rx) };
    let sx = hw.wrapping_sub(projected);
    c.wr32(ox, sx as u32);
    c.wr32(oy, h.wrapping_div(rx) as u32);
    true
}

pub(super) fn h_rwc_plot(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, x, y, h) = (c.r[EAX], c.r[EDX] as i32, c.r[EBX] as i32, c.r[ECX] as i32);
    let (ox, oy, md) = (c.stack_arg(0), c.stack_arg(1), c.stack_arg(2) as i32);
    c.r[EAX] = rwc_plot(c, this, x, y, h, ox, oy, md) as u32;
    c.ret_near(0xC);
    Ok(())
}

/// Clip a wall end point that is behind the camera: slide it along the wall
/// (angle `ang`) to the edge of the view and project it.
pub fn new_point(c: &mut Cpu, this: u32, x: i32, y: i32, ang: u16, h: i32, ox: u32, oy: u32) -> bool {
    let (cu, cv) = (rd(c, this, CAM_U), rd(c, this, CAM_V));
    let (x, _) = fixed_rotate(c, this, x, y, cu, cv, ang.wrapping_neg());
    let ba = c.rd16(this.wrapping_add(CAM_ANGLE)).wrapping_sub(ang) as i32;
    let q = (ba % 0x4000) >> 3;
    let rx = x.wrapping_sub(cu);
    let y = if ba < 0x4000 {
        cv.wrapping_sub(mul16(rx, rdi(c, this, TAN_TABLE, 0x7FF - q))).wrapping_add(0x800)
    } else if ba < 0x8000 {
        mul16(rx, rdi(c, this, TAN_TABLE, q)).wrapping_add(cv).wrapping_add(0x800)
    } else if ba < 0xC000 {
        cv.wrapping_sub(mul16(rx, rdi(c, this, TAN_TABLE, 0x7FF - q))).wrapping_sub(0x800)
    } else {
        mul16(rx, rdi(c, this, TAN_TABLE, q)).wrapping_add(cv).wrapping_sub(0x800)
    };
    let (x, y) = fixed_rotate(c, this, x, y, cu, cv, ang);
    rwc_plot(c, this, x, y, h, ox, oy, 0x100)
}

pub(super) fn h_new_point(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, x, y, ang) = (c.r[EAX], c.r[EDX] as i32, c.r[EBX] as i32, c.r[ECX] as u16);
    let (h, ox, oy) = (c.stack_arg(0) as i32, c.stack_arg(1), c.stack_arg(2));
    c.r[EAX] = new_point(c, this, x, y, ang, h, ox, oy) as u32;
    c.ret_near(0xC);
    Ok(())
}

/// Clip the flat edge from corner a towards corner b (b behind the camera)
/// against the view and project the intersection.  Returns true on FAILURE
/// (as the original).
pub fn find_point(c: &mut Cpu, this: u32, xa: i32, ya: i32, xb: i32, ox: u32, oy: u32, h: i32) -> bool {
    let cam = c.rd16(this.wrapping_add(CAM_ANGLE)) as i32;
    // DOS clips at 479 horizon-relative rows. At native resolutions that plane
    // can land inside the viewport, chopping off the near floor/ceiling.
    let rows = if NATIVE_TARGET.with(|s| s.get().1) != 0 {
        (rd(c, this, VIEW_H) - 1 - rd(c, this, HORIZON)).max(1)
    } else { 0x1DF };
    let mut d = h.wrapping_div(rows);
    if d < 0x1000 {
        d = 0x1000;
    }
    let a = cam >> 5;
    let cx = rd(c, this, CAM_U).wrapping_add(mul16(d, rdi(c, this, COS_TABLE, a)));
    let cy = rd(c, this, CAM_V).wrapping_add(mul16(d, rdi(c, this, SIN_TABLE, a)));
    // (cam +- 0x4000) mod 0x4000 == cam mod 0x4000
    let q = (cam & 0x3FFF) >> 3;
    let t = |c: &Cpu, i: i32| rdi(c, this, TAN_TABLE, i);
    let (rx, ry);
    if xb == xa {
        let dn = xa.wrapping_sub(cx).wrapping_abs();
        if dn == 0 {
            (rx, ry) = (cx, cy);
        } else if cam < 0x4000 {
            let k = mul16(dn, t(c, 0x7FF - q));
            if cx < xa {
                (rx, ry) = (dn.wrapping_add(cx), cy.wrapping_sub(k));
            } else {
                (rx, ry) = (cx.wrapping_sub(dn), cy.wrapping_add(k));
            }
        } else {
            let (xp, xm) = (dn.wrapping_add(cx), cx.wrapping_sub(dn));
            if cam < 0x8000 {
                let k = mul16(dn, t(c, q));
                if cx < xa {
                    (rx, ry) = (xp, cy.wrapping_add(k));
                } else {
                    (rx, ry) = (xm, cy.wrapping_sub(k));
                }
            } else if cam < 0xC000 {
                let k = mul16(dn, t(c, 0x7FF - q));
                if cx > xa {
                    (rx, ry) = (xm, cy.wrapping_add(k));
                } else {
                    (rx, ry) = (xp, cy.wrapping_sub(k));
                }
            } else {
                let k = mul16(dn, t(c, q));
                if cx > xa {
                    (rx, ry) = (xm, cy.wrapping_sub(k));
                } else {
                    (rx, ry) = (xp, cy.wrapping_add(k));
                }
            }
        }
    } else {
        let dn = ya.wrapping_sub(cy).wrapping_abs();
        if dn == 0 {
            (rx, ry) = (cx, cy);
        } else {
            let (yp, ym) = (dn.wrapping_add(cy), cy.wrapping_sub(dn));
            if cam < 0x4000 {
                let k = mul16(dn, t(c, q));
                if cy > ya {
                    (rx, ry) = (cx.wrapping_add(k), ym);
                } else {
                    (rx, ry) = (cx.wrapping_sub(k), yp);
                }
            } else if cam < 0x8000 {
                let k = mul16(dn, t(c, 0x7FF - q));
                if cy < ya {
                    (rx, ry) = (cx.wrapping_add(k), yp);
                } else {
                    (rx, ry) = (cx.wrapping_sub(k), ym);
                }
            } else if cam < 0xC000 {
                let k = mul16(dn, t(c, q));
                if cy < ya {
                    (rx, ry) = (cx.wrapping_sub(k), yp);
                } else {
                    (rx, ry) = (cx.wrapping_add(k), ym);
                }
            } else {
                let k = mul16(dn, t(c, 0x7FF - q));
                if cy > ya {
                    (rx, ry) = (cx.wrapping_sub(k), ym);
                } else {
                    (rx, ry) = (cx.wrapping_add(k), yp);
                }
            }
        }
    }
    !rwc_plot(c, this, rx, ry, h, ox, oy, 0)
}

pub(super) fn h_find_point(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, xa, ya, xb) = (c.r[EAX], c.r[EDX] as i32, c.r[EBX] as i32, c.r[ECX] as i32);
    let (ox, oy, h) = (c.stack_arg(1), c.stack_arg(2), c.stack_arg(3) as i32);
    c.r[EAX] = find_point(c, this, xa, ya, xb, ox, oy, h) as u32;
    c.ret_near(0x10);
    Ok(())
}

/// Build the left / right edge chains of the projected flat polygon
/// (corners in CORNER_X/Y, projections in WALL_SX/SY, VISIBLE flags).
pub fn connect_points(c: &mut Cpu, this: u32, h: i32) -> bool {
    let vis = |c: &Cpu, i: i32| rdi(c, this, VISIBLE, i) != 0;
    let sx = |c: &Cpu, i: i32| rdi(c, this, WALL_SX, i);
    let sy = |c: &Cpu, i: i32| rdi(c, this, WALL_SY, i);
    let wx = |c: &Cpu, i: i32| rdi(c, this, CORNER_X, i);
    let wy = |c: &Cpu, i: i32| rdi(c, this, CORNER_Y, i);
    let a = |off: u32, i: i32| this.wrapping_add(off).wrapping_add(4 * i as u32);
    let mut s = 0;
    while s < 4 && !vis(c, s) {
        s += 1;
    }
    if s == 4 {
        return false;
    }
    for i in s + 1..4 {
        if vis(c, i) && sy(c, i) < sy(c, s) {
            s = i;
        }
    }
    let x0 = sx(c, s);
    wr(c, this, POLY_X0, x0);
    let top = sy(c, s) + rd(c, this, HORIZON);
    wr(c, this, POLY_TOP, top);

    // one chain: corners visited in `order`, written to (x, end, slope, last)
    let mut chain = |c: &mut Cpu, order: [i32; 3], xo: u32, eo: u32, so: u32, lasto: u32| -> bool {
        let mut prev_x = x0;
        let mut prev_y = top;
        let mut prev_c = s;
        for k in 0..3 {
            let cc = order[k];
            if !vis(c, cc) {
                if find_point(c, this, wx(c, cc), wy(c, cc), wx(c, prev_c), a(xo, k as i32), a(eo, k as i32), h) {
                    return false;
                }
                let e = rdi(c, this, eo, k as i32) + rd(c, this, HORIZON);
                wri(c, this, eo, k as i32, e);
                if k < 2 {
                    wr(c, this, lasto, k as i32);
                }
                let sl = xslope(rdi(c, this, xo, k as i32), e, prev_x, prev_y);
                wri(c, this, so, k as i32, sl);
                return true;
            }
            let x = sx(c, cc);
            let e = sy(c, cc) + rd(c, this, HORIZON);
            wri(c, this, xo, k as i32, x);
            wri(c, this, eo, k as i32, e);
            if k == 1 {
                wr(c, this, lasto, 2);
            }
            let sl = xslope(x, e, prev_x, prev_y);
            wri(c, this, so, k as i32, sl);
            prev_x = x;
            prev_y = e;
            prev_c = cc;
        }
        true
    };
    if !chain(c, [(s + 1) % 4, (s + 2) % 4, (s + 3) % 4], L_X, L_END, L_SLOPE, L_LAST) {
        return false;
    }
    chain(c, [(s + 3) % 4, (s + 2) % 4, (s + 1) % 4], R_X, R_END, R_SLOPE, R_LAST)
}

pub(super) fn h_connect_points(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, h) = (c.r[EAX], c.r[EDX] as i32);
    c.r[EAX] = connect_points(c, this, h) as u32;
    c.ret_near(0);
    Ok(())
}

// ============================================================== walls / flats

/// Project a wall's four corners (with end-point clipping) and draw it.
pub fn render_wall(m: &mut Machine, c: &mut Cpu, this: u32, w: i32) -> Result<(), Fault> {
    let hw = focal(c, this);
    let base = rdi(c, this, WALL_BOTTOM, w).wrapping_sub(rd(c, this, CAM_Z));
    let height = rdi(c, this, WALL_HEIGHT, w);
    let ang = c.rd16(this.wrapping_add(WALL_ANGLE).wrapping_add(2 * w as u32));
    let px = |c: &Cpu, k: u32| c.rd32(this.wrapping_add(WALL_PX + 4 * k).wrapping_add(16 * w as u32)) as i32;
    let py = |c: &Cpu, k: u32| c.rd32(this.wrapping_add(WALL_PY + 4 * k).wrapping_add(16 * w as u32)) as i32;
    let ox = |k: u32| this.wrapping_add(WALL_SX + 4 * k);
    let oy = |k: u32| this.wrapping_add(WALL_SY + 4 * k);
    let mut ok = [false; 4];
    // bottom edge (corners 0,1), then top edge (corners 2,3)
    for (pair, h) in [(0u32, base.wrapping_sub(height).wrapping_mul(hw)), (2, base.wrapping_add(height).wrapping_mul(hw))] {
        let (i, j) = (pair, pair + 1);
        ok[i as usize] = rwc_plot(c, this, px(c, i), py(c, i), h, ox(i), oy(i), 0x400);
        ok[j as usize] = rwc_plot(c, this, px(c, j), py(c, j), h, ox(j), oy(j), 0x400);
        if ok[j as usize] && !ok[i as usize] {
            ok[i as usize] = new_point(c, this, px(c, i), py(c, i), ang, h, ox(i), oy(i));
        }
        if ok[i as usize] && !ok[j as usize] {
            ok[j as usize] = new_point(c, this, px(c, j), py(c, j), ang, h, ox(j), oy(j));
        }
    }
    if ok.iter().all(|&v| v) {
        if flag(c, this, w, 3) & 0x40 != 0 {
            if flag(c, this, w, 2) & 8 != 0 {
                draw_wall(c, this, w, true);
            } else {
                draw_wall_kind(c, this, w, WallKind::Masked);
            }
        } else {
            draw_wall(c, this, w, false);
        }
    }
    Ok(())
}

pub(super) fn h_render_wall(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, w) = (c.r[EAX], c.r[EDX] as i32);
    render_wall(m, c, this, w)?;
    c.ret_near(0);
    Ok(())
}

/// Project a floor (or ceiling, `top`) rectangle and draw it.
pub fn render_flat(m: &mut Machine, c: &mut Cpu, this: u32, f: i32, top: bool) -> Result<(), Fault> {
    if top {
        let p = rd(c, this, PITCH_POS).wrapping_neg();
        wr(c, this, PITCH_POS, p);
        set_view_horizon(c, this);
    }
    let z = rdi(c, this, WALL_BOTTOM, f);
    let cz = rd(c, this, CAM_Z);
    let h = if top { z.wrapping_sub(cz) } else { cz.wrapping_sub(z) }.wrapping_mul(focal(c, this));
    let (cx, cy) = (rdi(c, this, WALL_X, f), rdi(c, this, WALL_Y, f));
    let (hx, hy) = (rdi(c, this, WALL_LENGTH, f), rdi(c, this, WALL_HEIGHT, f));
    let (x0, x1) = (cx.wrapping_sub(hx), hx.wrapping_add(cx));
    let (y0, y1) = (cy.wrapping_sub(hy), cy.wrapping_add(hy));
    for (k, (x, y)) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].into_iter().enumerate() {
        wri(c, this, CORNER_X, k as i32, x);
        wri(c, this, CORNER_Y, k as i32, y);
    }
    for k in 0..4u32 {
        let (x, y) = (rdi(c, this, CORNER_X, k as i32), rdi(c, this, CORNER_Y, k as i32));
        let v = rwc_plot(c, this, x, y, h, this + WALL_SX + 4 * k, this + WALL_SY + 4 * k, 0x400);
        wri(c, this, VISIBLE, k as i32, v as i32);
    }
    if connect_points(c, this, h) {
        if top {
            draw_flat(c, this, f as u32, h, true);
        } else if flag(c, this, f, 3) & 0x40 != 0 {
            draw_flat_ex(c, this, f as u32, h, false, true);
        } else {
            draw_flat(c, this, f as u32, h, false);
        }
    }
    if top {
        let p = rd(c, this, PITCH_POS).wrapping_neg();
        wr(c, this, PITCH_POS, p);
        set_view_horizon(c, this);
    }
    Ok(())
}

pub(super) fn h_render_flat(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, f) = (c.r[EAX], c.r[EDX] as i32);
    render_flat(m, c, this, f, false)?;
    c.ret_near(0);
    Ok(())
}

pub(super) fn h_render_top_flat(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, f) = (c.r[EAX], c.r[EDX] as i32);
    render_flat(m, c, this, f, true)?;
    c.ret_near(0);
    Ok(())
}

#[allow(dead_code)]
const _UNUSED: u32 = ATAN2;

/// FOV grows horizontally with aspect ratio; vertical projection stays fixed.
fn ray_angles(c: &mut Cpu, dst: u32, w: i32, f: f64) {
    for x in 0..w {
        let a = ((w as f64 / 2.0 - x as f64 - 0.5) / f).atan() * 65536.0 / std::f64::consts::TAU;
        c.wr16(dst + x as u32 * 2, (a.round() as i32) as u16);
    }
}

/// Output rectangle and focal length of the current view's native pass.
/// Vertical projection matches the legacy view; wider views see further sideways.
fn native_view(m: &Machine, c: &Cpu, this: u32) -> ((usize, usize, usize, usize), i32) {
    let (w, h) = (rd(c, this, VIEW_W), rd(c, this, VIEW_H));
    let view = m.display.current_view * 36;
    let (x, y) = (c.rd32(this + view + 0x4B154) as usize, c.rd32(this + view + 0x4B158) as usize);
    let view = (x, y, w as usize, h as usize);
    let wide = crate::display::view_widens(view, c.rd32(this + 0x1444F0));
    let rect = crate::display::world_rect(view, m.display.size(), wide);
    (rect, (w as i64 * rect.3 as i64 / (2 * h as i64)) as i32)
}

fn render_native(m: &mut Machine, c: &mut Cpu, this: u32) -> Result<(), Fault> {
    let (rect, output_focal) = native_view(m, c, this);
    let (vw,vh) = m.display.raster_size(rect.2,rect.3);
    let f = (output_focal as i64 * vh as i64 / rect.3 as i64).max(1) as i32;
    if m.display.guest_fb == 0 {
        m.display.guest_fb = m.heap_alloc(crate::display::MODES.iter().map(|&(w,h)| w * h).max().unwrap() as u32).expect("native framebuffer");
    }
    // ponytail: two world passes retain DOS UI/targeting; port the UI if frame time demands it.
    // Only renderer scratch/state is restored. Gameplay and the guest framebuffer
    // are never redirected or resized, including the secondary vehicle view.
    let start = this + 0x4B000;
    let saved = c.read_bytes(start, (0xE34E4 - 0x4B000) as usize).to_vec();
    let fb = m.display.guest_fb;
    c.mem[fb as usize..fb as usize + vw * vh].fill(0);
    wr(c, this, FB_OFFSET, fb.wrapping_sub(this) as i32);
    wr(c, this, VIEW_W, vw as i32);
    wr(c, this, HALF_W, vw as i32 / 2);
    wr(c, this, VIEW_H, vh as i32);
    wr(c, this, HALF_H, vh as i32 / 2);
    wr(c, this, 0x4B008, 0); // no target-list side effects in the presentation pass
    ray_angles(c, m.display.guest_angles, vw as i32, f as f64);
    c.wr32(this + COL_ANGLES, m.display.guest_angles);
    NATIVE_TARGET.with(|s| s.set((vw as u32, f)));
    set_view_horizon(c, this);
    if rd(c, this, SKY_TYPE) != 0 { render_background(c, this); }
    let result = render_scene(m, c, this);
    NATIVE_TARGET.with(|s| s.set((640, 0)));
    if result.is_ok() {
        m.display.pending_world.clear();
        m.display.pending_world.extend_from_slice(c.read_bytes(fb, vw * vh));
        m.display.pending_rect = rect;
        m.display.pending_world_size = (vw,vh);
    }
    c.write_bytes(start, &saved);
    result
}

#[cfg(test)]
mod native_tests {
    use super::*;
    #[test]
    fn native_flat_clipping_stays_beyond_the_viewport_at_all_pitches() {
        let mut c = Cpu::new();
        let this = 0x100000;
        wr(&mut c,this,CAM_COS,65536);
        wri(&mut c,this,COS_TABLE,0,65536);
        for &(w,h) in crate::display::MODES {
            let (w,h) = (w as i32,h as i32);
            let f = h * 2 / 3;
            NATIVE_TARGET.with(|s| s.set((w as u32,f)));
            wr(&mut c,this,HALF_W,w/2);
            wr(&mut c,this,HALF_H,h/2);
            wr(&mut c,this,VIEW_H,h);
            for pitch in [-224,0,224] {
                wr(&mut c,this,PITCH_POS,pitch);
                set_view_horizon(&mut c,this);
                // Horizontal floor edge crossing the camera's near plane.
                assert!(!find_point(&mut c,this,-65536,-65536,65536,0x100,0x104,65536*f));
                assert!(c.rd32(0x104) as i32 + rd(&c,this,HORIZON) >= h-1,
                    "clipped floor edge inside {w}x{h} at pitch {pitch}");
            }
        }
        // Keep the original clipping plane on the DOS/differential path.
        NATIVE_TARGET.with(|s| s.set((640,0)));
        wr(&mut c,this,HALF_W,320);
        assert!(!find_point(&mut c,this,-65536,-65536,65536,0x100,0x104,65536*320));
        assert_eq!(c.rd32(0x104),479);
    }
    #[test]
    fn wide_projection_keeps_vertical_fov_and_centres_rays() {
        let mut c = Cpu::new();
        let this = 0x100000;
        wr(&mut c, this, HALF_W, 960);
        wr(&mut c, this, CAM_COS, 65536);
        NATIVE_TARGET.with(|s| s.set((1920, 720)));
        assert!(rwc_plot(&mut c, this, 65536, 65536, 720 * 65536, 0x100, 0x104, 0));
        assert_eq!(c.rd32(0x100), 240);
        assert_eq!(c.rd32(0x104), 720);
        ray_angles(&mut c, 0x200, 1920, 720.0);
        assert_eq!(c.rd16(0x200) as i16, -(c.rd16(0x200 + 1919 * 2) as i16));
        assert!(c.rd16(0x200) > 8192); // wider than the original 90-degree view
        NATIVE_TARGET.with(|s| s.set((640, 0)));
    }
}

// ============================================================== props
const PROP_ROTS: u32 = 0x83412; // i8 per prop: rotation frames (or -3 / -9 special)
const PROP_KIND: u32 = 0x83F2C; // u8 per prop
const FRAME_HALF_W: u32 = 0xA923C; // per texture frame
const FRAME_DEPTH: u32 = 0xAC23C;
const FRAME_BASE: u32 = 0xAAA3C;
const FRAME_HEIGHT: u32 = 0xADA3C;

/// Texture frame of a prop as seen from the camera (rotation sprites).
pub fn prop_frame(m: &mut Machine, c: &mut Cpu, this: u32, p: i32) -> Result<i32, Fault> {
    let mut tex = (c.rd32(this.wrapping_add(FLAT_TEXTURE).wrapping_add(2 * p as u32)) as i32) >> 16;
    let rots = c.rd8(this.wrapping_add(PROP_ROTS).wrapping_add(p as u32)) as i8 as i32;
    let (wx, wy) = (rdi(c, this, WALL_X, p), rdi(c, this, WALL_Y, p));
    let (cu, cv) = (rd(c, this, CAM_U), rd(c, this, CAM_V));
    let heading = c.rd16(this.wrapping_add(WALL_ANGLE).wrapping_add(2 * p as u32));
    if rots > 1 {
        let a = atan64k(m, c, wx, wy, cu, cv)?;
        let rel = (a & 0xFFFF).wrapping_sub(heading as i16 as i32);
        let half = 0x8000i32.wrapping_div(rots);
        let ang = rel.wrapping_add(half) as u16 as i32;
        tex += ang.wrapping_div(0x10000i32.wrapping_div(rots));
    } else if rots == -3 {
        let a = atan64k(m, c, wx, wy, cu, cv)?;
        let e = ((a as u16).wrapping_sub(heading).wrapping_add(0x1000) >> 13) as i32;
        if e > 0 {
            tex += if e < 5 {
                1
            } else if c.rd8(this.wrapping_add(PROP_KIND).wrapping_add(p as u32)) == 3 {
                2
            } else {
                7
            };
        }
    } else if rots == -9 {
        let a = atan64k(m, c, wx, wy, cu, cv)?;
        let e = ((a as u16).wrapping_sub(heading).wrapping_add(0x800) >> 12) as i32;
        if e > 0 {
            tex += if e > 4 && e < 14 {
                5
            } else if e > 13 {
                e - 8
            } else {
                e
            };
        }
    }
    Ok(tex)
}

/// Project a camera-facing sprite quad and draw it with the prop drawers.
pub fn render_prop(m: &mut Machine, c: &mut Cpu, this: u32, p: i32) -> Result<(), Fault> {
    let tex = prop_frame(m, c, this, p)?;
    let ang = (c.rd16(this.wrapping_add(CAM_ANGLE)).wrapping_add(0x4000) >> 5) as i32;
    let (co, si) = (rdi(c, this, COS_TABLE, ang), rdi(c, this, SIN_TABLE, ang));
    let (hw, hd) = (rdi(c, this, FRAME_HALF_W, tex), rdi(c, this, FRAME_DEPTH, tex));
    let (w_cos, w_sin) = (mul16(hw, co), mul16(hw, si));
    let (d_cos, d_sin) = (mul16(hd, co), mul16(hd, si));
    let (wx, wy) = (rdi(c, this, WALL_X, p), rdi(c, this, WALL_Y, p));
    let q = this.wrapping_add(16 * p as u32);
    let corners = [
        (wx.wrapping_add(w_cos).wrapping_sub(d_cos), wy.wrapping_add(w_sin).wrapping_sub(d_sin)),
        (wx.wrapping_sub(w_cos).wrapping_sub(d_cos), wy.wrapping_sub(w_sin).wrapping_sub(d_sin)),
        (wx.wrapping_sub(w_cos).wrapping_sub(d_cos), wy.wrapping_sub(w_sin).wrapping_sub(d_sin)),
        (wx.wrapping_add(w_cos).wrapping_sub(d_cos), wy.wrapping_add(w_sin).wrapping_sub(d_sin)),
    ];
    for (k, (x, y)) in corners.iter().enumerate() {
        c.wr32(q.wrapping_add(WALL_PX + 4 * k as u32), *x as u32);
        c.wr32(q.wrapping_add(WALL_PY + 4 * k as u32), *y as u32);
    }
    let hwid = focal(c, this);
    let top = rdi(c, this, WALL_BOTTOM, p).wrapping_add(rdi(c, this, OBJ_TOP, p)).wrapping_sub(rd(c, this, CAM_Z));
    let (fb, fh) = (rdi(c, this, FRAME_BASE, tex), rdi(c, this, FRAME_HEIGHT, tex));
    let f1 = flag(c, this, p, 1);
    let h_lo = top.wrapping_sub(fb).wrapping_sub(fh).wrapping_mul(hwid);
    let h_hi = if f1 & 0x10 != 0 { top.wrapping_sub(fh) } else { top.wrapping_add(fb).wrapping_sub(fh) }.wrapping_mul(hwid);
    let cam = c.rd16(this.wrapping_add(CAM_ANGLE));
    let px = |c: &Cpu, k: u32| c.rd32(q.wrapping_add(WALL_PX + 4 * k)) as i32;
    let py = |c: &Cpu, k: u32| c.rd32(q.wrapping_add(WALL_PY + 4 * k)) as i32;
    let ox = |k: u32| this.wrapping_add(WALL_SX + 4 * k);
    let oy = |k: u32| this.wrapping_add(WALL_SY + 4 * k);
    let mut ok = [false; 4];
    for (pair, h) in [(0u32, h_lo), (2, h_hi)] {
        let (i, j) = (pair, pair + 1);
        ok[i as usize] = rwc_plot(c, this, px(c, i), py(c, i), h, ox(i), oy(i), 0x400);
        ok[j as usize] = rwc_plot(c, this, px(c, j), py(c, j), h, ox(j), oy(j), 0x400);
        if ok[j as usize] && !ok[i as usize] {
            ok[i as usize] = new_point(c, this, px(c, i), py(c, i), cam, h, ox(i), oy(i));
        }
        if ok[i as usize] && !ok[j as usize] {
            ok[j as usize] = new_point(c, this, px(c, j), py(c, j), cam, h, ox(j), oy(j));
        }
    }
    if ok.iter().all(|&v| v) {
        let f2 = flag(c, this, p, 2);
        let mode = if f2 & 8 != 0 {
            if f1 & 0x20 != 0 {
                PropMode::Aura(9)
            } else if f2 & 2 != 0 {
                PropMode::Invis
            } else {
                PropMode::Pack
            }
        } else {
            PropMode::Mask
        };
        draw_prop(c, this, p, tex, mode);
    }
    Ok(())
}

pub(super) fn h_render_prop(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, p) = (c.r[EAX], c.r[EDX] as i32);
    render_prop(m, c, this, p)?;
    c.ret_near(0);
    Ok(())
}
