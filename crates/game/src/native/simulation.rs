//! Collision queries, object scans, animation and common movement bookkeeping.
use crate::machine::Machine;
use x86::*;

pub fn install(m: &mut Machine, c: &mut Cpu) {
    m.hook_sig(c, 0x02DBA8, "Xox::MakeMissileList", 1, false, missile_list);
    m.hook_sig(c, 0x041C4C, "Xox::CollSwimFlat", 4, true, swim_flat);
    m.hook_sig(c, 0x041B30, "Xox::CollFlat", 4, false, coll_flat);
    m.hook_sig(c, 0x041978, "Xox::CollTopFlat", 4, true, coll_top_flat);
    m.hook_sig(c, 0x02AB30, "Xox::FindTarget", 3, false, find_target);
    m.hook_sig(c, 0x033ED0, "Xox::MoveObject", 4, false, move_object);
    m.hook_sig(c, 0x047B90, "Xox::MoveMonster", 4, false, move_monster);
    m.hook_sig(c, 0x02AD6C, "Xox::Turn", 4, true, turn);
    m.hook_sig(c, 0x02A9B0, "Xox::SightPerson", 4, false, sight_person);
    m.hook_sig(c, 0x0344E0, "Xox::TrackTarget", 4, false, track_target);
    m.hook_sig(c, 0x044EF4, "Xox::PathCheck", 3, true, path_check);
    m.hook_sig(c, 0x034488, "Xox::UpdatePCObject", 2, false, update_pc_object);
    m.hook_sig(c, 0x0348C4, "Xox::UpdateObjects", 2, false, update_objects);
}

fn missile_list(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let this = c.r[EAX];
    c.wr32(this.wrapping_add(0x136224), 0);
    let mut obj = 1i32;
    while obj < c.rd32(this.wrapping_add(0x130D48)) as i32 {
        let p = this.wrapping_add((obj as u32).wrapping_mul(4));
        if c.rd8(this.wrapping_add(0x7DB42).wrapping_add(obj as u32)) != 0
            && c.rd8(p.wrapping_add(0x5E7B3)) & 3 == 0
            && c.rd8(p.wrapping_add(0x5E7B1)) & 4 == 0 {
            let n = c.rd32(this.wrapping_add(0x136224));
            c.wr32(this.wrapping_add(0x136228).wrapping_add(n.wrapping_mul(4)), obj as u32);
            c.wr32(this.wrapping_add(0x136224), n.wrapping_add(1));
        }
        obj = obj.wrapping_add(1);
    }
    c.ret_near(0);
    Ok(())
}

/// Skip inactive actors without interpreting the table/definition/loop scan.
/// ponytail: AI states remain guest code; port individual states if profiling
/// shows their bookkeeping dominates after the native scans and leaf ports.
fn update_objects(m: &mut Machine,c: &mut Cpu)->Result<(),Fault> {
    let this = c.r[EAX];
    for reg in [EBX,ECX,ESI,EDI,EBP] {c.push32(c.r[reg]);}
    c.r[ESP] = c.r[ESP].wrapping_sub(0x74);
    c.r[EDI]=this; c.r[EBX]=c.r[EDX]; c.eip=crate::le::rt(0x0348D0);
    let end = crate::le::rt(0x036F06);
    while c.eip != crate::le::rt(0x0349A5) && c.eip != end {c.step(m)?;}
    let sp = c.r[ESP];
    while c.eip != end {
        let record = c.rd32(c.rd32(sp.wrapping_add(0x6C)).wrapping_add(0x13141C));
        let obj = c.rd16(record) as i16 as i32 as u32;
        if c.rd8(this.wrapping_add(0x7DB42).wrapping_add(obj)) != 0 {
            c.eip=if prepare_actor(m,c,this,sp,record,obj)? {crate::le::rt(0x034D54)} else {crate::le::rt(0x0349A5)};
            while c.eip != crate::le::rt(0x0349E4) && c.eip != end {c.step(m)?;}
            if c.eip == end {break;}
        } else {
            c.wr32(sp.wrapping_add(0x10),obj);
            c.r[EAX]=0;
        }
        c.r[ECX]=c.rd32(sp.wrapping_add(0x6C)).wrapping_add(4);
        c.r[ESI]=c.rd32(sp.wrapping_add(4)).wrapping_add(1);
        c.r[EBP]=c.rd32(this.wrapping_add(0x130D24));
        c.wr32(sp.wrapping_add(0x6C),c.r[ECX]); c.wr32(sp.wrapping_add(4),c.r[ESI]);
        c.eip=if (c.r[ESI] as i32) >= c.r[EBP] as i32 {end} else {crate::le::rt(0x0349A5)};
    }
    c.r[ESP]=c.r[ESP].wrapping_add(0x74);
    for reg in [EBP,EDI,ESI,ECX,EBX] {c.r[reg]=c.pop32();}
    c.ret_near(0);
    Ok(())
}

/// Common actor bookkeeping. Water transitions, damage, buffs and corpses
/// fall back to the original block; their uncommon sound/state paths stay intact.
fn prepare_actor(m: &mut Machine,c: &mut Cpu,this: u32,sp: u32,record: u32,obj: u32)->Result<bool,Fault> {
    if [0x5C,0x74,0x58].iter().any(|&off| c.rd32(record.wrapping_add(off)) != 0)
        || c.rd32(sp.wrapping_add(0x54)) != 0
        || (c.rd32(this.wrapping_add(0x130CEC)) == 3 && c.rd32(sp.wrapping_add(4)) == 0x52) {return Ok(false);}
    let def = actor_definition(c,this,record);
    if c.rd32(crate::le::rt(0x1E3924)) != 0 {
        let water = m.call_guest(c,crate::le::rt(0x041C4C),&[this,obj,c.rd32(crate::le::rt(0x9260C)),c.rd32(crate::le::rt(0x92610))])? != 0;
        if water != (c.rd16(record.wrapping_add(0xB6)) != 0) {return Ok(false);}
        c.wr16(record.wrapping_add(0xB6),u16::from(water));
        let flags = this.wrapping_add(0x5E7B1).wrapping_add(obj.wrapping_mul(4));
        c.wr8(flags,if water {c.rd8(flags)|0x10} else {c.rd8(flags)&!0x10});
    }
    let kind = c.rd8(def.wrapping_add(0x3A));
    let tick = if kind == 17 {1} else {c.rd32(sp.wrapping_add(if kind == 9 {0x5C} else {0x58}))};
    let fire = c.rd32(crate::le::rt(0x92EE0).wrapping_add(c.rd8(def.wrapping_add(0x80)) as u32*4));
    let throw = c.rd32(crate::le::rt(0x92F10).wrapping_add(c.rd8(def.wrapping_add(0x98)) as u32*4));
    for (off,value) in [(0,tick),(0x10,obj),(0x48,1),(0x50,0),(0x20,u32::from(c.rd8(def.wrapping_add(0x1C)) != 0)),
        (0x18,def.wrapping_add(0x18)),(0x1C,1),(0x2C,c.rd32(def.wrapping_add(0x5C))),(0x24,c.rd32(def.wrapping_add(0x74))),
        (8,fire),(0x34,throw),(0x4C,0)] {c.wr32(sp.wrapping_add(off),value);}
    if ![28,29].contains(&c.rd8(record.wrapping_add(0xBC))) {c.wr8(this.wrapping_add(0x83412).wrapping_add(obj),8);}
    c.r[ESI]=record; c.r[EBP]=def; c.r[ECX]=0; c.r[EDX]=0;
    c.r[EBX]=if c.rd32(crate::le::rt(0x1E3924)) != 0 {c.rd32(crate::le::rt(0x9260C))} else {0};
    c.r[EAX]=(throw&!0xFF)|kind as u32;
    Ok(true)
}

