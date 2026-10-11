//! Per-cell render object lists (doubly linked, nodes allocated per frame)
//! and object creation for flats found by the ray caster.

use super::*;
use crate::le::rt;

pub(super) const NODE_COUNT: u32 = 0x4B3A0;
pub(super) const OBJ_HEAD: u32 = 0x4B3A4; // first node per cell (-1 = empty)
pub(super) const OBJ_TAIL: u32 = 0x4F3A4;
pub(super) const NODE_PREV: u32 = 0x533A4;
pub(super) const NODE_NEXT: u32 = 0x5600C;
pub(super) const NODE_OBJ: u32 = 0x58C74;
pub(super) const INSERT_CELL: u32 = 0x5B8DC;
pub(super) const OBJ_KIND: u32 = 0xA3DDA; // u8 per object: 0 fx, 1 flat, 2 prop
pub(super) const NEXT_OBJ: u32 = 0x5E7AC; // next free dynamic object slot
pub(super) const MAX_OBJ: i32 = 0xB1A;
const ID_FLAGS_DW: u32 = 0x5DFA4; // per map id: object flags
const ID_TEX: u32 = 0x5E5A4; // per map id: texture (u16)
const OBJ_TEX: u32 = 0x81DDE; // per object: texture (u16, high half of FLAT_TEXTURE)

// depth comparison tests (still original code)
const FXO_VS_FXO: u32 = 0x059934;
const FXO_VS_FLAT: u32 = 0x05A2F8;
const FXO_VS_PROP: u32 = 0x059D28;
const CAM_VS_CAM: u32 = 0x05A1D0;
const FLAT_VS_PROP: u32 = 0x05A7F0;
/// kind(new) * 3 + kind(existing) -> comparison
const COMPARE: [u32; 9] =
    [FXO_VS_FXO, FXO_VS_FLAT, FXO_VS_PROP, FXO_VS_FLAT, CAM_VS_CAM, FLAT_VS_PROP, FXO_VS_PROP, FLAT_VS_PROP, CAM_VS_CAM];

#[inline]
fn set(c: &mut Cpu, this: u32, off: u32, i: i32, v: i32) {
    c.wr32(this.wrapping_add(off).wrapping_add((i as u32).wrapping_mul(4)), v as u32)
}

/// Create a floor (`top` = false) or ceiling object for map id `id` in the
/// current tile at `level`.  Returns false when the object table is full.
pub fn create_flat(c: &mut Cpu, this: u32, id: i32, level: i32, top: bool) -> bool {
    let tile = rd(c, this, CUR_TILE);
    let (ty, tx) = (tile >> 5, tile & 0x1F);
    let n = rd(c, this, NEXT_OBJ);
    wr(c, this, NEXT_OBJ, n + 1);
    if n + 1 >= MAX_OBJ {
        return false;
    }
    let fl = rdi(c, this, ID_FLAGS_DW, id) | if top { 0x0280_0000 } else { 0x0244_0000 };
    set(c, this, WALL_FLAGS, n, fl);
    set(c, this, WALL_X, n, (tx << 16) + 0x8000);
    set(c, this, WALL_Y, n, (ty << 16) + 0x8000);
    c.wr16(this.wrapping_add(WALL_ANGLE).wrapping_add(2 * n as u32), 0);
    set(c, this, WALL_LENGTH, n, 0x8000);
    set(c, this, WALL_HEIGHT, n, 0x8000);
    let z = if top { ((level + 1) << 16) - 1 } else { level << 16 };
    set(c, this, WALL_BOTTOM, n, z);
    let t = c.rd16(this.wrapping_add(ID_TEX).wrapping_add(2 * id as u32));
    c.wr16(this.wrapping_add(OBJ_TEX).wrapping_add(2 * n as u32), t);
    olist_add(c, this, n, level);
    true
}

pub(super) fn h_create_flat(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    c.r[EAX] = create_flat(c, c.r[EAX], c.r[EDX] as i32, c.r[EBX] as i32, false) as u32;
    c.ret_near(0);
    Ok(())
}

pub(super) fn h_create_top_flat(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    c.r[EAX] = create_flat(c, c.r[EAX], c.r[EDX] as i32, c.r[EBX] as i32, true) as u32;
    c.ret_near(0);
    Ok(())
}

