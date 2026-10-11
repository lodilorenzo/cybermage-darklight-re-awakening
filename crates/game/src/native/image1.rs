//! Native port of the raycast renderer (`Image1` class, image1.cpp).
//!
//! The renderer state lives in the original `Image1` object in guest memory
//! (the base of the global `Xox xox`), so these functions read and write the
//! same fields as the original code.  Field offsets are relative to `this`.

use crate::machine::Machine;
use x86::*;

// ---- Image1 fields (offsets from `this`) -----------------------------------
/// Texture index per flat (read as the high half of a dword at +2*idx)
const FLAT_TEXTURE: u32 = 0x81DDC;
/// Pointers to 128x128 textures
const TEXTURE_PTR: u32 = 0xA4A3C;
/// Light-type per light index (u16)
const LIGHT_TYPE: u32 = 0xA4934;
/// Light parameter per light index (i16 at +2, read as dword >> 16)
const LIGHT_PARAM: u32 = 0xA48F2;
/// Currently selected light-curve index
const CUR_CURVE: u32 = 0xA4974;
const CURVE_BASE: u32 = 0xA497C;
const CURVE_SHIFT: u32 = 0xA49BC;
const CURVE_SCALE: u32 = 0xA49FC;
/// Active light curve: distance bias, shift, scale (see `shade_distance`)
const SHADE_BIAS: u32 = 0xBDECC;
const SHADE_SHIFT: u32 = 0xBDED0;
const SHADE_SCALE: u32 = 0xBDED4;
/// 64 shade colormaps (pointers)
const SHADE_MAPS: u32 = 0xBDED8;
/// Fixed colormaps used by light types 2..5
const FIXED_MAPS: u32 = 0xBDFD8;
/// 17 fog colormaps (pointers)
const FOG_MAPS: u32 = 0xBE000;
/// Flat light index for the current frame (0 = distance shading)
const FLAT_LIGHT: u32 = 0xBDEB0;
/// Fog enabled
const FOG_ON: u32 = 0xBDEB8;

const CAM_U: u32 = 0xE33E8;
const CAM_V: u32 = 0xE33EC;
/// Horizon row
const HORIZON: u32 = 0xE33FC;
/// Polygon description of the flat being drawn (two edge chains, <= 3 segments)
const POLY_X0: u32 = 0xE3450;
const POLY_TOP: u32 = 0xE3454;
const L_X: u32 = 0xE3458;
const L_END: u32 = 0xE3464;
const R_X: u32 = 0xE3470;
const R_END: u32 = 0xE347C;
const L_SLOPE: u32 = 0xE3488;
const R_SLOPE: u32 = 0xE3494;
const L_LAST: u32 = 0xE34A0;
const R_LAST: u32 = 0xE34A4;
/// Framebuffer offset from `this`
const FB_OFFSET: u32 = 0xE34A8;
const VIEW_H: u32 = 0xE34AC;
const VIEW_W: u32 = 0xE34B4;
const HALF_W: u32 = 0xE34B8;
const U_COS: u32 = 0xE34C4;
const U_SIN: u32 = 0xE34C8;
const U_STEP: u32 = 0xE34CC;
const V_COS: u32 = 0xE34D0;
const V_SIN: u32 = 0xE34D4;
const V_STEP: u32 = 0xE34D8;

thread_local! {
    // Zero focal length selects the original, bit-exact 640-wide renderer.
    static NATIVE_TARGET: std::cell::Cell<(u32, i32)> = const { std::cell::Cell::new((640, 0)) };
}
fn render_pitch() -> u32 { NATIVE_TARGET.with(|s| s.get().0) }
fn focal(c: &Cpu, this: u32) -> i32 {
    let f = NATIVE_TARGET.with(|s| s.get().1);
    if f == 0 { rd(c, this, HALF_W) } else { f }
}
/// Wall corner X (4 entries) and Y (4 entries, converted to horizon-relative)
const WALL_SX: u32 = 0xE3400;
const WALL_SY: u32 = 0xE3410;
const CAM_ANGLE: u32 = 0xE33F4;
const CAM_Z: u32 = 0xE33F0;
/// Per-wall data (indexed by wall number)
const WALL_X: u32 = 0x61418;
const WALL_Y: u32 = 0x64080;
const WALL_BOTTOM: u32 = 0x66CE8;
const WALL_HEIGHT: u32 = 0x6C5B8;
const WALL_LENGTH: u32 = 0x69950;
const WALL_FLAGS: u32 = 0x5E7B0; // byte +3 bit 7: mirrored texture
const WALL_ANGLE: u32 = 0x7E65C; // u16
const TEX_W: u32 = 0xA623C;
const TEX_H: u32 = 0xA7A3C;
/// Per-screen-column ray angle offsets (pointer to u16 array)
const COL_ANGLES: u32 = 0xB4278;
/// Fixed-point trig tables (2048 entries)
const TAN_TABLE: u32 = 0xB427C;
const SIN_TABLE: u32 = 0xB627C;
const COS_TABLE: u32 = 0xB827C;
/// Pointer to the 1024x? sky texture (rows of 1024 bytes, drawn from the end)
const SKY_TEXTURE: u32 = 0x5DF98;
/// Colour used above the sky texture
const SKY_FILL: u32 = 0xBE059;