fn update_pc_object(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, tick) = (c.r[EAX], c.r[EDX]);
    let record = actor(c, this, c.rd32(this.wrapping_add(0x130D08)));
    let hidden = c.rd32(record.wrapping_add(0x5C));
    if hidden != 0 {
        let obj = c.rd32(this.wrapping_add(0x130D04));
        c.r[EBX] = obj; // original clobbers this non-argument register
        let flags = this.wrapping_add(0x5E7B2).wrapping_add(obj.wrapping_mul(4));
        c.wr8(flags, c.rd8(flags) | 2);
        if tick != 0 {
            c.wr32(record.wrapping_add(0x5C), hidden.wrapping_sub(1));
            if hidden == 1 { c.wr8(flags, c.rd8(flags) & !2); }
        }
    }
    let aura = c.rd32(record.wrapping_add(0x74));
    if tick != 0 && aura != 0 { c.wr32(record.wrapping_add(0x74), aura.wrapping_sub(1)); }
    c.ret_near(0);
    Ok(())
}

fn path_check(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, x, y) = (c.r[EAX], c.r[EDX] as i32, c.r[EBX] as i32);
    let tile = (x >> 16).wrapping_add((y >> 16).wrapping_shl(5)) as u32;
    let shift = ((x >> 14) % 4).wrapping_add(((y >> 14) % 4).wrapping_mul(4)) as u32;
    let mask = c.rd16(c.rd32(this.wrapping_add(0xCA098)).wrapping_add(tile.wrapping_mul(2))) as u32;
    let mut blocked = mask & 1u32.wrapping_shl(shift) != 0;
    if blocked && c.rd32(this.wrapping_add(0x1423F0)) == 0 {
        let region = c.rd8(c.rd32(this.wrapping_add(0xC6094)).wrapping_add(tile)) as u32;
        let p = this.wrapping_add(region.wrapping_mul(4));
        if c.rd8(this.wrapping_add(0x142390).wrapping_add(region)) != 0
            && x >= c.rd32(p.wrapping_add(0x141D90)) as i32 && x <= c.rd32(p.wrapping_add(0x141F10)) as i32
            && y >= c.rd32(p.wrapping_add(0x142090)) as i32 && y <= c.rd32(p.wrapping_add(0x142210)) as i32 {
            blocked = false;
        }
    }
    c.r[EAX] = u32::from(blocked);
    c.ret_near(0);
    Ok(())
}

fn track_target(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, obj, other, offset) = (c.r[EAX], c.r[EDX], c.r[EBX], c.r[ECX]);
    let mut angle = super::image1::atan64k(m, c, object_field(c,this,obj,0x61418), object_field(c,this,obj,0x64080),
        object_field(c,this,other,0x61418), object_field(c,this,other,0x64080))? as u16;
    let target = this.wrapping_add(0x7FC90).wrapping_add(obj.wrapping_mul(2));
    c.wr16(target, angle);
    c.wr8(this.wrapping_add(0x812C4).wrapping_add(obj), 2);
    if offset != 0 {
        let heading = c.rd16(this.wrapping_add(0x7E65C).wrapping_add(other.wrapping_mul(2)));
        angle = if angle.wrapping_sub(heading) < 0x8000 { angle.wrapping_add(0x1555) } else { angle.wrapping_sub(0x1555) };
        c.wr16(target, angle);
    }
    c.ret_near(0);
    Ok(())
}

fn sight_person(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, source, target, range) = (c.r[EAX], c.r[EDX], c.r[EBX], c.r[ECX] as i32);
    let record = actor(c, this, source);
    let obj = c.rd16(record) as i16 as i32 as u32;
    let other_record = actor(c, this, target);
    let other = c.rd16(other_record) as i16 as i32 as u32;
    c.wr32(record.wrapping_add(8), u32::MAX);
    if (c.rd32(other_record.wrapping_add(0x34)) as i32) >> 16 != 2
        && (range <= 0 || c.rd32(other_record.wrapping_add(0x5C)) == 0) {
        let range = if range <= 0 {0x70000} else {range};
        let (x,y) = (object_field(c,this,obj,0x61418),object_field(c,this,obj,0x64080));
        let (tx,ty) = (object_field(c,this,other,0x61418),object_field(c,this,other,0x64080));
        let dist = super::image1::distance(x,y,tx,ty);
        if dist < range {
            let angle = super::image1::atan64k(m,c,x,y,tx,ty)? as u16;
            let heading = c.rd16(this.wrapping_add(0x7E65C).wrapping_add(obj.wrapping_mul(2)));
            if (angle.wrapping_sub(heading) as i16 as i32).abs() < 0x4000 {
                let z = object_field(c,this,obj,0x78D8C).wrapping_add(object_field(c,this,obj,0x66CE8)).wrapping_add(object_field(c,this,obj,0x6C5B8));
                let tz = object_field(c,this,other,0x78D8C).wrapping_add(
                    if other == c.rd32(this.wrapping_add(0x130D04)) {c.rd32(this.wrapping_add(0x130D1C)) as i32}
                    else {object_field(c,this,other,0x66CE8).wrapping_add(object_field(c,this,other,0x6C5B8))});
                if m.call_guest(c,crate::le::rt(0x05D6D4),&[this,obj,other,dist as u32,z as u32,tz as u32,angle as u32])? == 1 {
                    c.wr32(actor(c,this,source).wrapping_add(8),target);
                    c.wr32(actor(c,this,source).wrapping_add(0xC),dist as u32);
                }
            }
        }
    }
    c.ret_near(0);
    Ok(())
}

fn turn(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (current, target, step) = (c.r[EDX], c.r[EBX], c.r[ECX] as i32);
    c.r[EAX] = if current as u16 == target as u16 { current } else {
        let mut a = current as i16 as i32 + 0x10000;
        let mut b = target as i16 as i32 + 0x10000;
        if a >= b + 0x8000 { a %= 0x10000; }
        if b >= a + 0x8000 { b %= 0x10000; }
        a = if a < b { a.wrapping_add(step).min(b) } else { a.wrapping_sub(step).max(b) };
        (a % 0x10000) as u32
    };
    c.ret_near(0);
    Ok(())
}