/// Push object `obj` at the front of the list of the current tile at `level`.
pub fn olist_add(c: &mut Cpu, this: u32, obj: i32, level: i32) {
    let cell = rd(c, this, CUR_TILE) + (level << 10);
    let head = rdi(c, this, OBJ_HEAD, cell);
    let k = rd(c, this, NODE_COUNT);
    set(c, this, OBJ_HEAD, cell, k);
    if head < 0 {
        set(c, this, OBJ_TAIL, cell, k);
        set(c, this, NODE_PREV, k, -1);
        set(c, this, NODE_NEXT, k, -1);
    } else {
        set(c, this, NODE_PREV, k, -1);
        set(c, this, NODE_NEXT, k, head);
        set(c, this, NODE_PREV, head, k);
    }
    set(c, this, NODE_OBJ, k, obj);
    wr(c, this, NODE_COUNT, k + 1);
}

pub(super) fn h_olist_add(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    olist_add(c, c.r[EAX], c.r[EDX] as i32, c.r[EBX] as i32);
    c.ret_near(0);
    Ok(())
}

/// Insert object `obj` into the list of cell INSERT_CELL in depth order:
/// before the first entry it must be drawn in front of (or `key` itself).
pub fn olist_insert(m: &mut Machine, c: &mut Cpu, this: u32, obj: i32, key: i32) -> Result<(), Fault> {
    let cell = rd(c, this, INSERT_CELL);
    let head = rdi(c, this, OBJ_HEAD, cell);
    let k = rd(c, this, NODE_COUNT);
    if head < 0 {
        set(c, this, OBJ_HEAD, cell, k);
        set(c, this, OBJ_TAIL, cell, k);
        set(c, this, NODE_PREV, k, -1);
        set(c, this, NODE_NEXT, k, -1);
    } else {
        // `before` keeps its previous value for an out-of-range kind pair
        // (initially the caller's ESI, as in the original)
        let mut before = c.r[ESI];
        let mut node = head;
        let mut last = head;
        loop {
            let id = rdi(c, this, NODE_OBJ, node);
            if id == key {
                before = 1;
            } else {
                let kn = c.rd8(this.wrapping_add(OBJ_KIND).wrapping_add(obj as u32)) as usize;
                let ki = c.rd8(this.wrapping_add(OBJ_KIND).wrapping_add(id as u32)) as usize;
                let idx = kn * 3 + ki;
                if idx <= 8 {
                    before = m.call_guest(c, rt(COMPARE[idx]), &[this, obj as u32, id as u32])?;
                }
            }
            if before != 0 {
                let p = rdi(c, this, NODE_PREV, node);
                if p == -1 {
                    set(c, this, OBJ_HEAD, cell, k);
                } else {
                    set(c, this, NODE_NEXT, p, k);
                }
                set(c, this, NODE_PREV, k, p);
                set(c, this, NODE_NEXT, k, node);
                set(c, this, NODE_PREV, node, k);
                break;
            }
            last = node;
            node = rdi(c, this, NODE_NEXT, node);
            if node == -1 {
                set(c, this, OBJ_TAIL, cell, k);
                set(c, this, NODE_PREV, k, last);
                set(c, this, NODE_NEXT, k, -1);
                set(c, this, NODE_NEXT, last, k);
                break;
            }
        }
    }
    set(c, this, NODE_OBJ, k, obj);
    wr(c, this, NODE_COUNT, k + 1);
    Ok(())
}

pub(super) fn h_olist_insert(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, obj, key) = (c.r[EAX], c.r[EDX] as i32, c.r[EBX] as i32);
    olist_insert(m, c, this, obj, key)?;
    c.ret_near(0);
    Ok(())
}