pub fn install(m: &mut Machine, cpu: &mut Cpu) {
    m.hook_sig(cpu, 0x065D34, "Image1::Render", 2, false, h_render);
    m.hook_sig(cpu, 0x0584C0, "Image1::OMapInsert", 2, false, h_omap_insert);
    m.hook_sig(cpu, 0x0581DC, "Image1::OMapDelete", 2, false, h_omap_delete);
    m.hook_sig(cpu, 0x05F94C, "Image1::ShadeDistance", 2, true, h_shade_distance);
    m.hook_sig(cpu, 0x05F984, "Image1::FogDistance", 2, true, h_fog_distance);
    m.hook_sig(cpu, 0x063760, "Image1::DrawFlat", 3, false, h_draw_flat);
    m.hook_sig(cpu, 0x064534, "Image1::DrawTopFlat", 3, false, h_draw_top_flat);
    m.hook_sig(cpu, 0x063D9C, "Image1::DrawMaskFlat", 3, false, h_draw_mask_flat);
    m.hook_sig(cpu, 0x05F45C, "Image1::DrawBackdrop", 4, false, h_draw_backdrop);
    m.hook_sig(cpu, 0x05FA84, "Image1::YSlope", 4, true, h_yslope);
    m.hook_sig(cpu, 0x0596E4, "Image1::FixedRotate", 4, false, h_fixed_rotate);
    m.hook_sig(cpu, 0x05FADC, "Image1::DrawWall", 2, false, h_draw_wall);
    m.hook_sig(cpu, 0x060C70, "Image1::DrawPackWall", 2, false, h_draw_pack_wall);
    m.hook_sig(cpu, 0x0603AC, "Image1::DrawMaskWall", 2, false, h_draw_mask_wall);
    m.hook_sig(cpu, 0x05CBF8, "Image1::RayCast", 1, false, h_ray_cast);
    m.hook_sig(cpu, 0x05C5C8, "Image1::CheckTile", 1, false, h_check_tile);
    m.hook_sig(cpu, 0x0597EC, "Image1::CameraInit", 4, false, h_camera_init);
    m.hook_sig(cpu, 0x059874, "Image1::RenderInit", 2, false, h_render_init);
    m.hook_sig(cpu, 0x05F42C, "Image1::SetViewHorizon", 1, false, h_set_view_horizon);
    m.hook_sig(cpu, 0x059774, "Image1::CVRotate", 4, false, h_cv_rotate);
    m.hook_sig(cpu, 0x0598E4, "Image1::atan64k", 4, true, h_atan64k);
    m.hook_sig(cpu, 0x05F9A0, "Image1::Distance", 4, true, h_distance);
    m.hook_sig(cpu, 0x0650B8, "Image1::XSlope", 4, true, h_xslope);
    m.hook_sig(cpu, 0x05F598, "Image1::RenderBackground", 1, false, h_render_background);
    m.hook_sig(cpu, 0x065B98, "Image1::RenderScene", 1, false, h_render_scene);
    m.hook_sig(cpu, 0x065A60, "Image1::RenderObject", 3, false, h_render_object);
    m.hook_sig(cpu, 0x062C28, "Image1::RWCPlot", 4, true, h_rwc_plot);
    m.hook_sig(cpu, 0x062CB0, "Image1::NewPoint", 4, true, h_new_point);
    m.hook_sig(cpu, 0x064B4C, "Image1::FindPoint", 4, true, h_find_point);
    m.hook_sig(cpu, 0x0650F0, "Image1::ConnectPoints", 2, true, h_connect_points);
    m.hook_sig(cpu, 0x062E0C, "Image1::RenderWall", 2, false, h_render_wall);
    m.hook_sig(cpu, 0x065744, "Image1::RenderFlat", 2, false, h_render_flat);
    m.hook_sig(cpu, 0x0658D4, "Image1::RenderTopFlat", 2, false, h_render_top_flat);
    m.hook_sig(cpu, 0x0630AC, "Image1::RenderProp", 2, false, h_render_prop);
    m.hook_sig(cpu, 0x061960, "Image1::DrawMaskProp", 3, false, h_draw_mask_prop);
    m.hook_sig(cpu, 0x061DC0, "Image1::DrawPackProp", 3, false, h_draw_pack_prop);
    m.hook_sig(cpu, 0x062368, "Image1::DrawPackInvis", 3, false, h_draw_pack_invis);
    m.hook_sig(cpu, 0x0627E8, "Image1::DrawPackAura", 4, false, h_draw_pack_aura);
    m.hook_sig(cpu, 0x05BAB8, "Image1::CreateFlat", 3, true, h_create_flat);
    m.hook_sig(cpu, 0x05BB88, "Image1::CreateTopFlat", 3, true, h_create_top_flat);
    m.hook_sig(cpu, 0x05B4F0, "Image1::OListAdd", 3, false, h_olist_add);
    m.hook_sig(cpu, 0x05AE60, "Image1::OListInsert", 3, false, h_olist_insert);
    m.hook_sig(cpu, 0x05B77C, "Image1::CreateWall", 3, true, h_create_wall);
    m.hook_sig(cpu, 0x05B58C, "Image1::TListInsert", 3, false, h_tlist_insert);
    m.hook_sig(cpu, 0x05BC5C, "Image1::CheckObjects", 2, true, h_check_objects);
    m.hook_sig(cpu, 0x05C028, "Image1::CheckAbove", 2, false, h_check_above);
    m.hook_sig(cpu, 0x05C2FC, "Image1::CheckBelow", 2, false, h_check_below);
    m.hook_sig(cpu, 0x05D5A8, "Image1::TileSight", 2, true, h_tile_sight);
    m.hook_sig(cpu, 0x05D6D4, "Image1::RaySight", 4, true, h_ray_sight);
    m.hook_sig(cpu, 0x05B3A0, "Image1::FXOListInsert", 2, false, h_fxo_list_insert);
    m.hook_sig(cpu, 0x05B250, "Image1::PROPListInsert", 2, false, h_prop_list_insert);
    m.hook_sig(cpu, 0x05B068, "Image1::OPROPListInsert", 2, false, h_oprop_list_insert);
}

