//! Ray casting: grid DDA per screen column (`RayCast`) and per-tile
//! visibility / occlusion bookkeeping (`CheckTile`).
//!
//! The map is 32x32 tiles per level (1024 dwords per level, one byte per
//! tile side).  Visibility is tracked per level with bitmasks (bit = level).

use super::*;
use crate::le::rt;

// ---- ray state (offsets from `this`)
pub(super) const RAY_ANGLE: u32 = 0xAF23C; // u16
pub(super) const RAY_X1: u32 = 0xAF240; // x at next y-crossing
pub(super) const RAY_Y1: u32 = 0xAF244; // y of next y-crossing (high word stepped)
pub(super) const RAY_X2: u32 = 0xAF248; // x of next x-crossing (high word stepped)
pub(super) const RAY_Y2: u32 = 0xAF24C; // y at next x-crossing
pub(super) const RAY_DX: u32 = 0xAF250; // x step per y-crossing
pub(super) const RAY_DY: u32 = 0xAF254; // y step per x-crossing
pub(super) const CAM_FRAC_L: u32 = 0xAF260; // camera fraction to the left tile edge
pub(super) const CAM_FRAC_B: u32 = 0xAF264;
pub(super) const CAM_FRAC_R: u32 = 0xAF268;
pub(super) const CAM_FRAC_T: u32 = 0xAF26C;
pub(super) const RAY_COLUMN: u32 = 0xBA28C;
/// Tile side being checked (0..3) and its bit position (side*8)
pub(super) const SIDE: u32 = 0xBA27C;
pub(super) const SIDE_SHIFT: u32 = 0xBA2A8; // u16
/// Tiles visited on this ray / remaining tile budget
pub(super) const RAY_STEPS: u32 = 0xBA284;
pub(super) const RAY_BUDGET: u32 = 0xBDEB4;
pub(super) const MAX_COORD: i32 = 0x1F_C000;

// ---- level / tile visibility state
pub(super) const LEVEL: u32 = 0xBA280; // current level
pub(super) const LEVEL_BIT: u32 = 0xBA2A7; // u8: 1 << level
pub(super) const LEVEL_SHIFT: u32 = 0xBA2A6; // dword >> 16 = shift for per-level masks
pub(super) const ABOVE_OPEN: u32 = 0xBA2A4; // u8 mask: levels open upwards
pub(super) const BELOW_OPEN: u32 = 0xBA2A5; // u8 mask: levels open downwards
pub(super) const OPEN_MASK: u32 = 0xBA298;
pub(super) const DEPTH_ABOVE: u32 = 0xBA290;
pub(super) const DEPTH_BELOW: u32 = 0xBA294;
pub(super) const BLOCKED: u32 = 0xBDEAC; // levels blocked on this ray
pub(super) const TOP_LEVEL: u32 = 0x5E7A4;
pub(super) const CUR_TILE: u32 = 0xC2078;
pub(super) const MAP_WALLS: u32 = 0xC2074; // ptr to u32 per tile (4 side bytes)
pub(super) const LEVEL_OFFSET: u32 = 0xC2084; // level * 1024
pub(super) const MAP_FLOORS: u32 = 0xC408C; // ptr to u8 per tile
pub(super) const MAP_CEILS: u32 = 0xC3088; // ptr to u8 per tile
pub(super) const SIDE_DONE: u32 = 0xBB2AC; // u8 per tile: sides fully blocked
pub(super) const TILE_SEEN: u32 = 0xBA2AC; // u32 per tile: level mask << shift
pub(super) const TILE_BLOCK: u32 = 0xBCEAC; // u32 per tile
pub(super) const OBJ_DONE: u32 = 0xBB6AC; // u8 per tile
pub(super) const FLOOR_DONE: u32 = 0xBCAAC;
pub(super) const CEIL_DONE: u32 = 0xBC6AC;
pub(super) const TRANS_DONE: u32 = 0xBC2AC;
pub(super) const ABOVE_SEEN: u32 = 0xBBAAC;
pub(super) const BELOW_SEEN: u32 = 0xBBEAC;
/// Per wall/flat id: draw-list index (i16 at +2), flags (byte at +3 of dword)
pub(super) const ID_LIST: u32 = 0x5E5A2;
pub(super) const ID_FLAGS: u32 = 0x5DFA7;