/// Create the wall object for map id `id` on the current tile side (SIDE)
/// at `level`.  Returns false when the object table is full.
pub fn create_wall(c: &mut Cpu, this: u32, id: i32, level: i32) -> bool {
    let tile = rd(c, this, CUR_TILE);
    let (ty, tx) = (tile >> 5, tile & 0x1F);
    let n = rd(c, this, NEXT_OBJ);
    wr(c, this, NEXT_OBJ, n + 1);
    if n + 1 >= MAX_OBJ {
        return false;
    }
    set(c, this, WALL_FLAGS, n, rdi(c, this, ID_FLAGS_DW, id) | 0x0400_0000);
    let side = rd(c, this, SIDE) as u32;
    if side <= 3 {
        let (x, y) = (tx << 16, ty << 16);
        // anchor, angle, and which axis is constant; corner offsets along the other
        let (wx, wy, ang, offs): (i32, i32, u16, [i32; 4]) = match side {
            0 => (x + 0xFFFF, y + 0x8000, 0, [0x8000, -0x8000, -0x8000, 0x8000]),
            1 => (x + 0x8000, y + 0xFFFF, 0x4000, [-0x8000, 0x8000, 0x8000, -0x8000]),
            2 => (x, y + 0x8000, 0x8000, [-0x8000, 0x8000, 0x8000, -0x8000]),
            _ => (x + 0x8000, y, 0xC000, [0x8000, -0x8000, -0x8000, 0x8000]),
        };
        set(c, this, WALL_X, n, wx);
        set(c, this, WALL_Y, n, wy);
        c.wr16(this.wrapping_add(WALL_ANGLE).wrapping_add(2 * n as u32), ang);
        let q = this.wrapping_add(16 * n as u32);
        for k in 0..4u32 {
            let (px, py) = if side == 0 || side == 2 {
                (wx, wy.wrapping_add(offs[k as usize]))
            } else {
                (wx.wrapping_add(offs[k as usize]), wy)
            };
            c.wr32(q.wrapping_add(WALL_PX_ + 4 * k), px as u32);
            c.wr32(q.wrapping_add(WALL_PY_ + 4 * k), py as u32);
        }
    }
    set(c, this, WALL_LENGTH, n, 0x8000);
    set(c, this, WALL_HEIGHT, n, 0x8000);
    set(c, this, WALL_BOTTOM, n, (level << 16) + 0x8000);
    let t = c.rd16(this.wrapping_add(ID_TEX).wrapping_add(2 * id as u32));
    c.wr16(this.wrapping_add(OBJ_TEX).wrapping_add(2 * n as u32), t);
    olist_add(c, this, n, level);
    true
}
const WALL_PX_: u32 = 0x84A48;
const WALL_PY_: u32 = 0x8FBE8;

pub(super) fn h_create_wall(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    c.r[EAX] = create_wall(c, c.r[EAX], c.r[EDX] as i32, c.r[EBX] as i32) as u32;
    c.ret_near(0);
    Ok(())
}

// ---- dynamic object map (16-bit next/object pairs; entry 0 is the free list)
const OBJECT_MAP: u32 = 0xD7240;

fn omap_cells(c: &Cpu, this: u32, obj: i32) -> [u32;4] {
    let limit = if c.rd8(this.wrapping_add(WALL_FLAGS + 1).wrapping_add((obj as u32).wrapping_mul(4))) & 0x80 != 0 {0x7400} else {0x3800};
    let radius = rdi(c,this,WALL_LENGTH,obj).min(limit);
    let (x,y) = (rdi(c,this,WALL_X,obj),rdi(c,this,WALL_Y,obj));
    let (xp,xm,yp,ym) = (x.wrapping_add(radius),x.wrapping_sub(radius),y.wrapping_add(radius),y.wrapping_sub(radius));
    let cell = |x: i32,y: i32| ((y as u32 >> 16) << 5) | ((x as u32 >> 16) & 31);
    [cell(xp,yp),cell(xm,yp),cell(xm,ym),cell(xp,ym)]
}

pub(super) fn h_omap_insert(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this,obj) = (c.r[EAX],c.r[EDX]);
    let base = c.rd32(this.wrapping_add(OBJECT_MAP));
    for cell in omap_cells(c,this,obj as i32) {
        let p = base.wrapping_add(cell.wrapping_mul(4));
        if c.rd32(p) == 0 { c.wr16(p + 2,obj as u16); continue; }
        let mut node = cell;
        while node != 0 {
            let p = base.wrapping_add(node.wrapping_mul(4));
            if c.rd16(p + 2) as u32 == obj { break; }
            node = c.rd16(p) as u32;
        }
        if node != 0 { continue; } // already present (corners can share a cell)
        if c.rd16(p + 2) == 0 { c.wr16(p + 2,obj as u16); continue; }
        let free = c.rd16(base);
        if free == 0 { continue; } // original silently drops inserts when full
        let q = base.wrapping_add(free as u32 * 4);
        c.wr32(base,c.rd16(q) as u32);
        c.wr16(q + 2,obj as u16);
        c.wr16(q,c.rd16(p));
        c.wr16(p,free);
    }
    c.ret_near(0);
    Ok(())
}