/// Whole-frame orchestration. The display's pause-menu wrapper copies this
/// entry trap into its trampoline; verification uses the real guest entry.
pub fn h_render(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this,page) = (c.r[EAX],c.r[EDX]);
    let call = |m: &mut Machine,c: &mut Cpu,link,args: &[u32]| m.call_guest(c,crate::le::rt(link),args);
    let paint = |m: &mut Machine,c: &mut Cpu,part| {
        let vtable = c.rd32(this.wrapping_add(0xE34E4));
        m.call_guest(c,c.rd32(vtable.wrapping_add(4)),&[this,part])
    };
    let widgets = [(0x4B138,1,0x059404),(0x4B144,2,0x059470),(0x4B140,4,0x0594EC),(0x4B148,5,0x059530),(0x4B14C,6,0x05958C)];
    if rd(c,this,0xBDEC4) != 0 {
        paint(m,c,0)?;
        wr(c,this,0xBDEC8,1);
        wr(c,this,0xBDEC4,rd(c,this,0xBDEC4).wrapping_sub(1));
    } else {
        wr(c,this,0xBDEC8,rd(c,this,0xBDEC8)|rd(c,this,0x4B390));
        if rd(c,this,0x4B390) == 0 {
            for &(flag,part,_) in &widgets {if rd(c,this,flag) != 0 {paint(m,c,part)?;}}
        }
    }
    wr(c,this,0x4B008,rd(c,this,0x4B000));
    wr(c,this,0x4B00C,0);
    static ZERO: [u8;0x4000] = [0;0x4000];
    static EMPTY: [u8;0x4000] = [0xFF;0x4000];
    for view in 0..2 {
        let setup = this.wrapping_add(view*36);
        if rd(c,setup,0x4B150) == 0 {continue;}
        let obj = rd(c,setup,0x4B16C);
        let bottom = rdi(c,this,WALL_BOTTOM,obj);
        let z = rd(c,setup,0x4B170).wrapping_add(rdi(c,this,0x78D8C,obj)).wrapping_add(bottom).wrapping_add(bottom/2);
        let angle = c.rd16(this.wrapping_add(WALL_ANGLE).wrapping_add((obj as u32).wrapping_mul(2)));
        call(m,c,0x0597EC,&[this,rdi(c,this,WALL_X,obj) as u32,rdi(c,this,WALL_Y,obj) as u32,z as u32,angle as u32])?;
        call(m,c,0x059874,&[this,view])?;
        set_view_horizon(c,this);
        if rd(c,this,0x5DFA0) != 0 {call(m,c,0x05F598,&[this])?;} else {
            let half = rd(c,this,0xE34B0);
            let radius = (half>>2).wrapping_mul((rd(c,this,CAM_Z)>>16).wrapping_add(1)).min(half);
            let first = rd(c,this,HORIZON).wrapping_sub(radius).max(0);
            let last = rd(c,this,HORIZON).wrapping_add(radius).min(rd(c,this,VIEW_H).wrapping_sub(1));
            crate::native::misc::memset2(c,this.wrapping_add(rd(c,this,FB_OFFSET) as u32).wrapping_add((first as u32).wrapping_mul(640)),0,
                rd(c,this,VIEW_W) as u32,last.wrapping_sub(first).wrapping_add(1) as u32,640);
        }
        c.write_bytes(this.wrapping_add(0x5BF00),&EMPTY[..0x98]);
        c.write_bytes(this.wrapping_add(0x4B3A4),&EMPTY);
        wr(c,this,0x5BEFC,-1); wr(c,this,0x5BEF8,0); wr(c,this,NODE_COUNT,0);
        for (off,len) in [(TRANS_DONE,0x400),(TILE_SEEN,0x1000),(SIDE_DONE,0x400),(TILE_BLOCK,0x1000),
            (ABOVE_SEEN,0x400),(BELOW_SEEN,0x400),(FLOOR_DONE,0x400),(CEIL_DONE,0x400),(OBJ_DONE,0x400)] {
            c.write_bytes(this.wrapping_add(off),&ZERO[..len]);
        }
        let level = rd(c,this,CAM_Z)>>16;
        wr(c,this,LEVEL,level);
        c.wr8(this.wrapping_add(LEVEL_BIT),(1u32.wrapping_shl(level as u32)) as u8);
        let quarter = (rd(c,this,CAM_Z)&0xFFFF)>>14;
        wr(c,this,DEPTH_ABOVE,2<<quarter); wr(c,this,DEPTH_BELOW,2<<(3-quarter));
        wr(c,this,LEVEL_OFFSET,level.wrapping_shl(10)); wr(c,this,NEXT_OBJ,0x94A);
        call(m,c,0x0581DC,&[this,obj as u32])?;
        for column in (0..rd(c,this,HALF_W)).rev() {
            wr(c,this,RAY_COLUMN,column); wr(c,this,0xBA288,column.wrapping_mul(2));
            ray_cast(m,c,this)?;
            wr(c,this,RAY_COLUMN,rd(c,this,VIEW_W).wrapping_sub(column).wrapping_sub(1));
            wr(c,this,0xBA288,column.wrapping_mul(2).wrapping_add(1));
            ray_cast(m,c,this)?;
        }
        call(m,c,0x065B98,&[this])?;
        call(m,c,0x0584C0,&[this,obj as u32,view])?;
        wr(c,this,0x4B008,0);
        // RenderBellsAndWhistles(this, fb, view): EBX still holds the view from
        // the OMapInsert setup (0x0660B3); the HUD is skipped unless it matches.
        let vtable = c.rd32(this.wrapping_add(0xE34E4));
        m.call_guest(c,c.rd32(vtable.wrapping_add(8)),&[this,this,view])?;
        if rd(c,this,0xBDEC8) == 0 && rd(c,this,0x4B394) == 0 {call(m,c,0x0593D0,&[this,view,page])?;}
    }
    if rd(c,this,0x4B394) == 0 {
        if rd(c,this,0xBDEC8) != 0 {call(m,c,0x05934C,&[this])?;} else {
            for &(flag,_,blit) in &widgets {if rd(c,this,flag) != 0 {call(m,c,blit,&[this,page])?;}}
        }
    }
    c.ret_near(0);
    Ok(())

}