/// Animation tables, bob/knockback and floor following. Retain guest movement
/// and sound callees; only the per-actor bookkeeping is replaced.
fn move_object(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, id, tick, moving) = (c.r[EAX], c.r[EDX], c.r[EBX], c.r[ECX]);
    let record = actor(c, this, id);
    let obj = c.rd16(record) as i16 as i32 as u32;
    m.call_guest(c, crate::le::rt(0x0581DC), &[this, obj])?;
    let field = |c: &Cpu, off: u32| c.rd32(record.wrapping_add(off)) as i32;
    let mut bob = 0;
    if field(c, 0x60) != 0 {
        if tick != 0 {
            let rising = field(c, 0x68) != 0;
            let speed = field(c, 0x64).wrapping_add(if rising {0x200} else {-0x200});
            c.wr32(record.wrapping_add(0x64), speed as u32);
            if (rising && speed > 0) || (!rising && speed < 0) { c.wr32(record.wrapping_add(0x68), u32::from(!rising)); }
        }
        bob = field(c, 0x60).wrapping_add(field(c, 0x64));
    }
    let def = actor_definition(c, this, record);
    let animation = c.rd16(def.wrapping_add(2)) as i16 as i32 as u32;
    let images = c.rd32(this.wrapping_add(0x131B4C).wrapping_add(animation.wrapping_mul(4)));
    let sequence = c.rd32(this.wrapping_add(0x131B8C).wrapping_add(((c.rd32(images) as i32 >> 16) as u32).wrapping_mul(4)));
    let frame;
    if tick != 0 || c.rd16(record.wrapping_add(0x16)) != 0 {
        c.wr16(record.wrapping_add(0x18), 0);
        let mut cursor;
        if c.rd16(record.wrapping_add(0x16)) != 0 {
            cursor = c.rd8(sequence.wrapping_add(0xE + c.rd8(record.wrapping_add(0xBC)) as u32)) as u32;
            frame = c.rd8(sequence.wrapping_add(0x2C + cursor)) as u32;
            c.wr16(record.wrapping_add(0x16), 0);
        } else {
            cursor = c.rd8(record.wrapping_add(0xBD)) as u32 + 1;
            let next = c.rd8(sequence.wrapping_add(0x2C + cursor));
            if next == 0xFF {
                cursor = c.rd8(sequence.wrapping_add(0xE + c.rd8(record.wrapping_add(0xBC)) as u32)) as u32;
                frame = c.rd8(sequence.wrapping_add(0x2C + cursor)) as u32;
                c.wr16(record.wrapping_add(0x18), 1);
            } else { frame = next as u32; }
        }
        c.wr8(record.wrapping_add(0xBD), cursor as u8);
        for (source, dest) in [(0x7C,0x69950),(0xCC,0x6C5B8),(0x11C,0x6F220),(0x16C,0x71E88)] {
            c.wr32(this.wrapping_add(obj.wrapping_mul(4)).wrapping_add(dest), c.rd32(sequence.wrapping_add(frame*4+source)));
        }
        let image = c.rd8(sequence.wrapping_add(frame + 0x25C)) as u32;
        c.wr16(this.wrapping_add(0x81DDE).wrapping_add(obj.wrapping_mul(2)), c.rd16(images.wrapping_add(image*2+0xC)));
        c.wr8(this.wrapping_add(0x83F2C).wrapping_add(obj), c.rd8(images.wrapping_add(image+0xB4)));
        if c.rd8(actor_definition(c, this, record).wrapping_add(0x3A)) == 10 && (image == 3 || image == 0) {
            m.call_guest(c, crate::le::rt(0x027F2C), &[this, if image == 3 {111} else {112}, obj, 0x40000, 0x9FFF, 0])?;
        }
    } else {
        frame = c.rd8(sequence.wrapping_add(0x2C + c.rd8(record.wrapping_add(0xBD)) as u32)) as u32;
    }
    if field(c, 0xA4) != 0 {
        if id != c.rd32(this.wrapping_add(0x130D08)) {
            let heading = this.wrapping_add(0x7E65C).wrapping_add(obj.wrapping_mul(2));
            let saved = c.rd16(heading);
            c.wr16(heading, c.rd16(record.wrapping_add(0xB4)));
            let limit = c.rd32(crate::le::rt(0x1E3944)) != 0;
            let speed = if limit { field(c, 0xA8) / 128 } else { 2 };
            m.call_guest(c, crate::le::rt(0x047B90), &[this, obj, speed as u32, field(c,0xB0).wrapping_add(bob) as u32])?;
            c.wr16(heading, saved);
            if limit {
                let speed = if c.rd16(record.wrapping_add(0xB6)) != 0 {
                    let speed = field(c, 0xA8) / 2;
                    c.wr32(record.wrapping_add(0xAC), 0); c.wr32(record.wrapping_add(0xB0), 0);
                    speed
                } else { field(c, 0xA8).wrapping_mul(4) / 5 };
                c.wr32(record.wrapping_add(0xA8), speed as u32);
                if field(c, 0xA4) > 0 {
                    let remaining = field(c, 0xA4).wrapping_sub(1);
                    c.wr32(record.wrapping_add(0xA4), remaining as u32);
                    if remaining == 0 || (speed < 0x4000 && field(c,0xB0) / 2 > speed) {
                        c.wr32(record.wrapping_add(0xA4), u32::MAX);
                        c.wr32(record.wrapping_add(0xAC), if c.rd32(this.wrapping_add(0x5E7A8)) != 0 {0x4CC} else {0x800});
                    } else {
                        let accel = field(c, 0xAC).wrapping_mul(2) / 3;
                        c.wr32(record.wrapping_add(0xAC), accel as u32);
                        c.wr32(record.wrapping_add(0xB0), field(c, 0xB0).wrapping_add(accel) as u32);
                    }
                }
                if field(c, 0xA4) < 0 {
                    let accel = field(c, 0xAC).wrapping_mul(2);
                    let speed = field(c, 0xB0).wrapping_sub(accel);
                    c.wr32(record.wrapping_add(0xAC), accel as u32);
                    c.wr32(record.wrapping_add(0xB0), speed as u32);
                    if speed <= 0 { c.wr32(record.wrapping_add(0xA4), 0); c.wr32(record.wrapping_add(0xB0), 0); }
                }
            }
        }
    } else if moving != 0 {
        let speed = c.rd32(sequence.wrapping_add(frame*4 + 0x1BC));
        let (speed, vertical) = if tick != 0 {
            (speed, c.rd32(sequence.wrapping_add(frame*4 + 0x20C)).wrapping_shl(6).wrapping_add(bob as u32))
        } else { (if speed != 0 {2} else {0}, bob as u32) };
        m.call_guest(c, crate::le::rt(0x047B90), &[this, obj, speed, vertical])?;
    } else if id != c.rd32(this.wrapping_add(0x130D08)) && c.rd8(actor_definition(c, this, record).wrapping_add(0x3A)) != 14 {
        let z = this.wrapping_add(obj.wrapping_mul(4)).wrapping_add(0x78D8C);
        let old = c.rd32(z) as i32;
        m.call_guest(c, crate::le::rt(0x041B30), &[this, obj, object_field(c,this,obj,0x61418) as u32, object_field(c,this,obj,0x64080) as u32])?;
        if bob != 0 {
            let floor = c.rd32(z) as i32;
            c.wr32(z, floor.wrapping_add(bob) as u32);
            m.call_guest(c, crate::le::rt(0x041978), &[this, obj, old.max(floor) as u32, 0x1100])?;
            if floor > c.rd32(z) as i32 { c.wr32(z, floor as u32); }
        }
        if old.wrapping_sub(0x4000) > c.rd32(z) as i32 {
            let fall = 0x4000i64 * c.rd32(this.wrapping_add(0x144B6C)) as i32 as i64 / 2;
            let fall = i32::try_from(fall).map_err(|_| Fault::DivideError)?;
            c.wr32(z, old.wrapping_sub(fall) as u32);
        }
    }
    m.call_guest(c, crate::le::rt(0x0584C0), &[this, obj])?;
    c.ret_near(0);
    Ok(())
}

