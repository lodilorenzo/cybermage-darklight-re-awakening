//! `CheckObjects`: gather flats, FX objects and props standing in the current
//! tile into the cell's depth-ordered draw list.

use super::*;

const TILE_SEEN_BITS: u32 = 0xBE070; // pointer: 32 tiles per dword, 32 dwords per level
const FLAT_CHAIN: u32 = 0xE33E4; // pointer: per tile (u16 next, u16 obj)
const SPRITE_CHAIN: u32 = 0xD7240;
const OBJ_SEEN: u32 = 0xA32C0; // u8 per object
const OBJ_COUNT_: u32 = 0x76124; // u16 per object (cells remaining)
const PROP_LEVEL_: u32 = 0x77758; // u16 per object: (ray step << 8) | level distance
const OBJ_TOP_: u32 = 0x78D8C;
// temporary sub-lists (see lists.rs)
const FXL_COUNT: u32 = 0x5BBEC;
const FXL_HEAD: u32 = 0x5BBF0;
const FXL_TAIL: u32 = 0x5BBF4;
const FXL_PREV: u32 = 0x5BBF8;
const FXL_OBJ: u32 = 0x5BDF8;
const PRL_COUNT: u32 = 0x5B8E0;
const PRL_HEAD: u32 = 0x5B8E4;
const PRL_TAIL: u32 = 0x5B8E8;
const PRL_PREV: u32 = 0x5B8EC;
const PRL_OBJ: u32 = 0x5BAEC;

pub(super) fn h_check_objects(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, level) = (c.r[EAX], c.r[EDX] as i32);
    c.r[EAX] = check_objects(m, c, this, level)? as u32;
    c.ret_near(0);
    Ok(())
}

/// Returns true if anything was found.
pub fn check_objects(m: &mut Machine, c: &mut Cpu, this: u32, level: i32) -> Result<bool, Fault> {
    let zlo = level << 16;
    let zhi = zlo + 0x10000;
    let tile = rd(c, this, CUR_TILE);
    let bits = (rd(c, this, TILE_SEEN_BITS) as u32).wrapping_add((((tile >> 5) + (level << 5)) as u32).wrapping_mul(4));
    let v = c.rd32(bits) | (1u32 << ((tile % 32) as u32 & 31));
    c.wr32(bits, v);
    wr(c, this, PRL_COUNT, 0);
    wr(c, this, PRL_HEAD, -1);
    wr(c, this, PRL_TAIL, -1);
    wr(c, this, FXL_COUNT, 0);
    wr(c, this, FXL_HEAD, -1);
    wr(c, this, FXL_TAIL, -1);
    let mut found = false;
    wr(c, this, INSERT_CELL, tile + (level << 10));
    let link = |c: &Cpu, chain: u32, n: i32, off: u32| {
        let tbl = rd(c, this, chain) as u32;
        c.rd16(tbl.wrapping_add((n as u32).wrapping_mul(4)).wrapping_add(off)) as i32
    };
    let objb = |c: &mut Cpu, base: u32, o: i32, v: u8| c.wr8(this.wrapping_add(base).wrapping_add(o as u32), v);

    // flats in this tile (tile 0 has no chain)
    let mut node = tile;
    while node != 0 {
        let obj = link(c, FLAT_CHAIN, node, 2);
        if obj != 0 {
            let z = rdi(c, this, WALL_BOTTOM, obj);
            if z >= zlo && z < zhi {
                found = true;
                objb(c, OBJ_SEEN, obj, 0);
                objb(c, OBJ_KIND, obj, 1);
                olist_insert(m, c, this, obj, -1)?;
            }
        }
        node = link(c, FLAT_CHAIN, node, 0);
    }

    // FX objects and props
    let mut node = tile;
    while node != 0 {
        let obj = link(c, SPRITE_CHAIN, node, 2);
        if obj != 0 {
            let flag = |c: &Cpu, b: u32| c.rd8(this.wrapping_add(WALL_FLAGS + b).wrapping_add(4 * obj as u32));
            let cnt = this.wrapping_add(OBJ_COUNT_).wrapping_add(2 * obj as u32);
            if flag(c, 3) & 8 != 0 {
                let z = rdi(c, this, WALL_BOTTOM, obj);
                let hh = rdi(c, this, WALL_HEIGHT, obj);
                if z.wrapping_add(hh) > zlo && z.wrapping_sub(hh) < zhi {
                    let n = c.rd16(cnt).wrapping_add(1);
                    c.wr16(cnt, n);
                    found = true;
                    objb(c, OBJ_SEEN, obj, 0);
                    objb(c, OBJ_KIND, obj, 0);
                    fxo_list_insert(m, c, this, obj)?;
                }
            } else if !(flag(c, 2) & 1 != 0 && rd(c, this, RAY_STEPS) >= 3) {
                let top = rdi(c, this, OBJ_TOP_, obj);
                let reach = top.wrapping_add(rdi(c, this, WALL_BOTTOM, obj)).wrapping_add(rdi(c, this, WALL_HEIGHT, obj));
                if top < zhi && reach >= zlo {
                    let pla = this.wrapping_add(PROP_LEVEL_).wrapping_add(2 * obj as u32);
                    if c.rd16(cnt) as i16 <= 0 {
                        c.wr16(cnt, 0);
                        objb(c, OBJ_SEEN, obj, 0);
                    }
                    if rd(c, this, CAM_Z) < rdi(c, this, OBJ_TOP_, obj) && flag(c, 1) & 8 != 0 {
                        let steps = rd(c, this, RAY_STEPS);
                        let d = rd(c, this, LEVEL).wrapping_sub(level).wrapping_abs();
                        if c.rd16(cnt) as i16 == 0 {
                            c.wr16(pla, (steps << 8).wrapping_add(d) as u16);
                        }
                        let pl = c.rd16(pla) as i32;
                        if (pl >> 8) > steps {
                            c.wr16(pla, (steps << 8).wrapping_add(pl & 0xFF) as u16);
                        }
                        let pl = c.rd16(pla) as i32;
                        if rd(c, this, LEVEL).wrapping_sub(pl & 0xFF).wrapping_abs() > d {
                            c.wr16(pla, (pl & 0xFF00).wrapping_add(d) as u16);
                        }
                    }
                    let n = c.rd16(cnt).wrapping_add(1);
                    c.wr16(cnt, n);
                    found = true;
                    objb(c, OBJ_KIND, obj, 2);
                    prop_list_insert(m, c, this, obj)?;
                }
            }
        }
        node = link(c, SPRITE_CHAIN, node, 0);
    }

    // flush the sorted sub-lists into the cell list
    let mut key = -1;
    let mut n = rd(c, this, FXL_TAIL);
    while n != -1 {
        let obj = rdi(c, this, FXL_OBJ, n);
        olist_insert(m, c, this, obj, key)?;
        key = obj;
        n = rdi(c, this, FXL_PREV, n);
    }
    let mut n = rd(c, this, PRL_TAIL);
    while n != -1 {
        let obj = rdi(c, this, PRL_OBJ, n);
        oprop_list_insert(m, c, this, obj)?;
        n = rdi(c, this, PRL_PREV, n);
    }
    Ok(found)
}