#[inline]
fn rd(c: &Cpu, this: u32, off: u32) -> i32 {
    c.rd32(this.wrapping_add(off)) as i32
}
#[inline]
fn rdi(c: &Cpu, this: u32, off: u32, i: i32) -> i32 {
    c.rd32(this.wrapping_add(off).wrapping_add((i as u32).wrapping_mul(4))) as i32
}
#[inline]
fn wr(c: &mut Cpu, this: u32, off: u32, v: i32) {
    c.wr32(this.wrapping_add(off), v as u32)
}

/// Colormap index (0..63) for a distance, using the active light curve.
pub fn shade_distance(c: &Cpu, this: u32, d: i32) -> i32 {
    let shift = (rd(c, this, SHADE_SHIFT) as u32) & 31;
    let v = ((d >> shift).wrapping_sub(rd(c, this, SHADE_BIAS))).wrapping_mul(rd(c, this, SHADE_SCALE));
    v.clamp(0, 63)
}

/// Fog colormap index (0..16) for a distance.
pub fn fog_distance(d: i32) -> i32 {
    ((d >> 14) + 2).clamp(0, 16)
}

fn h_shade_distance(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let r = shade_distance(c, c.r[EAX], c.r[EDX] as i32);
    c.r[EAX] = r as u32;
    c.ret_near(0);
    Ok(())
}

fn h_fog_distance(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    c.r[EAX] = fog_distance(c.r[EDX] as i32) as u32;
    c.ret_near(0);
    Ok(())
}