/// Movement setup and ordinary floor following; retain both original boundary
/// solvers and the exceptional bob/knockback/ceiling branch as guest code.
fn move_monster(m: &mut Machine,c: &mut Cpu)->Result<(),Fault> {
    let (this,obj,speed,vertical)=(c.r[EAX],c.r[EDX],c.r[EBX] as i32,c.r[ECX]);
    for reg in [ESI,EDI,EBP] {c.push32(c.r[reg]);} c.r[ESP]=c.r[ESP].wrapping_sub(0x44);
    let sp=c.r[ESP];
    let owner=c.rd16(this.wrapping_add(0x7B9F4).wrapping_add(obj.wrapping_mul(2))) as i16 as i32 as u32;
    let record=actor(c,this,owner); let def=actor_definition(c,this,record);
    c.r[ESI]=this; c.r[EDI]=obj; c.r[EBP]=record;
    c.wr32(this.wrapping_add(0x144504),0); c.wr32(this.wrapping_add(0x130D18),0);
    let kind=c.rd8(def.wrapping_add(0x3A)); let airborne=matches!(kind,9|16|10);
    match kind {17|5=>c.wr32(this.wrapping_add(0x1423F0),1),9|16|10=>{},_=>c.wr32(this.wrapping_add(0x1423F0),0)}
    let level=move_collision_level(c,this,object_field(c,this,obj,0x78D8C),c.rd32(record.wrapping_add(0xA4))!=0);
    let xpos=this.wrapping_add(obj.wrapping_mul(4)).wrapping_add(0x61418);
    let ypos=this.wrapping_add(obj.wrapping_mul(4)).wrapping_add(0x64080);
    let zpos=this.wrapping_add(obj.wrapping_mul(4)).wrapping_add(0x78D8C);
    if m.call_guest(c,crate::le::rt(0x044EF4),&[this,c.rd32(xpos),c.rd32(ypos)])? as u16!=0 {
        c.wr32(xpos,snap_path_axis(c.rd32(xpos) as i32) as u32);
        c.wr32(ypos,snap_path_axis(c.rd32(ypos) as i32) as u32);
    }
    let heading=this.wrapping_add(0x7E65C).wrapping_add(obj.wrapping_mul(2));
    let riding=c.rd32(record.wrapping_add(0x7C));
    if riding!=0 {c.wr16(heading,c.rd16(this.wrapping_add(0x7E65C).wrapping_add(riding.wrapping_mul(2))));}
    c.wr16(crate::le::rt(0x206B20),c.rd16(heading));
    let speed=speed.clamp(-256,256);
    for (off,value) in [(0,c.rd32(ypos)),(4,c.rd32(xpos)),(0x14,vertical),(0x18,u32::from(airborne)),(0x2C,level as u32),(0x30,speed as u32)] {c.wr32(sp.wrapping_add(off),value);}
    if speed!=0 {
        let angle=(c.rd16(heading)>>5) as u32; let velocity=speed.wrapping_shl(7);
        let dx=super::image1::mul16(velocity,c.rd32(this.wrapping_add(0xB827C).wrapping_add(angle.wrapping_mul(4))) as i32);
        let dy=super::image1::mul16(velocity,c.rd32(this.wrapping_add(0xB627C).wrapping_add(angle.wrapping_mul(4))) as i32);
        c.wr32(sp.wrapping_add(4),c.rd32(sp.wrapping_add(4)).wrapping_add(dx as u32)); c.wr32(sp,c.rd32(sp).wrapping_add(dy as u32));
        let bound=if c.rd32(record.wrapping_add(0xA4))==0 {0x045DA8} else {0x045AB8};
        m.call_guest(c,crate::le::rt(bound),&[this,obj,sp.wrapping_add(4),sp])?;
    }
    if c.rd32(record.wrapping_add(0x60))!=0 || c.rd32(record.wrapping_add(0xA4))!=0 {
        c.eip=crate::le::rt(0x047E2A);
        while c.eip!=crate::le::rt(0x0483BC) {c.step(m)?;}
    } else {
        let old_z=c.rd32(zpos) as i32;
        m.call_guest(c,crate::le::rt(0x041B30),&[this,obj,c.rd32(sp.wrapping_add(4)),c.rd32(sp)])?;
        c.wr32(zpos,c.rd32(zpos).wrapping_add(vertical));
        let support=c.rd32(this.wrapping_add(0x144504));
        if support!=0 && c.rd32(this.wrapping_add(0x1423F0))==0 && !airborne {
            let ride_z=record.wrapping_add(0x80);
            c.wr32(ride_z,if c.rd32(record.wrapping_add(0x7C))==0 {c.rd32(zpos).wrapping_add(0x800)} else {c.rd32(ride_z).wrapping_add(0x800)});
            let top=object_field(c,this,support,0x66CE8).wrapping_add(object_field(c,this,support,0x6C5B8));
            if (c.rd32(ride_z) as i32)>top {c.wr32(ride_z,top as u32);}
            c.wr32(record.wrapping_add(0x7C),support); c.wr32(zpos,c.rd32(ride_z));
        } else {c.wr32(record.wrapping_add(0x7C),0);}
        if old_z.wrapping_sub(0x4000) > c.rd32(zpos) as i32 {
            let fall=0x4000i64*(c.rd32(this.wrapping_add(0x144B6C)) as i32 as i64)/2;
            let fall=i32::try_from(fall).map_err(|_| Fault::DivideError)?;
            c.wr32(zpos,old_z.wrapping_sub(fall) as u32);
        }
        if (c.rd32(zpos) as i32)<old_z {
            move_collision_level(c,this,c.rd32(zpos) as i32,false);
            c.wr32(xpos,c.rd32(sp.wrapping_add(4))); c.wr32(ypos,c.rd32(sp));
            m.call_guest(c,crate::le::rt(0x045768),&[this,obj])?;
            c.wr32(sp.wrapping_add(4),c.rd32(xpos)); c.wr32(sp,c.rd32(ypos));
        }
        c.wr32(xpos,c.rd32(sp.wrapping_add(4))); c.wr32(ypos,c.rd32(sp));
        c.wr32(this.wrapping_add(0x1423F0),0); c.wr32(this.wrapping_add(0x130D18),0);
    }
    c.r[ESP]=c.r[ESP].wrapping_add(0x44); for reg in [EBP,EDI,ESI] {c.r[reg]=c.pop32();} c.ret_near(0); Ok(())
}

fn move_collision_level(c: &mut Cpu,this: u32,z: i32,inertia: bool)->i32 {
    let level=(z.wrapping_add(0x4800)>>16).min(c.rd32(this.wrapping_add(0x5E7A4)) as i32);
    c.wr32(this.wrapping_add(0xCA098),this.wrapping_add(if inertia {0xC6098} else {0xC8098}).wrapping_add((level as u32).wrapping_shl(11)));
    c.wr32(this.wrapping_add(0xC6094),this.wrapping_add(0xC5094).wrapping_add((level as u32).wrapping_shl(10))); level
}
fn snap_path_axis(x: i32)->i32 {
    let q=x/0x4000; let tile=(q>>2).wrapping_shl(16);
    match q&3 {0=>tile.wrapping_add(0x4000),3=>tile.wrapping_add(0xBFFF),_=>x}
}

fn object_field(c: &Cpu, this: u32, obj: u32, off: u32) -> i32 {
    c.rd32(this.wrapping_add(obj.wrapping_mul(4)).wrapping_add(off)) as i32
}

fn actor(c: &Cpu, this: u32, id: u32) -> u32 {
    c.rd32(this.wrapping_add(0x13141C).wrapping_add(id.wrapping_mul(4)))
}

fn actor_definition(c: &Cpu, this: u32, record: u32) -> u32 {
    let kind = (c.rd32(record.wrapping_add(2)) as i32) >> 16;
    c.rd32(this.wrapping_add(0x13181C).wrapping_add((kind as u32).wrapping_mul(4)))
}