pub(super) fn h_omap_delete(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this,obj) = (c.r[EAX],c.r[EDX]);
    let base = c.rd32(this.wrapping_add(OBJECT_MAP));
    for cell in omap_cells(c,this,obj as i32) {
        let head = base.wrapping_add(cell.wrapping_mul(4));
        if c.rd32(head) == 0 { continue; }
        let (mut node,mut previous) = (cell,cell);
        while node != 0 {
            let p = base.wrapping_add(node.wrapping_mul(4));
            if c.rd16(p + 2) as u32 == obj { break; }
            previous = node;
            node = c.rd16(p) as u32;
        }
        if node == 0 { continue; }
        let p = base.wrapping_add(node.wrapping_mul(4));
        if node == cell { c.wr16(p + 2,0); }
        else {
            c.wr16(base.wrapping_add(previous.wrapping_mul(4)),c.rd16(p));
            c.wr32(p,c.rd32(base));
            c.wr32(base,node);
        }
    }
    c.ret_near(0);
    Ok(())
}

#[cfg(test)]
mod omap_tests {
    use super::*;
    #[test]
    fn omap_preserves_duplicates_tombstones_free_list_and_corner_order() {
        let path = std::path::PathBuf::from("__omap_no_files__");
        let mut m = Machine::new(crate::vfs::Vfs::new(path.clone(),path));
        let mut c = Cpu::new();
        let (this,base) = (0x100000,0x300000);
        c.wr32(this + OBJECT_MAP,base);
        c.wr32(base,1024); c.wr32(base + 1024*4,1025);
        let invoke = |c: &mut Cpu,m: &mut Machine,obj: u32,insert: bool| {
            c.r[ESP] = 0x10000; c.push32(0x1234); c.r[EAX] = this; c.r[EDX] = obj;
            if insert {h_omap_insert(m,c).unwrap();} else {h_omap_delete(m,c).unwrap();}
            assert_eq!((c.eip,c.r[ESP]),(0x1234,0x10000));
        };
        for obj in 1..=3 {
            set(&mut c,this,WALL_X,obj,0x58000); set(&mut c,this,WALL_Y,obj,0x68000);
            set(&mut c,this,WALL_LENGTH,obj,0x8000);
        }
        let cell = 6*32+5;
        invoke(&mut c,&mut m,1,true); invoke(&mut c,&mut m,1,true);
        assert_eq!(c.rd32(base+cell*4),1<<16);
        assert_eq!(c.rd32(base),1024);
        invoke(&mut c,&mut m,2,true);
        assert_eq!(c.rd32(base+cell*4),(1<<16)|1024);
        assert_eq!(c.rd32(base+1024*4),2<<16);
        assert_eq!(c.rd32(base),1025);
        invoke(&mut c,&mut m,1,false); // clearing head leaves its chained entries intact
        assert_eq!(c.rd32(base+cell*4),1024);
        invoke(&mut c,&mut m,3,true); // reuse the head tombstone, no free node needed
        assert_eq!(c.rd32(base+cell*4),(3<<16)|1024);
        invoke(&mut c,&mut m,2,false); // unlink and recycle non-head
        assert_eq!(c.rd32(base+cell*4),3<<16);
        assert_eq!(c.rd32(base),1024);
        assert_eq!(c.rd32(base+1024*4),1025);
        c.wr32(base,0); invoke(&mut c,&mut m,2,true);
        assert_eq!(c.rd32(base+cell*4),3<<16); // exhausted pool
        set(&mut c,this,WALL_X,1,0xC0000); set(&mut c,this,WALL_Y,1,0x100000);
        assert_eq!(omap_cells(&c,this,1),[524,523,491,492]);
        invoke(&mut c,&mut m,1,true);
        for cell in [524,523,491,492] {assert_eq!(c.rd16(base+cell*4+2),1);}
    }
}

// ---- visible cell list (sorted by ray step / level distance / column)
const VIS_COUNT: u32 = 0x5BEF8;
const VIS_HEAD: u32 = 0x5BEFC;
const VIS_STEP_FIRST: u32 = 0x5BF00; // first node per ray step
const VIS_INFO: u32 = 0x5BF98;
const VIS_PREV: u32 = 0x5C798;
const VIS_NEXT: u32 = 0x5CF98;
const VIS_CELL: u32 = 0x5D798;
const RAY_ORDER: u32 = 0xBA288;