/// Resolve the frame's light index into a fixed colormap (light types 0..5) or
/// a light curve (type 6, which then behaves like distance shading).
/// Clears `light`/`fog` when per-pixel distance shading must be used.
fn setup_light(c: &mut Cpu, this: u32, light: &mut i32, fog: &mut i32) -> u32 {
    let mut colormap = 0;
    if *light != 0 {
        let l = *light as u32;
        let ty = c.rd16(this.wrapping_add(LIGHT_TYPE).wrapping_add(l.wrapping_mul(2)));
        let param = (c.rd32(this.wrapping_add(LIGHT_PARAM).wrapping_add(l.wrapping_mul(2))) as i32) >> 16;
        match ty {
            0 => colormap = rdi(c, this, SHADE_MAPS, param) as u32,
            1 => colormap = rdi(c, this, FOG_MAPS, param) as u32,
            2..=5 => colormap = rdi(c, this, FIXED_MAPS, ty as i32 - 2) as u32,
            6 => {
                if *light != rd(c, this, CUR_CURVE) {
                    select_curve(c, this, *light);
                }
                *light = 0;
                *fog = 0;
            }
            _ => {} // original leaves the colormap uninitialised here
        }
    } else if rd(c, this, CUR_CURVE) != 0 {
        select_curve(c, this, 0);
    }
    colormap
}

/// Colormap for a distance (fog or light-curve shading).
fn distance_colormap(c: &Cpu, this: u32, fog: i32, dist: i32) -> u32 {
    if fog != 0 {
        rdi(c, this, FOG_MAPS, fog_distance(dist)) as u32
    } else {
        rdi(c, this, SHADE_MAPS, shade_distance(c, this, dist)) as u32
    }
}

/// Select light curve `k` (copies its parameters into the active slots).
fn select_curve(c: &mut Cpu, this: u32, k: i32) {
    wr(c, this, CUR_CURVE, k);
    let b = rdi(c, this, CURVE_BASE, k);
    let s = rdi(c, this, CURVE_SHIFT, k);
    let sc = rdi(c, this, CURVE_SCALE, k);
    wr(c, this, SHADE_BIAS, b);
    wr(c, this, SHADE_SHIFT, s);
    wr(c, this, SHADE_SCALE, sc);
}

fn h_draw_flat(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, idx, height) = (c.r[EAX], c.r[EDX], c.r[EBX] as i32);
    draw_flat(c, this, idx, height, false);
    c.ret_near(0);
    Ok(())
}

fn h_draw_mask_flat(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, idx, height) = (c.r[EAX], c.r[EDX], c.r[EBX] as i32);
    draw_flat_ex(c, this, idx, height, false, true);
    c.ret_near(0);
    Ok(())
}

fn h_draw_top_flat(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, idx, height) = (c.r[EAX], c.r[EDX], c.r[EBX] as i32);
    draw_flat(c, this, idx, height, true);
    c.ret_near(0);
    Ok(())
}

/// Draw a textured floor (`DrawFlat`) or ceiling (`DrawTopFlat`, rows mirrored
/// vertically) polygon with perspective spans.  `height` is the eye-to-plane
/// height numerator; rows are shaded by distance.
pub fn draw_flat(c: &mut Cpu, this: u32, idx: u32, height: i32, top_flat: bool) {
    draw_flat_ex(c, this, idx, height, top_flat, false)
}