/// Preserve original priority/nearest/facing selection and call RaySight with
/// its exact scratch-state writes and visibility result.
fn find_target(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, source, mut range) = (c.r[EAX], c.r[EDX], c.r[EBX] as i32);
    let record = actor(c, this, source);
    let obj = c.rd16(record) as i16 as i32 as u32;
    let def = actor_definition(c, this, record);
    let z = object_field(c, this, obj, 0x78D8C)
        .wrapping_add(object_field(c, this, obj, 0x66CE8))
        .wrapping_add(object_field(c, this, obj, 0x6C5B8));
    let mut selected = u32::MAX;
    let mut priority = 1;
    let mut id = 0i32;
    while id < c.rd32(this.wrapping_add(0x130D24)) as i32 {
        if id as u32 != source {
            let target = actor(c, this, id as u32);
            let state = (c.rd32(target.wrapping_add(0x1A)) as i32) >> 16;
            if (c.rd32(target.wrapping_add(0x34)) as i32) >> 16 != 2
                && state != 9 && state != 10 && c.rd32(target.wrapping_add(0x5C)) == 0 {
                let other = c.rd16(target) as i16 as i32 as u32;
                let faction = if id as u32 == c.rd32(this.wrapping_add(0x130D08)) { 0 }
                    else { c.rd8(actor_definition(c, this, target).wrapping_add(0x3B)) as u32 };
                let rank = c.rd8(def.wrapping_add(faction).wrapping_add(0x3C)) as i8 as i32;
                if rank >= priority {
                    let (x, y) = (object_field(c, this, obj, 0x61418), object_field(c, this, obj, 0x64080));
                    let (tx, ty) = (object_field(c, this, other, 0x61418), object_field(c, this, other, 0x64080));
                    let dist = super::image1::distance(x, y, tx, ty);
                    if dist < range {
                        let angle = super::image1::atan64k(m, c, x, y, tx, ty)? as u16;
                        let heading = c.rd16(this.wrapping_add(0x7E65C).wrapping_add(obj.wrapping_mul(2)));
                        if (angle.wrapping_sub(heading) as i16 as i32).abs() < 0x4000 {
                            let tz = object_field(c, this, other, 0x78D8C).wrapping_add(
                                if other == c.rd32(this.wrapping_add(0x130D04)) { c.rd32(this.wrapping_add(0x130D1C)) as i32 }
                                else { object_field(c, this, other, 0x66CE8).wrapping_add(object_field(c, this, other, 0x6C5B8)) });
                            if m.call_guest(c, crate::le::rt(0x05D6D4), &[this, obj, other, dist as u32, z as u32, tz as u32, angle as u32])? == 1 {
                                priority = rank; range = dist; selected = id as u32;
                            }
                        }
                    }
                }
            }
        }
        id = id.wrapping_add(1);
    }
    c.wr32(actor(c, this, source).wrapping_add(8), selected);
    c.wr32(actor(c, this, source).wrapping_add(0xC), range as u32);
    c.ret_near(0);
    Ok(())
}

/// Tile floor below the object's head; level zero terminates even on empty tiles.
fn tile_floor(c: &Cpu, this: u32, x: i32, y: i32, head: i32) -> (i32, u8, u32) {
    let tile = (x >> 16).wrapping_add((y >> 16).wrapping_shl(5)) as u32;
    let floors = c.rd32(this.wrapping_add(0xC408C));
    let mut level = head >> 16;
    while level != 0 && c.rd8(floors.wrapping_add((level as u32).wrapping_shl(10)).wrapping_add(tile)) == 0 {
        level = level.wrapping_sub(1);
    }
    (level.wrapping_shl(16), c.rd8(floors.wrapping_add((level as u32).wrapping_shl(10)).wrapping_add(tile)), tile)
}

fn flat_contains(c: &Cpu, f: u32, x: i32, y: i32) -> bool {
    let fx = c.rd32(f.wrapping_add(0x61418)) as i32;
    let fy = c.rd32(f.wrapping_add(0x64080)) as i32;
    let rx = c.rd32(f.wrapping_add(0x69950)) as i32;
    let ry = c.rd32(f.wrapping_add(0x6C5B8)) as i32;
    x >= fx.wrapping_sub(rx) && x <= fx.wrapping_add(rx)
        && y >= fy.wrapping_sub(ry) && y <= fy.wrapping_add(ry)
}

fn coll_flat(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, obj, x, y) = (c.r[EAX], c.r[EDX], c.r[EBX] as i32, c.r[ECX] as i32);
    let z = this.wrapping_add(obj.wrapping_mul(4)).wrapping_add(0x78D8C);
    let head = (c.rd32(z) as i32).wrapping_add(0x4800);
    let (floor_z, _, mut tile) = tile_floor(c, this, x, y, head);
    c.wr32(z, floor_z as u32);
    while tile != 0 {
        let chain = c.rd32(this.wrapping_add(0xE33E4));
        let node = chain.wrapping_add(tile.wrapping_mul(4));
        let id = c.rd16(node.wrapping_add(2)) as u32;
        let f = this.wrapping_add(id.wrapping_mul(4));
        let fz = c.rd32(f.wrapping_add(0x66CE8)) as i32;
        if id != 0 && c.rd8(f.wrapping_add(0x5E7B2)) & 4 != 0
            && flat_contains(c, f, x, y) && fz > c.rd32(z) as i32 && fz < head {
            c.wr32(z, fz as u32);
        }
        tile = c.rd16(c.rd32(this.wrapping_add(0xE33E4)).wrapping_add(tile.wrapping_mul(4))) as u32;
    }
    c.ret_near(0);
    Ok(())
}

fn coll_top_flat(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, obj, old_z, height) = (c.r[EAX], c.r[EDX], c.r[EBX] as i32, c.r[ECX] as i32);
    let p = this.wrapping_add(obj.wrapping_mul(4));
    let body = if obj == c.rd32(this.wrapping_add(0x130D04)) { c.rd32(this.wrapping_add(0x130D1C)) }
        else { c.rd32(p.wrapping_add(0x6C5B8)).wrapping_mul(2) };
    let height = height.wrapping_add(body as i32).max(0x5800);
    let x = c.rd32(p.wrapping_add(0x61418)) as i32;
    let y = c.rd32(p.wrapping_add(0x64080)) as i32;
    let mut tile = (x >> 16).wrapping_add((y >> 16).wrapping_shl(5)) as u32;
    let mut level = old_z >> 16;
    let ceils = c.rd32(this.wrapping_add(0xC3088));
    while level < c.rd32(this.wrapping_add(0x5E7A4)) as i32
        && c.rd8(ceils.wrapping_add((level as u32).wrapping_shl(10)).wrapping_add(tile)) == 0 {
        level = level.wrapping_add(1);
    }
    let ceiling = level.wrapping_shl(16).wrapping_add(0x10000);
    let z = p.wrapping_add(0x78D8C);
    let mut hit = false;
    while tile != 0 {
        let node = c.rd32(this.wrapping_add(0xE33E4)).wrapping_add(tile.wrapping_mul(4));
        let id = c.rd16(node.wrapping_add(2)) as u32;
        let f = this.wrapping_add(id.wrapping_mul(4));
        let fz = c.rd32(f.wrapping_add(0x66CE8)) as i32;
        if id != 0 && c.rd8(f.wrapping_add(0x5E7B2)) & 0x80 != 0
            && flat_contains(c, f, x, y) && old_z < fz && fz.wrapping_sub(height) <= c.rd32(z) as i32 {
            c.wr32(z, fz.wrapping_sub(height).wrapping_sub(1) as u32);
            hit = true;
        }
        tile = c.rd16(c.rd32(this.wrapping_add(0xE33E4)).wrapping_add(tile.wrapping_mul(4))) as u32;
    }
    if old_z < ceiling && ceiling.wrapping_sub(height) <= c.rd32(z) as i32 {
        c.wr32(z, ceiling.wrapping_sub(height).wrapping_sub(1) as u32);
        hit = true;
    }
    c.r[EAX] = u32::from(hit);
    c.ret_near(0);
    Ok(())
}

