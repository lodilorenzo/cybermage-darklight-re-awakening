//! `CheckAbove` / `CheckBelow`: continue a ray's tile check on the level above
//! (through open ceilings) or below (through open floors).

use super::*;

pub(super) fn h_tile_sight(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    c.r[EAX] = tile_sight(c,c.r[EAX],c.r[EDX]) as u32;
    c.ret_near(0);
    Ok(())
}

pub(super) fn tile_sight(c: &Cpu, this: u32, side: u32) -> i32 {
    let mut level = rd(c,this,LEVEL);
    let destination = rd(c,this,0xBA2A0);
    let tile = rd(c,this,CUR_TILE);
    let cell = |level: i32| level.wrapping_shl(10).wrapping_add(tile) as u32;
    if level < destination {
        let ceilings = rd(c,this,MAP_CEILS) as u32;
        while level < destination {
            if c.rd8(ceilings.wrapping_add(cell(level))) != 0 { return -1; }
            level = level.wrapping_add(1);
        }
    } else if level > destination {
        // Guest 05D63B decrements the level but not the floor pointer: it
        // repeatedly checks the starting floor, not intervening levels.
        if c.rd8((rd(c,this,MAP_FLOORS) as u32).wrapping_add(cell(level))) != 0 { return -1; }
    }
    if tile == rd(c,this,0xC2080) && destination == c.rd32(crate::le::rt(0x207038)) as i32 { return 1; }
    let walls = rd(c,this,MAP_WALLS) as u32;
    if c.rd32(walls.wrapping_add((tile as u32).wrapping_mul(4))) == u32::MAX { return -1; }
    let wall = (c.rd32(walls.wrapping_add(cell(destination).wrapping_mul(4))) >> (side.wrapping_mul(8)&31)) & 0xFF;
    if wall == 0 { return 0; }
    let flags = c.rd8(this.wrapping_add(ID_FLAGS).wrapping_add(wall*4));
    if flags & 0x40 != 0 && flags & 0x10 == 0 {0} else {-1}
}

pub(super) fn h_check_above(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, level) = (c.r[EAX], c.r[EDX] as i32);
    check_vertical(m, c, this, level, true)?;
    c.ret_near(0);
    Ok(())
}

pub(super) fn h_check_below(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, level) = (c.r[EAX], c.r[EDX] as i32);
    check_vertical(m, c, this, level, false)?;
    c.ret_near(0);
    Ok(())
}

#[inline]
fn b(c: &Cpu, this: u32, off: u32, i: i32) -> u32 {
    c.rd8(this.wrapping_add(off).wrapping_add(i as u32)) as u32
}
#[inline]
fn or_b(c: &mut Cpu, this: u32, off: u32, i: i32, v: u32) {
    let a = this.wrapping_add(off).wrapping_add(i as u32);
    let x = c.rd8(a) | v as u8;
    c.wr8(a, x);
}
#[inline]
fn upd(c: &mut Cpu, this: u32, off: u32, i: i32, f: impl Fn(u32) -> u32) {
    let a = this.wrapping_add(off).wrapping_add((i as u32).wrapping_mul(4));
    let x = f(c.rd32(a));
    c.wr32(a, x);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sight_preserves_vertical_barriers_end_tile_and_transparent_wall_flags() {
        let mut c = Cpu::new(); let this = 0x100000;
        wr(&mut c,this,CUR_TILE,7); wr(&mut c,this,0xC2080,8);
        wr(&mut c,this,LEVEL,0); wr(&mut c,this,0xBA2A0,2);
        wr(&mut c,this,MAP_CEILS,0x310000); wr(&mut c,this,MAP_FLOORS,0x320000); wr(&mut c,this,MAP_WALLS,0x330000);
        assert_eq!(tile_sight(&c,this,0),0);
        c.wr8(0x310000 + 1024 + 7,1); assert_eq!(tile_sight(&c,this,0),-1);
        c.wr8(0x310000 + 1024 + 7,0);
        wr(&mut c,this,LEVEL,2); wr(&mut c,this,0xBA2A0,0);
        c.wr8(0x320000 + 1024 + 7,1); assert_eq!(tile_sight(&c,this,0),0);
        c.wr8(0x320000 + 2048 + 7,1); assert_eq!(tile_sight(&c,this,0),-1);
        c.wr8(0x320000 + 2048 + 7,0);
        c.wr32(0x330000 + 7*4,0x03020100);
        c.wr8(this + ID_FLAGS + 2*4,0x40);
        assert_eq!(tile_sight(&c,this,0),0); assert_eq!(tile_sight(&c,this,1),-1);
        assert_eq!(tile_sight(&c,this,2),0); assert_eq!(tile_sight(&c,this,6),0);
        c.wr8(this + ID_FLAGS + 2*4,0x50); assert_eq!(tile_sight(&c,this,2),-1);
        c.wr32(0x330000 + 7*4,u32::MAX); assert_eq!(tile_sight(&c,this,0),-1);
        wr(&mut c,this,0xC2080,7); c.wr32(crate::le::rt(0x207038),0);
        assert_eq!(tile_sight(&c,this,0),1);
    }
}