/// `masked` (`DrawMaskFlat`): colour 0 is transparent and the texture is
/// rotated by the flat's quadrant (WALL_ANGLE >> 14).
pub fn draw_flat_ex(c: &mut Cpu, this: u32, idx: u32, height: i32, top_flat: bool, masked: bool) {
    let horizon = rd(c, this, HORIZON);
    let l_last = rd(c, this, L_LAST);
    let r_last = rd(c, this, R_LAST);
    // (the original compares against the second segment end, L_END[1]/R_END[1])
    if l_last != 0 && horizon + 1 > rdi(c, this, L_END, 1) {
        return;
    }
    if r_last != 0 && horizon + 1 > rdi(c, this, R_END, 1) {
        return;
    }
    let mut fog = rd(c, this, FOG_ON);
    let t = (c.rd32(this.wrapping_add(FLAT_TEXTURE).wrapping_add(idx.wrapping_mul(2))) as i32) >> 16;
    let tex = rdi(c, this, TEXTURE_PTR, t) as u32;
    let rot = (c.rd16(this.wrapping_add(WALL_ANGLE).wrapping_add(idx.wrapping_mul(2))) >> 14) as u32;

    // lowest row reached by either edge chain
    let mut li = 0;
    for i in 1..=l_last {
        if rdi(c, this, L_END, i) > rdi(c, this, L_END, li) {
            li = i;
        }
    }
    let mut ri = 0;
    let mut i = 1;
    while i <= rd(c, this, R_LAST) {
        if rdi(c, this, R_END, i) > rdi(c, this, R_END, ri) {
            ri = i;
        }
        i += 1;
    }
    let mut bottom = rdi(c, this, R_END, ri).max(rdi(c, this, L_END, li));
    let top = rd(c, this, POLY_TOP);
    if horizon + 1 > bottom {
        return;
    }
    let max_y = rd(c, this, VIEW_H) - if top_flat { 0 } else { 1 };
    if top > max_y {
        return;
    }
    if bottom > max_y {
        bottom = max_y;
    }

    let mut seg_l = 0i32;
    let mut lx = if rd(c, this, L_SLOPE) != 0 { rd(c, this, POLY_X0) } else { rd(c, this, L_X) } << 16;
    let mut seg_r = 0i32;
    let mut rx = if rd(c, this, R_SLOPE) != 0 { rd(c, this, POLY_X0) } else { rd(c, this, R_X) } << 16;

    // lighting: 0 = per-row distance shading, otherwise a light type
    let mut light = rd(c, this, FLAT_LIGHT);
    let mut colormap = setup_light(c, this, &mut light, &mut fog);

    let fb = this.wrapping_add(rd(c, this, FB_OFFSET) as u32);
    let first_row = if top_flat { rd(c, this, VIEW_H) - top } else { top };
    let pitch = render_pitch();
    let mut row = fb.wrapping_add((first_row as u32).wrapping_mul(pitch));
    let row_step = if top_flat { pitch.wrapping_neg() } else { pitch };
    let view_w = rd(c, this, VIEW_W);
    let mut y = top;
    while y <= bottom {
        let mut xs = lx >> 16;
        if xs < 0 {
            xs = 0;
        }
        let mut xe = rx >> 16;
        if xe > view_w - 1 {
            xe = view_w - 1;
        }
        if seg_l != 0 && xs > view_w - 1 {
            return;
        }
        if seg_r != 0 && xe < 0 {
            return;
        }
        let dy = y - rd(c, this, HORIZON);
        if dy > 0 {
            let dist = height.wrapping_div(dy);
            let half_w = focal(c, this) as u32;
            let q = (dist as u32 / half_w) as i32;
            let span_x = xs - rd(c, this, HALF_W) + half_w as i32;
            let du = q.wrapping_mul(rd(c, this, U_STEP));
            let dv = q.wrapping_mul(rd(c, this, V_STEP));
            let mul16 = |a: i32, b: i32| ((a as i64 * b as i64) >> 16) as i32;
            let u = rd(c, this, CAM_U)
                .wrapping_add(mul16(dist, rd(c, this, U_COS)))
                .wrapping_shl(16)
                .wrapping_add(dist.wrapping_mul(rd(c, this, U_SIN)))
                .wrapping_add(du.wrapping_mul(span_x));
            let v = rd(c, this, CAM_V)
                .wrapping_add(mul16(dist, rd(c, this, V_COS)))
                .wrapping_shl(16)
                .wrapping_add(dist.wrapping_mul(rd(c, this, V_SIN)))
                .wrapping_add(dv.wrapping_mul(span_x));
            if light == 0 {
                colormap = distance_colormap(c, this, fog, dist);
            }
            flat_span(c, row.wrapping_add(xs as u32), (xe - xs + 1).max(0) as usize,
                tex, colormap, u, v, du, dv, masked, if masked { rot } else { 0 });
        }
        row = row.wrapping_add(row_step);
        if y == rdi(c, this, L_END, seg_l) {
            seg_l += 1;
        }
        lx = lx.wrapping_add(rdi(c, this, L_SLOPE, seg_l));
        if y == rdi(c, this, R_END, seg_r) {
            seg_r += 1;
        }
        rx = rx.wrapping_add(rdi(c, this, R_SLOPE, seg_r));
        y += 1;
    }
}

#[inline]
fn flat_texel(u: u32, v: u32, rot: u32) -> u32 {
    match rot {
        0 => ((u >> 25) & 0x7F) + ((v >> 18) & 0x3F80),
        1 => (0x7F - ((u >> 25) & 0x7F)) + ((v >> 18) & 0x3F80),
        2 => ((v >> 25) & 0x7F) + ((u >> 18) & 0x3F80),
        _ => ((v >> 25) & 0x7F) + 0x3F80 - ((u >> 18) & 0x3F80),
    }
}

fn flat_span(c: &mut Cpu, dst: u32, count: usize, tex: u32, cmap: u32,
    mut u: i32, mut v: i32, du: i32, dv: i32, masked: bool, rot: u32) {
    if NATIVE_TARGET.with(|s| s.get().1) != 0 && !c.wlog_on {
        // The host-owned framebuffer is not guest code. Avoid self-modifying-code
        // invalidation and verification bookkeeping for every presentation pixel.
        // Fixed-size memory and masked reads preserve the CPU's wrap semantics.
        let mem: &mut [u8; MEM_SIZE + 16] = c.mem.as_mut_slice().try_into().unwrap();
        let mask = (MEM_SIZE - 1) as u32;
        if masked {
            // Normalize once: !(u + du) == !u - du, including wrap.
            let (mut u,mut v,du,dv)=match rot {
                0=>(u,v,du,dv), 1=>(!u,v,du.wrapping_neg(),dv),
                2=>(v,u,dv,du), _=>(v,!u,dv,du.wrapping_neg()),
            };
            for i in 0..count {
                let t=mem[(tex.wrapping_add(flat_texel(u as u32,v as u32,0)) & mask) as usize];
                if t!=0 {mem[(dst.wrapping_add(i as u32) & mask) as usize]=mem[(cmap.wrapping_add(t as u32) & mask) as usize];}
                u=u.wrapping_add(du); v=v.wrapping_add(dv);
            }
        } else {
            for i in 0..count {
                let t=mem[(tex.wrapping_add(flat_texel(u as u32,v as u32,rot)) & mask) as usize];
                mem[(dst.wrapping_add(i as u32) & mask) as usize]=mem[(cmap.wrapping_add(t as u32) & mask) as usize];
                u=u.wrapping_add(du); v=v.wrapping_add(dv);
            }
        }
    } else {
        for i in 0..count {
            let t = c.rd8(tex.wrapping_add(flat_texel(u as u32,v as u32,rot)));
            if !masked || t != 0 { c.wr8(dst.wrapping_add(i as u32),c.rd8(cmap.wrapping_add(t as u32))); }
            u = u.wrapping_add(du); v = v.wrapping_add(dv);
        }
    }
}