fn swim_flat(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, obj, panel, art) = (c.r[EAX], c.r[EDX], c.r[EBX] as i32, c.r[ECX] as i32);
    let p = this.wrapping_add(obj.wrapping_mul(4));
    let x = c.rd32(p.wrapping_add(0x61418)) as i32;
    let y = c.rd32(p.wrapping_add(0x64080)) as i32;
    let z = c.rd32(p.wrapping_add(0x78D8C)) as i32;
    let head = z.wrapping_add(0x4800);
    let (mut floor_z, tile_panel, mut tile) = tile_floor(c, this, x, y, head);
    let mut water = tile_panel as i32 == panel;
    let mut flat = 0;
    let chain = c.rd32(this.wrapping_add(0xE33E4));
    while tile != 0 {
        let node = chain.wrapping_add(tile.wrapping_mul(4));
        let id = c.rd16(node.wrapping_add(2)) as u32;
        if id != 0 {
            let f = this.wrapping_add(id.wrapping_mul(4));
            let fz = c.rd32(f.wrapping_add(0x66CE8)) as i32;
            if c.rd8(f.wrapping_add(0x5E7B2)) & 4 != 0
                && flat_contains(c, f, x, y) && fz >= floor_z && fz < head {
                floor_z = fz;
                flat = id;
            }
        }
        tile = c.rd16(node) as u32;
    }
    if floor_z.wrapping_add(0x1000) < z { water = false; }
    else if flat != 0 {
        water = (c.rd32(this.wrapping_add(0x81DDC).wrapping_add(flat.wrapping_mul(2))) as i32) >> 16 == art;
    }
    c.r[EAX] = u32::from(water);
    c.ret_near(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actor_animation_loops_and_knockback_preserve_frame_and_heading_state() {
        let path = std::path::PathBuf::from("__animation_no_files__");
        let mut m = Machine::new(crate::vfs::Vfs::new(path.clone(), path));
        let mut c = Cpu::new();
        let this = 0x100000;
        let record = 0x310000;
        c.r[ESP] = 0x30000;
        c.write_bytes(crate::machine::STUB_BASE + 0x800, &[0x0F, 0xFF, 0, 1]);
        for link in [0x0581DC,0x0584C0] { c.write_bytes(crate::le::rt(link), &[0xC3]); }
        // Movement stub records its horizontal/vertical inputs, without moving.
        c.write_bytes(crate::le::rt(0x047B90), &[0x89,0x1D,0,0,0x38,0,0x89,0x0D,4,0,0x38,0,0xC3]);
        c.wr32(this + 0x131420, record); c.wr16(record, 10);
        c.wr32(this + 0x13181C, 0x320000);
        c.wr32(this + 0x131B4C, 0x330000);
        c.wr32(this + 0x131B8C, 0x340000);
        c.wr8(0x34000E, 2); c.wr8(0x34002E, 3); c.wr8(0x34002F, 0xFF);
        for (off,value) in [(0x7C,0x1000),(0xCC,0x1800),(0x11C,0x2000),(0x16C,0x2800),(0x1BC,9),(0x20C,5)] {
            c.wr32(0x340000 + off + 3*4, value);
        }
        c.wr8(0x34025F, 5); c.wr16(0x330016, 42); c.wr8(0x3300B9, 7);
        c.wr16(record + 0x16, 1);
        let check = |m: &mut Machine, c: &mut Cpu, tick| {
            c.r[EAX] = this; c.r[EDX] = 1; c.r[EBX] = tick; c.r[ECX] = 1;
            c.push32(0x1234); move_object(m,c).unwrap();
            assert_eq!((c.eip,c.r[ESP]),(0x1234,0x30000));
        };
        check(&mut m,&mut c,1);
        assert_eq!(c.rd32(this + 0x69950 + 40),0x1000);
        assert_eq!(c.rd16(this + 0x81DDE + 20),42);
        assert_eq!(c.rd8(this + 0x83F2C + 10),7);
        assert_eq!((c.rd8(record+0xBD),c.rd16(record+0x18)),(2,0));
        assert_eq!((c.rd32(0x380000),c.rd32(0x380004)),(9,5*64));
        check(&mut m,&mut c,1);
        assert_eq!((c.rd8(record+0xBD),c.rd16(record+0x18)),(2,1));
        c.wr16(this + 0x7E65C + 20,0x4567); c.wr16(record + 0xB4,0x1234);
        c.wr32(crate::le::rt(0x1E3944),1);
        for (off,value) in [(0xA4,2),(0xA8,0x8000),(0xAC,0x600),(0xB0,0x1000)] {c.wr32(record+off,value);}
        check(&mut m,&mut c,0);
        assert_eq!(c.rd32(0x380000),256);
        assert_eq!(c.rd16(this + 0x7E65C + 20),0x4567);
        assert_eq!((c.rd32(record+0xA4),c.rd32(record+0xA8),c.rd32(record+0xAC),c.rd32(record+0xB0)),(1,0x6666,0x400,0x1400));
        check(&mut m,&mut c,0);
        assert_eq!((c.rd32(record+0xA4),c.rd32(record+0xAC),c.rd32(record+0xB0)),(u32::MAX,0x1000,0x400));
        check(&mut m,&mut c,0);
        assert_eq!((c.rd32(record+0xA4),c.rd32(record+0xB0)),(0,0));
        for (from,to,step,expected) in [(0xFFFF,1,1,0),(1,0xFFFF,1,0),(0,0x8000,4,4),(0x8000,0,4,0x7FFC),(10,12,20,12),(0x12340007,0xABCD0007,20,0x12340007)] {
            c.r[EDX]=from; c.r[EBX]=to; c.r[ECX]=step; c.push32(0x1234);
            turn(&mut m,&mut c).unwrap(); assert_eq!(c.r[EAX],expected);
        }
    }
    #[test]
    fn targeting_keeps_priority_nearest_facing_and_living_actor_filters() {
        let path = std::path::PathBuf::from("__targeting_no_files__");
        let mut m = Machine::new(crate::vfs::Vfs::new(path.clone(), path));
        let mut c = Cpu::new();
        let this = 0x100000;
        c.r[ESP] = 0x30000;
        c.write_bytes(crate::machine::STUB_BASE + 0x800, &[0x0F, 0xFF, 0, 1]);
        c.write_bytes(crate::le::rt(0x05D6D4), &[0xB8, 1, 0, 0, 0, 0xC2, 12, 0]);
        c.wr32(this + 0x130D24, 3);
        c.wr32(this + 0x130D04, 10);
        for i in 0..3 {
            c.wr32(this + 0x13141C + 4*i, 0x310000 + i*256);
            c.wr16(0x310000 + i*256, 10+i as u16);
            c.wr32(this + 0x61418 + (10+i)*4, if i == 0 {0x20000} else if i == 1 {0x10000} else {0x18000});
        }
        c.wr32(this + 0x13181C, 0x320000);
        c.wr8(0x32003B, 1);
        c.wr8(0x32003C, 1); c.wr8(0x32003D, 3);
        let check = |m: &mut Machine, c: &mut Cpu| {
            c.r[EAX] = this; c.r[EDX] = 1; c.r[EBX] = 0x30000; c.r[ECX] = 0xAA55;
            c.push32(0x1234); find_target(m, c).unwrap();
            assert_eq!((c.eip, c.r[ESP], c.r[ECX]), (0x1234, 0x30000, 0xAA55));
            (c.rd32(0x310108), c.rd32(0x31010C))
        };
        assert_eq!(check(&mut m, &mut c), (2, 0x8000));
        c.wr32(0x310200 + 0x5C, 1); // invisibility excludes the closer actor
        assert_eq!(check(&mut m, &mut c), (0, 0x10000));
        c.wr16(this + 0x7E65C + 11*2, 0x8000);
        assert_eq!(check(&mut m, &mut c), (u32::MAX, 0x30000));
        c.wr16(this + 0x7E65C + 11*2, 0);
        c.wr16(0x31001C, 9); // dead player state
        assert_eq!(check(&mut m, &mut c), (u32::MAX, 0x30000));
        for (range, expected) in [(0x30000,u32::MAX),(0,2),(u32::MAX,2)] {
            c.r[EAX]=this; c.r[EDX]=1; c.r[EBX]=2; c.r[ECX]=range; c.push32(0x1234);
            sight_person(&mut m,&mut c).unwrap();
            assert_eq!(c.rd32(0x310108),expected); // non-positive range can see invisible actors
        }
        c.r[EAX]=this; c.r[EDX]=11; c.r[EBX]=12; c.r[ECX]=1; c.push32(0x1234);
        track_target(&mut m,&mut c).unwrap();
        assert_eq!(c.rd16(this + 0x7FC90 + 22),0x1555);
        assert_eq!(c.rd8(this + 0x812C4 + 11),2);
        c.wr16(0x31001C, 0);
        c.wr8(0x32003C, 0xFF); // signed negative faction priority
        assert_eq!(check(&mut m, &mut c), (u32::MAX, 0x30000));
    }
    #[test]
    fn object_scan_skips_inactive_slots_tracks_added_actors_and_restores_guest_frame() {
        let path = std::path::PathBuf::from("__scan_no_files__");
        let mut m = Machine::new(crate::vfs::Vfs::new(path.clone(),path));
        let mut c = Cpu::new(); let this = 0x100000;
        // Header stub initializes UpdateObjects' actual stack locals.
        let header = crate::le::rt(0x0348D0);
        c.write_bytes(header,&[0xC7,0x44,0x24,4,1,0,0,0,0x8D,0x47,4,0x89,0x44,0x24,0x6C,0xE9,0,0,0,0]);
        c.wr32(header+16,crate::le::rt(0x0349A5).wrapping_sub(header+20));
        // Active actor stub: record id; actor 2 appends two more actor slots.
        let body = crate::le::rt(0x0349A5);
        c.write_bytes(body,&[0x8B,0x44,0x24,4,0x89,4,0x85,0,0,0x36,0,0xFF,5,0x20,0,0x36,0,
            0x83,0xF8,2,0x75,10,0xC7,5,0x24,0x0D,0x23,0,5,0,0,0,0xE9,0,0,0,0]);
        c.wr32(body+33,crate::le::rt(0x0349E4).wrapping_sub(body+37));
        c.wr32(this+0x130D24,3);
        for id in 1..5 {
            let record=0x310000+id*256;
            c.wr32(this+0x13141C+id*4,record); c.wr16(record,id as u16);
            c.wr32(record+0x58,1); // exercise the exceptional guest-body path
            c.wr8(this+0x7DB42+id,u8::from(id%2==0));
        }
        for (reg,value) in [(ESP,0x30000),(EAX,this),(EDX,7),(EBX,0xAA55),(ECX,0xBB66),(ESI,0xCC77),(EDI,0xDD88),(EBP,0xEE99)] {c.r[reg]=value;}
        c.push32(0x1234); update_objects(&mut m,&mut c).unwrap();
        assert_eq!((c.r[ESP],c.eip),(0x30000,0x1234));
        assert_eq!((c.r[EBX],c.r[ECX],c.r[ESI],c.r[EDI],c.r[EBP]),(0xAA55,0xBB66,0xCC77,0xDD88,0xEE99));
        assert_eq!(c.rd32(0x360020),2);
        for id in 1..5 {assert_eq!(c.rd32(0x360000+id*4),if id%2==0 {id} else {0});}
    }
    #[test]
    fn common_actor_preparation_keeps_tick_classes_and_falls_back_before_sound_transitions() {
        let path=std::path::PathBuf::from("__prepare_no_files__");
        let mut m=Machine::new(crate::vfs::Vfs::new(path.clone(),path)); let mut c=Cpu::new();
        let (this,sp,record,def)=(0x100000,0x30000,0x310000,0x320000);
        c.r[ESP]=sp; c.wr32(this+0x13181C,def); c.wr32(sp+0x5C,1);
        c.write_bytes(crate::machine::STUB_BASE+0x800,&[0x0F,0xFF,0,1]);
        c.write_bytes(crate::le::rt(0x041C4C),&[0x31,0xC0,0xC3]);
        c.wr32(crate::le::rt(0x1E3924),1);
        c.wr32(crate::le::rt(0x92EE0),111); c.wr32(crate::le::rt(0x92F10),0xABCDEF);
        for (kind,tick) in [(9,1),(17,1),(10,0)] {
            c.wr8(def+0x3A,kind); assert!(prepare_actor(&mut m,&mut c,this,sp,record,10).unwrap());
            assert_eq!(c.rd32(sp),tick); assert_eq!(c.rd32(sp+8),111);
            assert_eq!(c.rd32(sp+0x34),0xABCDEF); assert_eq!(c.r[EAX],0xABCD00|kind as u32);
            assert_eq!((c.r[ESI],c.r[EBP],c.r[ECX],c.r[EDX]),(record,def,0,0));
            assert_eq!(c.rd8(this+0x83412+10),8);
            assert_eq!(c.rd8(this+0x81234+10),0); // nearby target-mode array is not the rotation-count table
        }
        for off in [0x5C,0x74,0x58,0xB6] {
            c.wr32(record+off,1); assert!(!prepare_actor(&mut m,&mut c,this,sp,record,10).unwrap()); c.wr32(record+off,0);
        }
        c.wr32(sp+0x54,1); assert!(!prepare_actor(&mut m,&mut c,this,sp,record,10).unwrap());
    }
    #[test]
    fn ordinary_monster_motion_clamps_speed_follows_floors_and_keeps_boundary_corrections() {
        let path=std::path::PathBuf::from("__monster_no_files__"); let mut m=Machine::new(crate::vfs::Vfs::new(path.clone(),path)); let mut c=Cpu::new();
        let (this,obj,record,def)=(0x100000u32,2u32,0x310000u32,0x320000u32);
        c.write_bytes(crate::machine::STUB_BASE+0x800,&[0x0F,0xFF,0,1]);
        m.hook_sig(&mut c,0x044EF4,"test path",3,true,|_,c| {c.r[EAX]=0;c.ret_near(0);Ok(())});
        m.hook_sig(&mut c,0x045DA8,"test bound",4,false,|_,c| {c.ret_near(0);Ok(())});
        m.hook_sig(&mut c,0x041B30,"test floor",4,false,|_,c| {c.wr32(c.r[EAX]+0x78D8C+c.r[EDX]*4,0);c.ret_near(0);Ok(())});
        m.hook_sig(&mut c,0x045768,"test clear",2,false,|_,c| {let p=c.r[EAX]+0x61418+c.r[EDX]*4;c.wr32(p,c.rd32(p)+3);c.ret_near(0);Ok(())});
        c.wr16(this+0x7B9F4+obj*2,1); c.wr32(this+0x13141C+4,record); c.wr32(this+0x13181C,def);
        c.wr32(this+0xB827C,0x10000); c.wr32(this+0x5E7A4,3); c.wr32(this+0x144B6C,1);
        for (speed,x) in [(512,0x20003),(-512,0x10003),(0,0x18003)] {
            for (off,value) in [(0x61418,0x18000),(0x64080,0x28000),(0x78D8C,0x10000)] {c.wr32(this+off+obj*4,value);}
            c.r[ESP]=0x30000; c.r[EAX]=this; c.r[EDX]=obj; c.r[EBX]=speed as u32; c.r[ECX]=0;
            c.r[ESI]=11; c.r[EDI]=22; c.r[EBP]=33; c.push32(0x1234); move_monster(&mut m,&mut c).unwrap();
            assert_eq!((c.eip,c.r[ESP],c.r[ESI],c.r[EDI],c.r[EBP]),(0x1234,0x30000,11,22,33));
            assert_eq!((c.rd32(this+0x61418+obj*4),c.rd32(this+0x64080+obj*4),c.rd32(this+0x78D8C+obj*4)),(x,0x28000,0xE000));
            assert_eq!(c.rd32(this+0xCA098),this+0xC8898); assert_eq!(c.rd32(this+0x1423F0),0);
        }
        for (x,y) in [(0,0x4000),(0x10000,0x14000),(0x1FFFF,0x1BFFF),(-1,0x4000),(-0x4000,-0x4001),(-0x8000,-0x8000)] {assert_eq!(snap_path_axis(x),y);}
    }
    #[test]
    fn path_quadrants_apertures_and_player_effect_timers_match_dos() {
        let path = std::path::PathBuf::from("__path_no_files__");
        let mut m = Machine::new(crate::vfs::Vfs::new(path.clone(),path));
        let mut c = Cpu::new(); let this = 0x100000; c.r[ESP]=0x30000;
        c.wr32(this + 0xCA098,0x310000); c.wr32(this + 0xC6094,0x320000);
        c.wr16(0x310000 + 33*2,1 << 5); c.wr8(0x320000 + 33,7);
        for (off,value) in [(0x141D90,0x14000),(0x141F10,0x18000),(0x142090,0x14000),(0x142210,0x18000)] {c.wr32(this+off+7*4,value);}
        let check = |m: &mut Machine,c: &mut Cpu,x,y| {
            c.r[EAX]=this; c.r[EDX]=x; c.r[EBX]=y; c.r[ECX]=0xAA55;
            c.push32(0x1234); path_check(m,c).unwrap(); assert_eq!(c.r[ECX],0xAA55); c.r[EAX]
        };
        assert_eq!(check(&mut m,&mut c,0x14000,0x14000),1);
        assert_eq!(check(&mut m,&mut c,0x10000,0x14000),0);
        c.wr8(this + 0x142390 + 7,1);
        assert_eq!(check(&mut m,&mut c,0x14000,0x14000),0);
        c.wr32(this + 0x1423F0,1);
        assert_eq!(check(&mut m,&mut c,0x14000,0x14000),1);
        c.wr32(this + 0x13141C,0x330000); c.wr32(this + 0x130D04,10);
        c.wr32(0x33005C,1); c.wr32(0x330074,3);
        for tick in [0,1] {
            c.r[EAX]=this; c.r[EDX]=tick; c.push32(0x1234);
            update_pc_object(&mut m,&mut c).unwrap();
            assert_eq!(c.rd8(this + 0x5E7B2 + 40)&2,if tick==0 {2} else {0});
            assert_eq!(c.rd32(0x330074),3-tick);
            assert_eq!(c.rd32(0x33005C),1-tick);
        }
    }
    #[test]
    fn missile_filters_and_water_height_bounds_keep_guest_semantics() {
        let path = std::path::PathBuf::from("__simulation_no_files__");
        let mut m = Machine::new(crate::vfs::Vfs::new(path.clone(), path));
        let mut c = Cpu::new();
        let this = 0x100000;
        c.r[ESP] = 0x30000;
        c.wr32(this + 0x130D48, 6);
        for i in 0..6 { c.wr8(this + 0x7DB42 + i, 1); }
        c.wr8(this + 0x5E7B3 + 2*4, 1);
        c.wr8(this + 0x5E7B3 + 3*4, 2);
        c.wr8(this + 0x5E7B1 + 4*4, 4);
        c.r[EAX] = this; c.push32(0x1234);
        missile_list(&mut m, &mut c).unwrap();
        assert_eq!(c.rd32(this + 0x136224), 2);
        assert_eq!((c.rd32(this + 0x136228), c.rd32(this + 0x13622C)), (1, 5));
        assert_eq!((c.eip, c.r[ESP]), (0x1234, 0x30000));
        c.wr32(this + 0xC408C, 0x300000);
        c.wr32(this + 0xE33E4, 0x310000);
        c.wr32(this + 0x61418 + 4, 0x18000);
        c.wr32(this + 0x64080 + 4, 0x18000);
        c.wr8(0x300000 + 33, 7);
        let check = |c: &mut Cpu, m: &mut Machine, z: u32| {
            c.wr32(this + 0x78D8C + 4, z);
            c.r[EAX] = this; c.r[EDX] = 1; c.r[EBX] = 7; c.r[ECX] = 9;
            c.push32(0x1234); swim_flat(m, c).unwrap(); c.r[EAX]
        };
        assert_eq!(check(&mut c, &mut m, 0x1000), 1);
        assert_eq!(check(&mut c, &mut m, 0x1001), 0);
        // A raised flat overrides the tile's panel only inside its inclusive bounds.
        c.wr16(0x310000 + 33*4 + 2, 2);
        for (off, value) in [(0x61418,0x18000),(0x64080,0x18000),(0x69950,0x8000),(0x6C5B8,0x8000),(0x66CE8,0x8000)] {
            c.wr32(this + off + 8, value);
        }
        c.wr8(this + 0x5E7B2 + 8, 4);
        c.wr16(this + 0x81DDE + 4, 9);
        assert_eq!(check(&mut c, &mut m, 0x9000), 1);
        assert_eq!(check(&mut c, &mut m, 0x9001), 0);
        c.wr16(this + 0x81DDE + 4, 8);
        assert_eq!(check(&mut c, &mut m, 0x9000), 0);
        c.wr32(this + 0x78D8C + 4, 0x9000);
        c.r[EAX] = this; c.r[EDX] = 1; c.r[EBX] = 0x18000; c.r[ECX] = 0x18000;
        c.push32(0x1234); coll_flat(&mut m, &mut c).unwrap();
        assert_eq!(c.rd32(this + 0x78D8C + 4), 0x8000);
        c.wr32(this + 0xC3088, 0x320000);
        c.wr32(this + 0x5E7A4, 0);
        for current in [0xA7FF, 0xA800] {
            c.wr32(this + 0x78D8C + 4, current);
            c.r[EAX] = this; c.r[EDX] = 1; c.r[EBX] = 0; c.r[ECX] = 0;
            c.push32(0x1234); coll_top_flat(&mut m, &mut c).unwrap();
            assert_eq!(c.r[EAX], u32::from(current == 0xA800));
            assert_eq!(c.rd32(this + 0x78D8C + 4), 0xA7FF);
        }
        assert_eq!((c.eip, c.r[ESP]), (0x1234, 0x30000));
    }
}
