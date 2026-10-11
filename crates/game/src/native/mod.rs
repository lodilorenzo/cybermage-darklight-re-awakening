//! Native Rust replacements for original game functions.
//!
//! Each hook replaces a guest function at its link address (labels.csv).
//! Arguments follow the Watcom register convention: EAX, EDX, EBX, ECX, then
//! stack; the native code must perform the return (`cpu.ret_near(pop)`).

pub mod cmdraw;
pub mod image1;
pub mod misc;
pub mod sound;
pub mod simulation;

use crate::machine::Machine;
use x86::Cpu;

pub fn install(m: &mut Machine, cpu: &mut Cpu) {
    sound::install(m, cpu);
    image1::install(m, cpu);
    misc::install(m, cpu);
    simulation::install(m, cpu);
    cmdraw::install(m, cpu);
    // A guest-function port (verifiable); it also shades the native world in pause menus.
    m.hook_sig(cpu, 0x04E2BC, "Xox::ShadeMenuBox", 4, false, crate::display::shade_menu_box);
    // Host presentation/input hooks are asynchronous, not guest-function ports.
    if m.verifier.is_none() {
        crate::display::install(m, cpu);
        assert_eq!(cpu.read_bytes(crate::le::rt(0x044122), 4), [0x8B, 0x44, 0x24, 0x40]);
        m.hook(cpu, 0x044122, "Xox::SmallStepOpening", small_step_opening);
    }
}

/// FixNxNy's masked door frame stops the player's centre at 1/4 tile from
/// its face, but the opening starts 1/16 tile beyond that stop. DOS-sized
/// movement steps bridge the gap; faster native frames (or slow sticks) cannot.
/// Extend only the opening's depth, never its width or a solid door panel.
fn small_step_opening(_m: &mut Machine, c: &mut Cpu) -> Result<(), x86::Fault> {
    use x86::{EAX, ESI, ESP};
    let (this, sp) = (c.r[ESI], c.r[ESP]);
    let obj = c.rd32(sp + 0x54);
    let flags = c.rd32(this + 0x5E7B0 + obj * 4);
    let x = c.rd32(c.rd32(sp + 0x4C)) as i32;
    let y = c.rd32(c.rd32(sp + 0x7C)) as i32;
    if !c.wlog_on && flags & 0x4820_0000 == 0x4800_0000
        && (0..32*65536).contains(&x) && (0..32*65536).contains(&y) {
        let tile = ((y >> 16) * 32 + (x >> 16)) as u32;
        let region = c.rd8(c.rd32(this + 0xC6094) + tile) as u32;
        if c.rd8(this + 0x142390 + region) != 0 {
            let angle = c.rd16(this + 0x7E65C + obj * 2);
            // Apertures are axis aligned; retain original collision for diagonals.
            if angle & 0x3FFF == 0 {
                let (px, py) = if angle & 0x4000 == 0 { (0x1000, 0) } else { (0, 0x1000) };
                let min_x = c.rd32(this + 0x141D90 + region * 4) as i32;
                let max_x = c.rd32(this + 0x141F10 + region * 4) as i32;
                let min_y = c.rd32(this + 0x142090 + region * 4) as i32;
                let max_y = c.rd32(this + 0x142210 + region * 4) as i32;
                if x >= min_x - px && x <= max_x + px && y >= min_y - py && y <= max_y + py {
                    c.eip = crate::le::rt(0x044A7C); // original cleanup, including the stack argument
                    return Ok(());
                }
            }
        }
    }
    c.r[EAX] = c.rd32(sp + 0x40); // replay the displaced MOV; keep the original collision code
    c.eip = crate::le::rt(0x044126);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::le::rt;
    use x86::*;

    #[test]
    fn small_steps_pass_open_frames_but_not_panels_jambs_or_closed_openings() {
        let path = std::env::temp_dir().join(format!("cybermage-opening-{}", std::process::id()));
        let mut m = Machine::new(crate::vfs::Vfs::new(path.join("iso"), path.join("run")));
        let mut c = Cpu::new();
        let (this, sp) = (0x100000, 0x30000);
        c.r[ESI] = this; c.r[ESP] = sp;
        c.wr32(sp + 0x40, 936); c.wr32(sp + 0x54, 313);
        c.wr32(sp + 0x4C, 0x1000); c.wr32(sp + 0x7C, 0x1004);
        c.wr32(this + 0xC6094, 0x200000);
        for tile in [25*32+10,26*32+10] { c.wr8(0x200000 + tile, 7); }
        for (offset, value) in [(0x141D90,0xA4000),(0x141F10,0xAC000),(0x142090,0x19B000),(0x142210,0x1A5000)] {
            c.wr32(this + offset + 7*4, value);
        }
        let flags = this + 0x5E7B0 + 313*4;
        let angle = this + 0x7E65C + 313*2;
        let active = this + 0x142390 + 7;
        // The reproduced doorway and its reverse direction; narrow jambs remain solid.
        for (x,y,f,a,on,logging,passes) in [
            (0xA8000,0x19A001,0x48080000,0x4000,1,false,true),
            (0xA8000,0x1A5FFF,0x48080000,0x4000,1,false,true),
            (0xA8000,0x199FFF,0x48080000,0x4000,1,false,false),
            (0xA3FFF,0x19A001,0x48080000,0x4000,1,false,false),
            (0xAC001,0x19A001,0x48080000,0x4000,1,false,false),
            (0xA8000,0x19A001,0x08000000,0x4000,1,false,false),
            (0xA8000,0x19A001,0x48280000,0x4000,1,false,false),
            (0xA8000,0x19A001,0x48080000,0x4000,0,false,false),
            (0xA8000,0x19A001,0x48080000,0x3000,1,false,false),
            (0xA8000,0x19A001,0x48080000,0x4000,1,true,false),
            (0xA3FFF,0x19C000,0x48080000,0,1,false,true),
            (0xA8000,0x19A001,0x48080000,0,1,false,false),
        ] {
            c.wr32(0x1000,x); c.wr32(0x1004,y);
            c.wr32(flags,f); c.wr16(angle,a); c.wr8(active,on);
            c.wlog_on = logging;
            let stack = c.read_bytes(sp,0x80).to_vec();
            small_step_opening(&mut m,&mut c).unwrap();
            assert_eq!(c.eip,rt(if passes { 0x044A7C } else { 0x044126 }));
            assert_eq!(c.r[ESP],sp);
            assert_eq!(c.read_bytes(sp,0x80),stack);
        }
        // The existing epilogue, not the hook, restores registers and pops its argument.
        c.wlog_on = false;
        c.write_bytes(rt(0x044A7C), &[0x83,0xC4,0x6C,0x5D,0x5F,0x5E,0xC2,4,0]);
        c.wr32(sp+0x6C,0x1111); c.wr32(sp+0x70,0x2222);
        c.wr32(sp+0x74,this); c.wr32(sp+0x78,0x123456);
        c.eip = rt(0x044A7C);
        for _ in 0..5 { c.step(&mut m).unwrap(); }
        assert_eq!(c.eip,0x123456); assert_eq!(c.r[ESP],sp+0x80);
        assert_eq!((c.r[EBP],c.r[EDI],c.r[ESI]),(0x1111,0x2222,this));
        std::fs::remove_dir_all(path).unwrap();
    }
}