/// Insert map cell `cell` (seen on `level`) into the visible cell list.
pub fn tlist_insert(c: &mut Cpu, this: u32, cell: i32, level: i32) {
    let steps = rd(c, this, RAY_STEPS);
    let key = (rd(c, this, LEVEL).wrapping_sub(level).wrapping_abs().wrapping_add(steps << 8) << 16)
        .wrapping_add(rd(c, this, RAY_ORDER)) as u32;
    let k = rd(c, this, VIS_COUNT);
    let head = rd(c, this, VIS_HEAD);
    let finish = |c: &mut Cpu| {
        set(c, this, VIS_INFO, k, key as i32);
        set(c, this, VIS_CELL, k, cell);
        wr(c, this, VIS_COUNT, k + 1);
    };
    if head < 0 {
        wr(c, this, VIS_HEAD, k);
        set(c, this, VIS_STEP_FIRST, steps, k);
        set(c, this, VIS_PREV, k, -1);
        set(c, this, VIS_NEXT, k, -1);
        finish(c);
        return;
    }
    let s = rdi(c, this, VIS_STEP_FIRST, steps);
    let start = if s < 0 {
        set(c, this, VIS_STEP_FIRST, steps, k);
        rd(c, this, VIS_HEAD)
    } else {
        s
    };
    let mut node = start;
    let mut last = start;
    while node != -1 {
        if key > rdi(c, this, VIS_INFO, node) as u32 {
            if node == rdi(c, this, VIS_STEP_FIRST, steps) {
                set(c, this, VIS_STEP_FIRST, steps, k);
            }
            let p = rdi(c, this, VIS_PREV, node);
            if p == -1 {
                wr(c, this, VIS_HEAD, k);
            } else {
                set(c, this, VIS_NEXT, p, k);
            }
            set(c, this, VIS_PREV, k, rdi(c, this, VIS_PREV, node));
            set(c, this, VIS_NEXT, k, node);
            set(c, this, VIS_PREV, node, k);
            finish(c);
            return;
        }
        last = node;
        node = rdi(c, this, VIS_NEXT, node);
    }
    set(c, this, VIS_PREV, k, last);
    set(c, this, VIS_NEXT, k, -1);
    set(c, this, VIS_NEXT, last, k);
    finish(c);
}

pub(super) fn h_tlist_insert(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    tlist_insert(c, c.r[EAX], c.r[EDX] as i32, c.r[EBX] as i32);
    c.ret_near(0);
    Ok(())
}

// ---- temporary sorted sub-lists used by CheckObjects
/// FX objects (sorted with FxoVSFxo)
const FXL_COUNT: u32 = 0x5BBEC;
const FXL_HEAD: u32 = 0x5BBF0;
const FXL_TAIL: u32 = 0x5BBF4;
const FXL_PREV: u32 = 0x5BBF8;
const FXL_NEXT: u32 = 0x5BCF8;
const FXL_OBJ: u32 = 0x5BDF8;
/// props (sorted with CamVSCam)
const PRL_COUNT: u32 = 0x5B8E0;
const PRL_HEAD: u32 = 0x5B8E4;
const PRL_TAIL: u32 = 0x5B8E8;
const PRL_PREV: u32 = 0x5B8EC;
const PRL_NEXT: u32 = 0x5B9EC;
const PRL_OBJ: u32 = 0x5BAEC;

/// Sorted insert into a sub-list: walk from head along `next`, insert before
/// the first entry the comparison says `obj` precedes.
#[allow(clippy::too_many_arguments)]
fn sublist_insert(
    m: &mut Machine,
    c: &mut Cpu,
    this: u32,
    obj: i32,
    cmp: u32,
    (count, head, tail, prev, next, objs): (u32, u32, u32, u32, u32, u32),
) -> Result<(), Fault> {
    let h = rd(c, this, head);
    if h < 0 {
        wr(c, this, count, 0);
        wr(c, this, head, 0);
        wr(c, this, tail, 0);
        set(c, this, prev, 0, -1);
        set(c, this, next, 0, -1);
        set(c, this, objs, 0, obj);
        wr(c, this, count, 1);
        return Ok(());
    }
    let k = rd(c, this, count);
    let mut node = h;
    let mut last = h;
    while node != -1 {
        let other = rdi(c, this, objs, node);
        if m.call_guest(c, rt(cmp), &[this, obj as u32, other as u32])? != 0 {
            let p = rdi(c, this, prev, node);
            if p == -1 {
                wr(c, this, head, k);
            } else {
                set(c, this, next, p, k);
            }
            set(c, this, prev, k, rdi(c, this, prev, node));
            set(c, this, next, k, node);
            set(c, this, prev, node, k);
            set(c, this, objs, k, obj);
            wr(c, this, count, k + 1);
            return Ok(());
        }
        last = node;
        node = rdi(c, this, next, node);
    }
    wr(c, this, tail, k);
    set(c, this, prev, k, last);
    set(c, this, next, k, -1);
    set(c, this, next, last, k);
    set(c, this, objs, k, obj);
    wr(c, this, count, k + 1);
    Ok(())
}