// ---- callees still running as original code
pub(super) const CHECK_BELOW: u32 = 0x05C2FC;
pub(super) const CHECK_ABOVE: u32 = 0x05C028;

fn call(m: &mut Machine, c: &mut Cpu, link: u32, args: &[u32]) -> Result<u32, Fault> {
    m.call_guest(c, rt(link), args)
}

#[inline]
fn rd8(c: &Cpu, this: u32, off: u32) -> u32 {
    c.rd8(this.wrapping_add(off)) as u32
}
#[inline]
fn wr8(c: &mut Cpu, this: u32, off: u32, v: u32) {
    c.wr8(this.wrapping_add(off), v as u8)
}
#[inline]
fn wri(c: &mut Cpu, this: u32, off: u32, i: i32, v: i32) {
    c.wr32(this.wrapping_add(off).wrapping_add((i as u32).wrapping_mul(4)), v as u32)
}
/// Per-level mask shift (`LEVEL_SHIFT` word, arithmetic, masked like `shl cl`).
#[inline]
fn level_shift(c: &Cpu, this: u32) -> u32 {
    ((rd(c, this, LEVEL_SHIFT) >> 16) as u32) & 31
}

pub(super) fn h_ray_cast(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let this = c.r[EAX];
    ray_cast(m, c, this)?;
    c.ret_near(0);
    Ok(())
}

pub(super) fn h_ray_sight(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this,obj,target,range) = (c.r[EAX],c.r[EDX] as i32,c.r[EBX] as i32,c.r[ECX] as i32);
    let (z,tz,angle) = (c.stack_arg(0) as i32,c.stack_arg(1) as i32,c.stack_arg(2) as u16);
    c.r[EAX] = ray_sight(m,c,this,obj,target,range,z,tz,angle)? as u32;
    c.ret_near(12);
    Ok(())
}

fn ray_sight(m: &mut Machine,c: &mut Cpu,this: u32,obj: i32,target: i32,range: i32,z: i32,tz: i32,angle: u16) -> Result<i32,Fault> {
    wr(c,this,0xC2080,tile_of(rdi(c,this,WALL_X,target),rdi(c,this,WALL_Y,target)));
    c.wr32(rt(0x207038),(tz>>16) as u32);
    let (x,y) = (rdi(c,this,WALL_X,obj),rdi(c,this,WALL_Y,obj));
    let (cu,cv) = ((x & !255).wrapping_add(128),(y & !255).wrapping_add(128));
    let (left,bottom) = (cu & 0xFFFF,cv & 0xFFFF);
    for (off,value) in [(CAM_FRAC_L,left),(CAM_FRAC_R,0xFFFF-left),(CAM_FRAC_B,bottom),(CAM_FRAC_T,0xFFFF-bottom)] {wr(c,this,off,value);}
    let dz = tz.wrapping_sub(z);
    let mut previous_z = z;
    let mut result = 0;
    trace_grid(m,c,this,angle,cu,cv,|_m,c,this,rx,ry,side| {
        wr(c,this,CUR_TILE,tile_of(rx,ry));
        let distance = distance(x,y,rx,ry).min(range);
        let vertical = if range == 0 {i32::MAX} else {
            i32::try_from(distance as i64 * dz as i64 / range as i64).map_err(|_|Fault::DivideError)?
        };
        let next_z = z.wrapping_add(vertical);
        wr(c,this,0xBA2A0,next_z >> 16);
        wr(c,this,LEVEL,previous_z >> 16);
        result = super::levels::tile_sight(c,this,side as u32);
        if result != 0 { return Ok(false); }
        previous_z = if dz < 0 {next_z.max(tz)} else {next_z.min(tz)};
        Ok(true)
    })?;
    Ok(result)
}

pub(super) fn h_check_tile(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let this = c.r[EAX];
    c.r[EAX] = check_tile(m, c, this)?;
    c.ret_near(0);
    Ok(())
}

/// Tile index of a 16.16 map position.
#[inline]
fn tile_of(x: i32, y: i32) -> i32 {
    ((((y as u32) >> 16) << 5) | (((x as u32) >> 16) & 31)) as i32
}