#[cfg(test)]
mod span_tests {
    use super::*;
    #[test]
    fn frame_keeps_widget_redraw_full_repaint_and_skip_blit_order() {
        let path = std::path::PathBuf::from("__frame_no_files__");
        let mut m = Machine::new(crate::vfs::Vfs::new(path.clone(),path));
        let mut c = Cpu::new(); let this = 0x100000;
        c.r[ESP]=0x30000; c.r[EBX]=0xAA55; c.r[ECX]=0xBB66;
        c.write_bytes(crate::machine::STUB_BASE+0x800,&[0x0F,0xFF,0,1]);
        c.write_bytes(0x350000,&[0x89,0x15,0,0,0x36,0,0xFF,0x05,4,0,0x36,0,0xC3]);
        c.wr32(this+0xE34E4,0x340000); c.wr32(0x340004,0x350000);
        for link in [0x059404,0x059470] {c.write_bytes(crate::le::rt(link),&[0xFF,0x05,8,0,0x36,0,0xC3]);}
        c.write_bytes(crate::le::rt(0x05934C),&[0xFF,0x05,12,0,0x36,0,0xC3]);
        wr(&mut c,this,0x4B138,1); wr(&mut c,this,0x4B144,1); wr(&mut c,this,0x4B000,7);
        let check = |m: &mut Machine,c: &mut Cpu| {
            c.r[EAX]=this; c.r[EDX]=1; c.push32(0x1234);
            h_render(m,c).unwrap(); assert_eq!((c.r[ESP],c.eip,c.r[EBX],c.r[ECX]),(0x30000,0x1234,0xAA55,0xBB66));
        };
        check(&mut m,&mut c);
        assert_eq!((c.rd32(0x360000),c.rd32(0x360004),c.rd32(0x360008),c.rd32(0x36000C)),(2,2,2,0));
        assert_eq!((rd(&c,this,0x4B008),rd(&c,this,0x4B00C)),(7,0));
        wr(&mut c,this,0xBDEC4,2); check(&mut m,&mut c);
        assert_eq!((rd(&c,this,0xBDEC4),rd(&c,this,0xBDEC8)),(1,1));
        assert_eq!((c.rd32(0x360000),c.rd32(0x360004),c.rd32(0x360008),c.rd32(0x36000C)),(0,3,2,1));
        wr(&mut c,this,0x4B394,1); check(&mut m,&mut c);
        assert_eq!((c.rd32(0x360004),c.rd32(0x36000C)),(4,1));
        wr(&mut c,this,0x4B390,1); check(&mut m,&mut c);
        assert_eq!(c.rd32(0x360004),4);
    }
    #[test]
    fn frame_passes_view_to_bells_and_whistles() {
        // Xox::RenderBellsAndWhistles draws the HUD (weapon/power selectors, hand
        // screen) only when EBX equals the primary view.
        let path = std::path::PathBuf::from("__frame_no_files__");
        let mut m = Machine::new(crate::vfs::Vfs::new(path.clone(),path));
        let mut c = Cpu::new(); let this = 0x100000;
        c.r[ESP]=0x30000; c.r[EBX]=0xAA55;
        c.write_bytes(crate::machine::STUB_BASE+0x800,&[0x0F,0xFF,0,1]);
        for link in [0x0597EC,0x059874,0x05F598,0x0581DC,0x065B98,0x0584C0,0x0593D0] {c.wr8(crate::le::rt(link),0xC3);}
        c.write_bytes(0x350000,&[0x89,0x1D,0,0,0x36,0,0xC3]); // mov [0x360000],ebx; ret
        c.wr32(this+0xE34E4,0x340000); c.wr32(0x340008,0x350000);
        wr(&mut c,this,0x5DFA0,1); wr(&mut c,this,36+0x4B150,1);
        c.r[EAX]=this; c.r[EDX]=0; c.push32(0x1234);
        h_render(&mut m,&mut c).unwrap();
        assert_eq!((c.eip,c.r[EBX],c.rd32(0x360000)),(0x1234,0xAA55,1));
    }
    #[test]
    fn native_spans_match_guest_writes_and_keep_verification_logging() {
        let mut c = Cpu::new();
        for i in 0..16384 { c.wr8(0x100000+i,(i.wrapping_mul(17) >> 2) as u8); }
        for i in 0..256 { c.wr8(0x200000+i,(i as u8).wrapping_mul(3)); }
        for rot in 0..4 { for masked in [false,true] { for (u,v,du,dv) in [(i32::MAX,-91234,317923,-91289),(0,i32::MIN,i32::MIN,i32::MAX)] {
            NATIVE_TARGET.with(|s| s.set((640,0)));
            c.mem[0x300000..0x300400].fill(123);
            flat_span(&mut c,0x300000,1024,0x100000,0x200000,u,v,du,dv,masked,rot);
            let expected = c.read_bytes(0x300000,1024).to_vec();
            c.mem[0x300000..0x300400].fill(123);
            NATIVE_TARGET.with(|s| s.set((3840,1440)));
            flat_span(&mut c,0x300000,1024,0x100000,0x200000,u,v,du,dv,masked,rot);
            assert_eq!(c.read_bytes(0x300000,1024),expected);
        } } }
        for masked in [false,true] {
            NATIVE_TARGET.with(|s| s.set((640,0)));
            c.mem[0x300000..0x301400].fill(123);
            scale_column_m(&mut c,0x200000,8,0x18000,0,0x100000,0x300000+7*640,masked);
            let expected = c.read_bytes(0x300000,8*640).to_vec();
            c.mem[0x300000..0x301400].fill(123);
            NATIVE_TARGET.with(|s| s.set((640,720)));
            scale_column_m(&mut c,0x200000,8,0x18000,0,0x100000,0x300000+7*640,masked);
            assert_eq!(c.read_bytes(0x300000,8*640),expected);
        }
        c.wlog_on = true;
        flat_span(&mut c,0x300000,4,0x100000,0x200000,0,0,0,0,false,0);
        assert_eq!(c.wlog.len(),4);
        NATIVE_TARGET.with(|s| s.set((640,0)));
    }
}