/// Check the current tile side on `level` (`up`: CheckAbove, else CheckBelow).
pub fn check_vertical(m: &mut Machine, c: &mut Cpu, this: u32, level: i32, up: bool) -> Result<(), Fault> {
    let lb = 1u32 << ((level as u32) & 31);
    let sh = |c: &Cpu| ((rd(c, this, LEVEL_SHIFT) >> 16) as u32) & 31;
    let cur = rd(c, this, CUR_TILE);
    if rd(c, this, BLOCKED) as u32 & lb != 0 {
        let s = sh(c);
        upd(c, this, TILE_BLOCK, cur, |v| v | (lb << s));
    } else {
        let s = sh(c);
        upd(c, this, TILE_SEEN, cur, |v| v | (lb << s));
        let mut eax: u32 = 0;
        if b(c, this, OBJ_DONE, cur) & lb == 0 {
            eax = check_objects(m, c, this, level)? as u32;
            or_b(c, this, OBJ_DONE, rd(c, this, CUR_TILE), lb);
        }
        let cur = rd(c, this, CUR_TILE);
        let cell = (level << 10) + cur;
        let shift = ((rd(c, this, SIDE) as u32) << 3) & 31;
        let wall = (c.rd32((rd(c, this, MAP_WALLS) as u32).wrapping_add((cell as u32).wrapping_mul(4))) >> shift) & 0xFF;
        if wall != 0 {
            if (c.rd32(this.wrapping_add(ID_LIST).wrapping_add(wall * 2)) as i32) >> 16 >= 0 {
                eax = create_wall(c, this, wall as i32, level) as u32;
            }
            let s = sh(c);
            let cur = rd(c, this, CUR_TILE);
            if b(c, this, ID_FLAGS, 4 * wall as i32) & 0x40 == 0 {
                upd(c, this, TILE_BLOCK, cur, |v| v | (lb << s));
                let bl = rd(c, this, BLOCKED) as u32 | lb;
                wr(c, this, BLOCKED, bl as i32);
            } else {
                upd(c, this, TILE_BLOCK, cur, |v| v & !(lb << s));
            }
        } else {
            let s = sh(c);
            upd(c, this, TILE_BLOCK, cur, |v| v & !(lb << s));
        }
        // ceiling (up) or floor (down) of this level
        let (map, done) = if up { (MAP_CEILS, CEIL_DONE) } else { (MAP_FLOORS, FLOOR_DONE) };
        let id = c.rd8((rd(c, this, map) as u32).wrapping_add(cell as u32)) as u32;
        let can_open = if up { level < rd(c, this, TOP_LEVEL) } else { level > 0 };
        let open = if id != 0 {
            let cur = rd(c, this, CUR_TILE);
            if b(c, this, done, cur) & lb == 0 {
                or_b(c, this, done, cur, lb);
                if (c.rd32(this.wrapping_add(ID_LIST).wrapping_add(id * 2)) as i32) >> 16 >= 0 {
                    eax = create_flat(c, this, id as i32, level, up) as u32;
                }
            }
            can_open && b(c, this, ID_FLAGS, 4 * id as i32) & 0x10 != 0
        } else {
            can_open
        };
        if open {
            let (open_mask, m2) = if up { (ABOVE_OPEN, lb << 1) } else { (BELOW_OPEN, lb >> 1) };
            or_b(c, this, open_mask, 0, lb);
            let bl = rd(c, this, BLOCKED) as u32 & !m2;
            wr(c, this, BLOCKED, bl as i32);
            let s = sh(c);
            let cur = rd(c, this, CUR_TILE);
            upd(c, this, TILE_BLOCK, cur, |v| v & !(m2 << s));
        }
        if eax != 0 {
            let cur = rd(c, this, CUR_TILE);
            if b(c, this, TRANS_DONE, cur) & lb == 0 {
                or_b(c, this, TRANS_DONE, cur, lb);
                tlist_insert(c, this, cell, level);
            }
        }
    }
    // continue to the next level
    let (open_mask, m2) = if up { (ABOVE_OPEN, lb << 1) } else { (BELOW_OPEN, lb >> 1) };
    if b(c, this, open_mask, 0) & lb != 0 {
        let s = sh(c);
        let cur = rd(c, this, CUR_TILE);
        if rdi(c, this, TILE_SEEN, cur) as u32 & (m2 << s) == 0 {
            check_vertical(m, c, this, if up { level + 1 } else { level - 1 }, up)?;
        }
    }
    Ok(())
}