/// Visit tile side `side` at (x, y): returns false when the ray must stop.
fn visit(m: &mut Machine, c: &mut Cpu, this: u32, x: i32, y: i32, side: i32) -> Result<bool, Fault> {
    let tile = tile_of(x, y);
    wr(c, this, SIDE, side);
    wr(c, this, CUR_TILE, tile);
    // word store of (SIDE << 3); only the low word of the side value matters
    c.wr16(this.wrapping_add(SIDE_SHIFT), (side << 3) as u16);
    if rd8(c, this, SIDE_DONE + tile as u32) & (1 << side) != 0 {
        return Ok(false);
    }
    check_tile(m, c, this)?;
    wr(c, this, RAY_STEPS, rd(c, this, RAY_STEPS) + 1);
    let budget = rd(c, this, RAY_BUDGET) - 1;
    wr(c, this, RAY_BUDGET, budget);
    Ok(budget != 0)
}

/// Increment the integer (high) word of a 16.16 field with 16-bit wraparound.
#[inline]
fn inc_hi(c: &mut Cpu, this: u32, off: u32) {
    let a = this.wrapping_add(off + 2);
    let v = c.rd16(a).wrapping_add(1);
    c.wr16(a, v);
}

/// Cast the ray for screen column RAY_COLUMN, visiting tiles front to back.
pub fn ray_cast(m: &mut Machine, c: &mut Cpu, this: u32) -> Result<(), Fault> {
    wr(c, this, RAY_BUDGET, 0x12);
    let col = rd(c, this, RAY_COLUMN) as u32;
    let col_angles = rd(c, this, COL_ANGLES) as u32;
    let ang = c.rd16(this.wrapping_add(CAM_ANGLE)).wrapping_add(c.rd16(col_angles.wrapping_add(col * 2)));
    c.wr16(this.wrapping_add(RAY_ANGLE), ang);
    wr(c, this, RAY_STEPS, 0);
    wr(c, this, BLOCKED, 0);
    wr8(c, this, ABOVE_OPEN, 0);
    wr8(c, this, BELOW_OPEN, 0);
    trace_grid(m,c,this,ang,rd(c,this,CAM_U),rd(c,this,CAM_V),visit)
}