fn h_draw_backdrop(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, angle, dst_off, rows) = (c.r[EAX], c.r[EDX] as u16, c.r[EBX], c.r[ECX] as i32);
    draw_backdrop(c, this, angle, dst_off, rows);
    c.ret_near(0);
    Ok(())
}

/// Draw the sky into the top `rows` rows of the view (rows drawn bottom-up,
/// one sky texture row per screen row), filling any remaining rows above with
/// the sky colour.  `angle` scrolls the sky horizontally.
pub fn draw_backdrop(c: &mut Cpu, this: u32, angle: u16, dst_off: u32, rows: i32) {
    let w = rd(c, this, VIEW_W);
    let u0 = (angle as u32).wrapping_neg() << 16;
    let native = NATIVE_TARGET.with(|s| s.get().1) != 0;
    let f = focal(c, this);
    let sky_w = if native { f * 2 } else { w };
    let mut n = (sky_w << 7) / 640 * 2;
    let step = 0x5000_0000u32 / sky_w as u32;
    let u0 = if native { u0.wrapping_sub(step.wrapping_mul((w / 2 - f) as u32)) } else { u0 };
    let pitch = render_pitch();
    if n > rows {
        n = rows;
    }
    if n > 0 {
        let sky = (rd(c, this, SKY_TEXTURE) as u32).wrapping_add(0x1FC00);
        // destination is the last pixel of the bottom sky row
        let mut end = this
            .wrapping_add(dst_off)
            .wrapping_add(((rows - 1) as u32).wrapping_mul(pitch))
            .wrapping_add((w - 1) as u32);
        let mut v = 0u32;
        for _ in 0..n {
            let src = sky.wrapping_sub((v >> 22) << 10);
            // `_scale` row scaler: count = w, 10-bit texture coordinate
            let mut u = u0;
            let start = end.wrapping_sub(w as u32 - 1);
            for i in 0..w as u32 {
                let b = c.rd8(src.wrapping_add(u >> 22));
                c.wr8(start.wrapping_add(i), b);
                u = u.wrapping_add(step);
            }
            v = v.wrapping_add(step);
            end = end.wrapping_sub(pitch);
        }
    }
    if n < rows {
        let fill = c.rd8(this.wrapping_add(SKY_FILL));
        let fb = this.wrapping_add(rd(c, this, FB_OFFSET) as u32);
        crate::native::misc::memset2(c, fb, fill, w as u32, (rows - n) as u32, pitch);
    }
}

mod levels;
mod lists;
mod objects;
mod project;
mod props;
mod raycast;
mod wall;
pub use levels::*;
pub use lists::*;
pub use objects::*;
pub use project::*;
pub use props::*;
pub use raycast::*;
pub use wall::*;