pub fn fxo_list_insert(m: &mut Machine, c: &mut Cpu, this: u32, obj: i32) -> Result<(), Fault> {
    sublist_insert(m, c, this, obj, FXO_VS_FXO, (FXL_COUNT, FXL_HEAD, FXL_TAIL, FXL_PREV, FXL_NEXT, FXL_OBJ))
}

pub fn prop_list_insert(m: &mut Machine, c: &mut Cpu, this: u32, obj: i32) -> Result<(), Fault> {
    sublist_insert(m, c, this, obj, CAM_VS_CAM, (PRL_COUNT, PRL_HEAD, PRL_TAIL, PRL_PREV, PRL_NEXT, PRL_OBJ))
}

pub(super) fn h_fxo_list_insert(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, obj) = (c.r[EAX], c.r[EDX] as i32);
    fxo_list_insert(m, c, this, obj)?;
    c.ret_near(0);
    Ok(())
}

pub(super) fn h_prop_list_insert(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, obj) = (c.r[EAX], c.r[EDX] as i32);
    prop_list_insert(m, c, this, obj)?;
    c.ret_near(0);
    Ok(())
}

/// kind(new) * 3 + kind(existing) -> comparison for OPROPListInsert (0 = always 1)
const COMPARE_OPROP: [u32; 9] =
    [FXO_VS_FXO, FXO_VS_FLAT, FXO_VS_PROP, FXO_VS_FLAT, CAM_VS_CAM, FLAT_VS_PROP, FXO_VS_PROP, FLAT_VS_PROP, 0];

/// Insert prop `obj` into the cell list scanning from the tail: it goes after
/// the last entry the comparison does not put it in front of.
pub fn oprop_list_insert(m: &mut Machine, c: &mut Cpu, this: u32, obj: i32) -> Result<(), Fault> {
    let cell = rd(c, this, INSERT_CELL);
    let head = rdi(c, this, OBJ_HEAD, cell);
    let k = rd(c, this, NODE_COUNT);
    if head < 0 {
        set(c, this, OBJ_HEAD, cell, k);
        set(c, this, OBJ_TAIL, cell, k);
        set(c, this, NODE_PREV, k, -1);
        set(c, this, NODE_NEXT, k, -1);
    } else {
        let mut node = rdi(c, this, OBJ_TAIL, cell);
        let mut last = node;
        let mut before = c.r[ESI];
        let mut placed = false;
        while node != -1 {
            let other = rdi(c, this, NODE_OBJ, node);
            let kn = c.rd8(this.wrapping_add(OBJ_KIND).wrapping_add(obj as u32)) as usize;
            let ki = c.rd8(this.wrapping_add(OBJ_KIND).wrapping_add(other as u32)) as usize;
            let idx = kn * 3 + ki;
            if idx <= 8 {
                before = if COMPARE_OPROP[idx] == 0 {
                    1
                } else {
                    m.call_guest(c, rt(COMPARE_OPROP[idx]), &[this, obj as u32, other as u32])?
                };
            }
            if before == 0 {
                let n2 = rdi(c, this, NODE_NEXT, node);
                if n2 == -1 {
                    set(c, this, OBJ_TAIL, cell, k);
                } else {
                    set(c, this, NODE_PREV, n2, k);
                }
                set(c, this, NODE_NEXT, k, rdi(c, this, NODE_NEXT, node));
                set(c, this, NODE_PREV, k, node);
                set(c, this, NODE_NEXT, node, k);
                placed = true;
                break;
            }
            last = node;
            node = rdi(c, this, NODE_PREV, node);
        }
        if !placed {
            set(c, this, OBJ_HEAD, cell, k);
            set(c, this, NODE_NEXT, k, last);
            set(c, this, NODE_PREV, k, -1);
            set(c, this, NODE_PREV, last, k);
        }
    }
    set(c, this, NODE_OBJ, k, obj);
    wr(c, this, NODE_COUNT, k + 1);
    Ok(())
}

pub(super) fn h_oprop_list_insert(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, obj) = (c.r[EAX], c.r[EDX] as i32);
    oprop_list_insert(m, c, this, obj)?;
    c.ret_near(0);
    Ok(())
}