/// Rendering and line-of-sight use the same grid crossings, including their
/// asymmetric tie breaks, high-word increments and map-edge saturation.
fn trace_grid(m: &mut Machine, c: &mut Cpu, this: u32, ang: u16, cu: i32, cv: i32,
    mut visit: impl FnMut(&mut Machine,&mut Cpu,u32,i32,i32,i32)->Result<bool,Fault>) -> Result<(),Fault> {
    let q = ((ang as i32) % 0x4000) >> 3;
    let tan_a = rdi(c, this, TAN_TABLE, q);
    let tan_b = rdi(c, this, TAN_TABLE, 0x7FF - q);
    let f = |c: &Cpu, off: u32| rd(c, this, off);
    let quadrant = ang >> 14;
    // per quadrant: (dy per x-crossing, dx per y-crossing) and initial crossings
    let (dy, dx) = if quadrant & 1 == 0 { (tan_a, tan_b) } else { (tan_b, tan_a) };
    wr(c, this, RAY_DY, dy);
    wr(c, this, RAY_DX, dx);
    match quadrant {
        0 => {
            wr(c, this, RAY_Y2, cv.wrapping_add(mul16(f(c, CAM_FRAC_R), dy)));
            wr(c, this, RAY_X1, cu.wrapping_add(mul16(f(c, CAM_FRAC_T), dx)));
            wr(c, this, RAY_Y1, cv.wrapping_add(f(c, CAM_FRAC_T)));
            wr(c, this, RAY_X2, cu.wrapping_add(f(c, CAM_FRAC_R)));
            let mut l1 = f(c, RAY_X1) <= f(c, RAY_X2);
            loop {
                if l1 {
                    if !visit(m, c, this, f(c, RAY_X1), f(c, RAY_Y1), 1)? {
                        return Ok(());
                    }
                    wr(c, this, RAY_X1, f(c, RAY_X1).wrapping_add(f(c, RAY_DX)));
                    inc_hi(c, this, RAY_Y1);
                    if f(c, RAY_X1) > MAX_COORD {
                        wr(c, this, RAY_X1, MAX_COORD);
                    }
                    if f(c, RAY_Y1) > MAX_COORD {
                        wr(c, this, RAY_Y1, MAX_COORD);
                    }
                    l1 = f(c, RAY_X1) <= f(c, RAY_X2);
                } else {
                    if !visit(m, c, this, f(c, RAY_X2), f(c, RAY_Y2), 0)? {
                        return Ok(());
                    }
                    inc_hi(c, this, RAY_X2);
                    wr(c, this, RAY_Y2, f(c, RAY_Y2).wrapping_add(f(c, RAY_DY)));
                    if f(c, RAY_X2) > MAX_COORD {
                        wr(c, this, RAY_X2, MAX_COORD);
                    }
                    if f(c, RAY_Y2) > MAX_COORD {
                        wr(c, this, RAY_Y2, MAX_COORD);
                    }
                    l1 = f(c, RAY_Y2) > f(c, RAY_Y1);
                }
            }
        }
        1 => {
            wr(c, this, RAY_Y2, cv.wrapping_add(mul16(f(c, CAM_FRAC_L), dy)));
            wr(c, this, RAY_X1, cu.wrapping_sub(mul16(f(c, CAM_FRAC_T), dx)));
            wr(c, this, RAY_Y1, cv.wrapping_add(f(c, CAM_FRAC_T)));
            wr(c, this, RAY_X2, cu.wrapping_sub(f(c, CAM_FRAC_L)));
            let mut l1 = f(c, RAY_X1) >= f(c, RAY_X2);
            loop {
                if l1 {
                    if !visit(m, c, this, f(c, RAY_X1), f(c, RAY_Y1), 1)? {
                        return Ok(());
                    }
                    wr(c, this, RAY_X1, f(c, RAY_X1).wrapping_sub(f(c, RAY_DX)));
                    inc_hi(c, this, RAY_Y1);
                    if f(c, RAY_X1) < 0 {
                        wr(c, this, RAY_X1, 0);
                    }
                    if f(c, RAY_Y1) > MAX_COORD {
                        wr(c, this, RAY_Y1, MAX_COORD);
                    }
                    l1 = f(c, RAY_X1) >= f(c, RAY_X2);
                } else {
                    if !visit(m, c, this, f(c, RAY_X2), f(c, RAY_Y2), 2)? {
                        return Ok(());
                    }
                    wr(c, this, RAY_X2, f(c, RAY_X2).wrapping_sub(0x10000));
                    wr(c, this, RAY_Y2, f(c, RAY_Y2).wrapping_add(f(c, RAY_DY)));
                    if f(c, RAY_X2) < 0 {
                        wr(c, this, RAY_X2, 0);
                    }
                    if f(c, RAY_Y2) > MAX_COORD {
                        wr(c, this, RAY_Y2, MAX_COORD);
                    }
                    l1 = f(c, RAY_Y2) > f(c, RAY_Y1);
                }
            }
        }
        2 => {
            wr(c, this, RAY_Y2, cv.wrapping_sub(mul16(f(c, CAM_FRAC_L), dy)));
            wr(c, this, RAY_X1, cu.wrapping_sub(mul16(f(c, CAM_FRAC_B), dx)));
            wr(c, this, RAY_Y1, cv.wrapping_sub(f(c, CAM_FRAC_B)));
            wr(c, this, RAY_X2, cu.wrapping_sub(f(c, CAM_FRAC_L)));
            let mut l1 = f(c, RAY_X1) >= f(c, RAY_X2);
            loop {
                if l1 {
                    if !visit(m, c, this, f(c, RAY_X1), f(c, RAY_Y1), 3)? {
                        return Ok(());
                    }
                    wr(c, this, RAY_X1, f(c, RAY_X1).wrapping_sub(f(c, RAY_DX)));
                    wr(c, this, RAY_Y1, f(c, RAY_Y1).wrapping_sub(0x10000));
                    if f(c, RAY_X1) < 0 {
                        wr(c, this, RAY_X1, 0);
                    }
                    if f(c, RAY_Y1) < 0 {
                        wr(c, this, RAY_Y1, 0);
                    }
                    l1 = f(c, RAY_X1) >= f(c, RAY_X2);
                } else {
                    if !visit(m, c, this, f(c, RAY_X2), f(c, RAY_Y2), 2)? {
                        return Ok(());
                    }
                    wr(c, this, RAY_X2, f(c, RAY_X2).wrapping_sub(0x10000));
                    wr(c, this, RAY_Y2, f(c, RAY_Y2).wrapping_sub(f(c, RAY_DY)));
                    if f(c, RAY_X2) < 0 {
                        wr(c, this, RAY_X2, 0);
                    }
                    if f(c, RAY_Y2) < 0 {
                        wr(c, this, RAY_Y2, 0);
                    }
                    l1 = f(c, RAY_Y2) < f(c, RAY_Y1);
                }
            }
        }
        _ => {
            wr(c, this, RAY_Y2, cv.wrapping_sub(mul16(f(c, CAM_FRAC_R), dy)));
            wr(c, this, RAY_X1, cu.wrapping_add(mul16(f(c, CAM_FRAC_B), dx)));
            wr(c, this, RAY_Y1, cv.wrapping_sub(f(c, CAM_FRAC_B)));
            wr(c, this, RAY_X2, cu.wrapping_add(f(c, CAM_FRAC_R)));
            let mut l1 = f(c, RAY_X1) <= f(c, RAY_X2);
            loop {
                if l1 {
                    if !visit(m, c, this, f(c, RAY_X1), f(c, RAY_Y1), 3)? {
                        return Ok(());
                    }
                    wr(c, this, RAY_X1, f(c, RAY_X1).wrapping_add(f(c, RAY_DX)));
                    wr(c, this, RAY_Y1, f(c, RAY_Y1).wrapping_sub(0x10000));
                    if f(c, RAY_X1) > MAX_COORD {
                        wr(c, this, RAY_X1, MAX_COORD);
                    }
                    if f(c, RAY_Y1) < 0 {
                        wr(c, this, RAY_Y1, 0);
                    }
                    l1 = f(c, RAY_X1) <= f(c, RAY_X2);
                } else {
                    if !visit(m, c, this, f(c, RAY_X2), f(c, RAY_Y2), 0)? {
                        return Ok(());
                    }
                    inc_hi(c, this, RAY_X2);
                    wr(c, this, RAY_Y2, f(c, RAY_Y2).wrapping_sub(f(c, RAY_DY)));
                    if f(c, RAY_X2) > MAX_COORD {
                        wr(c, this, RAY_X2, MAX_COORD);
                    }
                    if f(c, RAY_Y2) < 0 {
                        wr(c, this, RAY_Y2, 0);
                    }
                    l1 = f(c, RAY_Y2) < f(c, RAY_Y1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sight_crosses_all_quadrants_stops_at_walls_and_cleans_stack_arguments() {
        let path = std::path::PathBuf::from("__sight_no_files__");
        let mut m = Machine::new(crate::vfs::Vfs::new(path.clone(),path));
        let mut c = Cpu::new(); let this = 0x100000;
        wr(&mut c,this,MAP_WALLS,0x310000); wr(&mut c,this,MAP_CEILS,0x320000); wr(&mut c,this,MAP_FLOORS,0x330000);
        for i in 0..2048 {wri(&mut c,this,TAN_TABLE,i,0x10000);}
        wri(&mut c,this,WALL_X,1,0x58000); wri(&mut c,this,WALL_Y,1,0x58000);
        c.wr16(this + RAY_ANGLE,0x1234);
        for (angle,x,y) in [(0x2000,0x78000,0x78000),(0x6000,0x38000,0x78000),(0xA000,0x38000,0x38000),(0xE000,0x78000,0x38000)] {
            wri(&mut c,this,WALL_X,2,x); wri(&mut c,this,WALL_Y,2,y);
            let range = distance(0x58000,0x58000,x,y);
            assert_eq!(ray_sight(&mut m,&mut c,this,1,2,range,0x8000,0x8000,angle).unwrap(),1);
            assert_eq!(rd(&c,this,CUR_TILE),tile_of(x,y));
            assert_eq!(rd(&c,this,LEVEL),0);
            assert_eq!(c.rd16(this + RAY_ANGLE),0x1234);
            let first = 0x310000 + tile_of(0x58000,0x58000) as u32*4;
            c.wr32(first,0x01010101);
            assert_eq!(ray_sight(&mut m,&mut c,this,1,2,range,0x8000,0x8000,angle).unwrap(),-1);
            c.wr32(first,0);
            c.r[ESP]=0x30000; c.r[EAX]=this; c.r[EDX]=1; c.r[EBX]=2; c.r[ECX]=range as u32;
            c.push32(angle as u32); c.push32(0x8000); c.push32(0x8000); c.push32(0x1234);
            h_ray_sight(&mut m,&mut c).unwrap();
            assert_eq!((c.r[EAX],c.r[ESP],c.eip),(1,0x30000,0x1234));
        }
    }
}

/// Process the tile side selected by CUR_TILE/SIDE on the current level:
/// queue its wall, floor, ceiling and objects for drawing and update the
/// per-level occlusion state.  Returns the original's final EAX.
pub fn check_tile(m: &mut Machine, c: &mut Cpu, this: u32) -> Result<u32, Fault> {
    let cur = rd(c, this, CUR_TILE);
    let walls = rd(c, this, MAP_WALLS) as u32;
    let side_bit = |c: &Cpu| (1u32 << (rd8(c, this, SIDE) & 31)) & 0xFF;
    if c.rd32(walls.wrapping_add((cur as u32).wrapping_mul(4))) == 0xFFFF_FFFF {
        let b = rd8(c, this, SIDE_DONE + cur as u32) | side_bit(c);
        wr8(c, this, SIDE_DONE + cur as u32, b);
        wr(c, this, RAY_BUDGET, 1);
        return Ok(0);
    }
    let lv = rd8(c, this, LEVEL_BIT);
    let mask = lv << level_shift(c, this);
    let seen = rdi(c, this, TILE_SEEN, cur) as u32;
    let mut eax: u32 = 0;
    let mut wall_id: u32 = 0;
    let mut skip_to_neighbours = false;
    if mask & seen != 0 {
        skip_to_neighbours = true;
    } else if rd(c, this, BLOCKED) as u32 & lv != 0 {
        let v = rdi(c, this, TILE_BLOCK, cur) as u32 | mask;
        wri(c, this, TILE_BLOCK, cur, v as i32);
        skip_to_neighbours = true;
    }
    if !skip_to_neighbours {
        wri(c, this, TILE_SEEN, cur, (seen | mask) as i32);
        // objects standing in this tile
        let cur = rd(c, this, CUR_TILE);
        if rd8(c, this, OBJ_DONE + cur as u32) & rd8(c, this, LEVEL_BIT) == 0 {
            eax = check_objects(m, c, this, rd(c, this, LEVEL))? as u32;
            let cur = rd(c, this, CUR_TILE) as u32;
            let v = rd8(c, this, OBJ_DONE + cur) | rd8(c, this, LEVEL_BIT);
            wr8(c, this, OBJ_DONE + cur, v);
        }
        // wall on this side
        let cell = (rd(c, this, CUR_TILE) + rd(c, this, LEVEL_OFFSET)) as u32;
        let shift = ((rd(c, this, SIDE) as u32) << 3) & 31;
        wall_id = (c.rd32((rd(c, this, MAP_WALLS) as u32).wrapping_add(cell.wrapping_mul(4))) >> shift) & 0xFF;
        let cur = rd(c, this, CUR_TILE);
        if wall_id != 0 {
            if (c.rd32(this.wrapping_add(ID_LIST).wrapping_add(wall_id * 2)) as i32) >> 16 >= 0 {
                eax = create_wall(c, this, wall_id as i32, rd(c, this, LEVEL)) as u32;
            }
            let lvm = rd8(c, this, LEVEL_BIT) << level_shift(c, this);
            let cur = rd(c, this, CUR_TILE);
            if rd8(c, this, ID_FLAGS + wall_id * 4) & 0x40 == 0 {
                let v = rdi(c, this, TILE_BLOCK, cur) as u32 | lvm;
                wri(c, this, TILE_BLOCK, cur, v as i32);
                let b = rd(c, this, BLOCKED) as u32 | rd8(c, this, LEVEL_BIT);
                wr(c, this, BLOCKED, b as i32);
            } else {
                let v = rdi(c, this, TILE_BLOCK, cur) as u32 & !lvm;
                wri(c, this, TILE_BLOCK, cur, v as i32);
                wall_id = 0;
            }
        } else {
            let lvm = rd8(c, this, LEVEL_BIT) << level_shift(c, this);
            let v = rdi(c, this, TILE_BLOCK, cur) as u32 & !lvm;
            wri(c, this, TILE_BLOCK, cur, v as i32);
        }
        // floor
        let floor = c.rd8((rd(c, this, MAP_FLOORS) as u32).wrapping_add(cell)) as u32;
        let mut clear_below = false;
        if floor != 0 {
            let cur = rd(c, this, CUR_TILE) as u32;
            let lv = rd8(c, this, LEVEL_BIT);
            if rd8(c, this, FLOOR_DONE + cur) & lv == 0 {
                wr8(c, this, FLOOR_DONE + cur, rd8(c, this, FLOOR_DONE + cur) | lv);
                if (c.rd32(this.wrapping_add(ID_LIST).wrapping_add(floor * 2)) as i32) >> 16 >= 0 {
                    eax = create_flat(c, this, floor as i32, rd(c, this, LEVEL), false) as u32;
                }
            }
            if rd(c, this, LEVEL) > 0 && rd8(c, this, ID_FLAGS + floor * 4) & 0x10 != 0 {
                clear_below = true;
            }
        } else if rd(c, this, LEVEL) > 0 {
            clear_below = true;
        }
        if clear_below {
            let lv = rd8(c, this, LEVEL_BIT);
            wr8(c, this, BELOW_OPEN, rd8(c, this, BELOW_OPEN) | lv);
            let b = rd(c, this, BLOCKED) as u32 & !(lv >> 1);
            wr(c, this, BLOCKED, b as i32);
            let m3 = (rd8(c, this, LEVEL_BIT) >> 1) << level_shift(c, this);
            let cur = rd(c, this, CUR_TILE);
            let v = rdi(c, this, TILE_BLOCK, cur) as u32 & !m3;
            wri(c, this, TILE_BLOCK, cur, v as i32);
        }
        // ceiling
        let ceil = c.rd8((rd(c, this, MAP_CEILS) as u32).wrapping_add(cell)) as u32;
        let mut clear_above = false;
        if ceil != 0 {
            let cur = rd(c, this, CUR_TILE) as u32;
            let lv = rd8(c, this, LEVEL_BIT);
            if rd8(c, this, CEIL_DONE + cur) & lv == 0 {
                wr8(c, this, CEIL_DONE + cur, rd8(c, this, CEIL_DONE + cur) | lv);
                if (c.rd32(this.wrapping_add(ID_LIST).wrapping_add(ceil * 2)) as i32) >> 16 >= 0 {
                    eax = create_flat(c, this, ceil as i32, rd(c, this, LEVEL), true) as u32;
                }
            }
            if rd(c, this, LEVEL) < rd(c, this, TOP_LEVEL) && rd8(c, this, ID_FLAGS + ceil * 4) & 0x10 != 0 {
                clear_above = true;
            }
        } else if rd(c, this, LEVEL) < rd(c, this, TOP_LEVEL) {
            clear_above = true;
        }
        if clear_above {
            let lv = rd8(c, this, LEVEL_BIT);
            wr8(c, this, ABOVE_OPEN, rd8(c, this, ABOVE_OPEN) | lv);
            let b = rd(c, this, BLOCKED) as u32 & !(lv << 1);
            wr(c, this, BLOCKED, b as i32);
            let m4 = (rd8(c, this, LEVEL_BIT) << 1) << level_shift(c, this);
            let cur = rd(c, this, CUR_TILE);
            let v = rdi(c, this, TILE_BLOCK, cur) as u32 & !m4;
            wri(c, this, TILE_BLOCK, cur, v as i32);
        }
        // translucent / object draw list
        if eax != 0 {
            let cur = rd(c, this, CUR_TILE) as u32;
            let lv = rd8(c, this, LEVEL_BIT);
            let b = rd8(c, this, TRANS_DONE + cur);
            if b & lv == 0 {
                wr8(c, this, TRANS_DONE + cur, b | lv);
                tlist_insert(c, this, cell as i32, rd(c, this, LEVEL));
            }
        }
        if wall_id != 0 && rd8(c, this, BELOW_OPEN) == 0 && rd8(c, this, ABOVE_OPEN) == 0 {
            let cur = rd(c, this, CUR_TILE) as u32;
            let b = rd8(c, this, SIDE_DONE + cur) | side_bit(c);
            wr8(c, this, SIDE_DONE + cur, b);
            wr(c, this, RAY_BUDGET, 1);
            return Ok(eax);
        }
    }

    // ---- look through to the levels below / above
    let lv = rd8(c, this, LEVEL_BIT);
    if rd8(c, this, BELOW_OPEN) & lv != 0 {
        let m5 = (lv >> 1) << level_shift(c, this);
        let cur = rd(c, this, CUR_TILE);
        if rdi(c, this, TILE_SEEN, cur) as u32 & m5 == 0 {
            check_vertical(m, c, this, rd(c, this, LEVEL) - 1, false)?;
        }
    }
    let lv = rd8(c, this, LEVEL_BIT);
    if rd8(c, this, ABOVE_OPEN) & lv != 0 {
        let m6 = (lv + lv) << level_shift(c, this);
        let cur = rd(c, this, CUR_TILE);
        if rdi(c, this, TILE_SEEN, cur) as u32 & m6 == 0 {
            check_vertical(m, c, this, rd(c, this, LEVEL) + 1, true)?;
        }
    }
    let cur = rd(c, this, CUR_TILE);
    let blocked = ((rdi(c, this, TILE_BLOCK, cur) >> level_shift(c, this)) as u32) & 0xFF;
    wr(c, this, BLOCKED, blocked as i32);
    let cur = rd(c, this, CUR_TILE) as u32;
    wr8(c, this, ABOVE_SEEN + cur, rd8(c, this, ABOVE_SEEN + cur) | rd8(c, this, ABOVE_OPEN));
    wr8(c, this, BELOW_SEEN + cur, rd8(c, this, BELOW_SEEN + cur) | rd8(c, this, BELOW_OPEN));
    wr8(c, this, ABOVE_OPEN, rd8(c, this, ABOVE_SEEN + cur));
    wr8(c, this, BELOW_OPEN, rd8(c, this, BELOW_SEEN + cur));
    let lv = rd8(c, this, LEVEL_BIT);
    let blocked = rd(c, this, BLOCKED) as u32;
    if lv & blocked == 0 {
        return Ok(lv);
    }
    let open = (rd8(c, this, BELOW_OPEN) >> 1) | (rd8(c, this, ABOVE_OPEN) * 2) | lv;
    wr(c, this, OPEN_MASK, open as i32);
    if (blocked & open) ^ open == 0 {
        wr(c, this, RAY_BUDGET, 1);
        return Ok(0);
    }
    let steps = rd(c, this, RAY_STEPS);
    let da = rd(c, this, DEPTH_ABOVE);
    let db = rd(c, this, DEPTH_BELOW);
    let budget = if rd8(c, this, ABOVE_OPEN) & lv != 0 {
        if rd8(c, this, BELOW_OPEN) & lv != 0 && db > da {
            db.wrapping_mul(steps).wrapping_add(db).wrapping_add(steps * 4)
        } else {
            da.wrapping_mul(steps).wrapping_add(da).wrapping_add(steps * 4)
        }
    } else if rd8(c, this, BELOW_OPEN) & lv != 0 {
        db.wrapping_mul(steps).wrapping_add(db).wrapping_add(steps * 4)
    } else {
        1
    };
    if budget < rd(c, this, RAY_BUDGET) {
        wr(c, this, RAY_BUDGET, budget);
    }
    Ok(budget as u32)
}
