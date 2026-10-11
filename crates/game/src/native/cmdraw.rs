//! Native ports of cmdraw.cpp (HUD / overlay drawing on the 640-wide view buffer).

use crate::le::rt;
use crate::machine::Machine;
use x86::*;

const PITCH: u32 = 640;

pub fn install(m: &mut Machine, cpu: &mut Cpu) {
    m.hook_sig(cpu, 0x02A37C, "Xox::DrawMonitor", 4, false, h_draw_monitor);
    m.hook_sig(cpu, 0x02A624, "Xox::DrawHGFXBitMap", 4, false, h_draw_hgfx_bitmap);
    m.hook_sig(cpu, 0x029524, "Xox::DrawXScaleGlow", 4, false, h_draw_xscale_glow);
    m.hook_sig(cpu, 0x029260, "Xox::DrawXScaleAim", 4, false, h_draw_xscale_aim);
    m.hook_sig(cpu, 0x0675D4, "TEXT_STR", 1, false, h_text_str);
    m.hook_sig(cpu, 0x067589, "TEXT_LEN", 2, true, h_text_len);
}

/// Blit a `w` x `h` bitmap to `dst + y*640 + x`, skipping colour 0.
pub fn draw_monitor(c: &mut Cpu, dst: u32, src: u32, x: u32, y: u32, w: i32, h: i32) {
    let mut row = dst.wrapping_add(y.wrapping_mul(PITCH)).wrapping_add(x);
    let mut s = src;
    let mut rows = h;
    loop {
        for i in 0..w.max(0) as u32 {
            let b = c.rd8(s.wrapping_add(i));
            if b != 0 {
                c.wr8(row.wrapping_add(i), b);
            }
        }
        row = row.wrapping_add(PITCH);
        s = s.wrapping_add(w as u32);
        // original uses `dec; jne`, so h == 0 loops (practically) forever; keep h <= 0 safe
        rows -= 1;
        if rows <= 0 {
            break;
        }
    }
}

fn h_draw_monitor(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (w, h) = (c.stack_arg(0) as i32, c.stack_arg(1) as i32);
    draw_monitor(c, c.r[EAX], c.r[EDX], c.r[EBX], c.r[ECX], w, h);
    c.ret_near(8);
    Ok(())
}

/// Opaque HUD bitmap: signed 16-bit width, 16-bit row countdown, MOVSD then
/// byte tails. Keep direction flag and overlapping-copy behavior, not memmove.
fn h_draw_hgfx_bitmap(_m: &mut Machine,c: &mut Cpu)->Result<(),Fault> {
    let (mut row,mut src)=(c.r[EAX].wrapping_add(c.r[EBX].wrapping_mul(PITCH)).wrapping_add(c.r[EDX]),c.r[ECX]);
    let width=c.r[ESP].wrapping_add(4); let height=c.r[ESP].wrapping_add(8);
    let backwards=c.flags()&DF!=0;
    while c.rd16(height)!=0 {
        let w=c.rd16(width) as i16 as i32 as u32; let (mut s,mut d)=(src,row);
        for _ in 0..(w>>2) {c.wr32(d,c.rd32(s));s=s.wrapping_add(if backwards {u32::MAX-3} else {4});d=d.wrapping_add(if backwards {u32::MAX-3} else {4});}
        for _ in 0..(w&3) {c.wr8(d,c.rd8(s));s=s.wrapping_add(if backwards {u32::MAX} else {1});d=d.wrapping_add(if backwards {u32::MAX} else {1});}
        row=row.wrapping_add(PITCH); src=src.wrapping_add(c.rd16(width) as i16 as i32 as u32);
        let h=c.rd32(height); c.wr32(height,(h&0xFFFF0000)|(h as u16).wrapping_sub(1) as u32);
    }
    c.ret_near(8); Ok(())
}

const FONT_TABLE: u32 = 0x20732F;
const FONT_X: u32 = 0x20732B;
const FONT_Y: u32 = 0x20732D;

fn h_text_len(_m: &mut Machine,c: &mut Cpu)->Result<(),Fault> {
    c.set_flags(c.flags()&!DF);
    let font=rt(FONT_TABLE).wrapping_add((c.r[EDX] as u8 as u32)*32);
    let data=c.rd32(font); let spacing=c.rd16(font+0x1A) as i16 as i32 as u32;
    let mut text=c.r[EAX]; let mut len=0u32;
    while c.rd8(text)!=0 {
        let ch=c.rd8(text) as u32; text=text.wrapping_add(1);
        let glyph=data.wrapping_add(ch.wrapping_mul(c.rd16(font+0x1E) as u32)).wrapping_add(14);
        len=len.wrapping_add((c.rd16(glyph) as i16 as i32 as u32).wrapping_add(spacing));
    }
    c.r[EAX]=len; c.ret_near(0); Ok(())
}

/// Original bitmap-font modes: FF is outline/transparent, FE is background
/// colour/remap/transparent. String drawing uses SIGNED character/stride values;
/// TEXT_LEN deliberately does not. Cursor arithmetic remains 16-bit.
fn h_text_str(_m: &mut Machine,c: &mut Cpu)->Result<(),Fault> {
    c.set_flags(c.flags()&!DF);
    let font=rt(FONT_TABLE).wrapping_add((c.rd8(rt(0x20732A)) as u32)*32);
    let colors=[c.rd8(font+0xA),c.rd8(font+0xC),c.rd8(font+0xE)]; let remap=c.rd8(font+0x10)!=0;
    let map=c.rd32(font+4)&!0xFF; let height=c.rd16(font+0x14);
    let mut dst=c.rd32(rt(0x20746F)).wrapping_add((c.rd16(rt(FONT_Y)) as u32)*PITCH).wrapping_add(c.rd16(rt(FONT_X)) as u32);
    let mut text=c.r[EAX];
    while c.rd8(text)!=0 {
        let ch=c.rd8(text) as i8 as i32 as u32; text=text.wrapping_add(1);
        let glyph=c.rd32(font).wrapping_add(ch.wrapping_mul(c.rd16(font+0x1E) as i16 as i32 as u32)).wrapping_add(14);
        let width=c.rd16(glyph); let spacing=c.rd16(font+0x1A) as i16 as i32;
        c.wr16(rt(FONT_X),c.rd16(rt(FONT_X)).wrapping_add(width)); c.wr16(rt(FONT_X),c.rd16(rt(FONT_X)).wrapping_add(spacing as u16));
        let mode=c.rd8(font+8); let colored=c.rd16(font+0x16)!=0;
        let mut src=glyph.wrapping_add(2); let mut p=dst;
        let rows=if height==0 {65536} else {height as u32}; let cols=if width==0 {65536} else {width as u32};
        for _ in 0..rows {
            for _ in 0..cols {
                let t=c.rd8(src); src=src.wrapping_add(1);
                let color=match t {
                    0xFF=>if mode==0 || mode==2 {None} else {Some(colors[1])},
                    0xFE=>if mode>=2 {None} else if remap {Some(c.rd8(map|c.rd8(p) as u32))} else {Some(colors[2])},
                    _=>Some(if colored {t} else {colors[0]}),
                };
                if let Some(color)=color {c.wr8(p,color);} p=p.wrapping_add(1);
            }
            p=p.wrapping_add(PITCH.wrapping_sub(width as u32));
        }
        dst=dst.wrapping_add((width as i32).wrapping_add(spacing) as u32);
    }
    c.ret_near(0); Ok(())
}

const SPELL_GLOW: u32 = 0x092E58; // u8[.][4][3] glow colour remap
const GLV_Y_OFF: u32 = 0x094408; // i32[glove][18]
const GLV_X_POS_OFF: u32 = 0x094888;
const GLV_X_OFF: u32 = 0x093F88;
// Image1 fields used here
const TEXTURE_PTR: u32 = 0xA4A3C;
const TEX_W: u32 = 0xA623C;
const TEX_H: u32 = 0xA7A3C;
const VIEW_W: u32 = 0xE34B4;
const VIEW_H: u32 = 0xE34AC;
const FB_OFFSET: u32 = 0xE34A8;

#[inline]
fn rd(c: &Cpu, a: u32) -> i32 {
    c.rd32(a) as i32
}

fn h_draw_xscale_glow(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, glove, frame, tex_id) = (c.r[EAX], c.r[EDX] as i32, c.r[EBX] as i32, c.r[ECX] as i32);
    let (ox, oy, glow_a, glow_b) =
        (c.stack_arg(0) as i32, c.stack_arg(1) as i32, c.stack_arg(2) as i32, c.stack_arg(3) as i32);
    let glow = rt(SPELL_GLOW).wrapping_add((glow_a * 12 + glow_b * 3) as u32);
    draw_xscale(c, this, glove, frame, tex_id, ox, oy, Some(glow));
    c.ret_near(0x10);
    Ok(())
}

fn h_draw_xscale_aim(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    draw_xscale(c,c.r[EAX],c.r[EDX] as i32,c.r[EBX] as i32,c.r[ECX] as i32,c.stack_arg(0) as i32,c.stack_arg(1) as i32,None);
    c.ret_near(8); Ok(())
}

/// Packed hand/weapon rows. Glow optionally remaps 0xFD..0xFF; ordinary aim
/// writes them unchanged, including opaque black texels in positive runs.
fn draw_xscale(
    c: &mut Cpu,
    this: u32,
    glove: i32,
    frame: i32,
    tex_id: i32,
    ox: i32,
    oy: i32,
    glow: Option<u32>,
) {
    let t = |off: u32| rd(c, this.wrapping_add(off).wrapping_add((tex_id as u32).wrapping_mul(4)));
    let tex = t(TEXTURE_PTR) as u32;
    let th2 = t(TEX_H).wrapping_mul(2);
    let tw2 = t(TEX_W).wrapping_mul(2);
    let view_w = rd(c, this.wrapping_add(VIEW_W));
    let view_h = rd(c, this.wrapping_add(VIEW_H));
    let width = view_w.wrapping_mul(th2).wrapping_div(640) * 2; // screen pixels per row
    let rows_total = tw2.wrapping_mul(view_w).wrapping_div(640) * 2;
    let ustep = ((th2 as u32) << 16) / width as u32; // texture step along a row
    let vstep = ((tw2 as u32) << 16) / rows_total as u32; // texture column step per row
    let mut v: u32 = 0;
    let mut rows = rows_total;
    if rows > view_h {
        v = vstep.wrapping_mul((rows - view_h) as u32);
        rows = view_h;
    }
    let idx = (glove * 72 + frame * 4) as u32;
    let mut y_top = (rd(c, rt(GLV_Y_OFF).wrapping_add(idx)) * 2 + 480 - oy * 2).wrapping_mul(view_w).wrapping_div(640);
    if y_top < view_h - rows {
        y_top = view_h - rows;
    } else if y_top > view_h - 4 {
        y_top = view_h - 4;
    }
    let x0 = ox * 2 + 320 - (rd(c, rt(GLV_X_POS_OFF).wrapping_add(idx)) << 1);
    let x_left = ((rd(c, rt(GLV_X_OFF).wrapping_add(idx)) << 1) + x0).wrapping_mul(view_w).wrapping_div(640);
    let fb = this.wrapping_add(rd(c, this.wrapping_add(FB_OFFSET)) as u32);
    let mut row = fb.wrapping_add((y_top as u32).wrapping_mul(PITCH)).wrapping_add(x_left as u32);
    let n_rows = view_h - y_top;
    for _ in 0..n_rows.max(0) {
        let data = tex.wrapping_add(c.rd32(tex.wrapping_add((v >> 16).wrapping_mul(4))));
        // packed run decode along the row (same scheme as DrawPackWall)
        let mut pos: u32 = 0;
        let mut x = 0i32;
        let mut off = 0u32;
        while x < width {
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
            let mut n = ((rem - 1) as u32 / ustep) as i32 + 1;
            if x + n > width {
                n = width - x;
            }
            if opaque {
                let mut p = row.wrapping_add(x as u32);
                for _ in 0..n {
                    let mut px = c.rd8(data.wrapping_add(pos >> 16));
                    let write = if let Some(glow) = glow.filter(|_| px > 0xFC) {
                        px = c.rd8(glow.wrapping_add(0xFF - px as u32));
                        px != 0
                    } else {
                        true
                    };
                    if write {
                        c.wr8(p, px);
                    }
                    p = p.wrapping_add(1);
                    pos = pos.wrapping_add(ustep);
                }
                x += n;
            } else {
                pos = pos.wrapping_add(ustep.wrapping_mul(n as u32)).wrapping_sub(run as u32);
                x += n;
            }
        }
        row = row.wrapping_add(PITCH);
        v = v.wrapping_add(vstep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bitmap_font_modes_keep_transparency_palette_remaps_and_cursor_wrap() {
        let path=std::path::PathBuf::from("__font_no_files__"); let mut m=Machine::new(crate::vfs::Vfs::new(path.clone(),path)); let mut c=Cpu::new();
        let (font,data,text,fb,map)=(rt(FONT_TABLE),0x200000u32,0x210000u32,0x300000u32,0x220000u32);
        c.wr32(font,data); c.wr32(font+4,map+17); c.wr32(rt(0x20746F),fb); c.wr16(font+0x14,2); c.wr16(font+0x1A,0xFFFF); c.wr16(font+0x1E,8);
        for (off,col) in [(0xA,7),(0xC,8),(0xE,9)] {c.wr8(font+off,col);}
        for i in 0..256 {c.wr8(map+i,(i as u8).wrapping_add(1));}
        let glyph=data+14+65*8; c.wr16(glyph,3); c.write_bytes(glyph+2,&[10,0xFE,0xFF,0,0xFF,0xFE]); c.write_bytes(text,b"A\0");
        for mode in 0..4 {for colored in [false,true] {for remap in [false,true] {
            c.wr8(font+8,mode); c.wr16(font+0x16,u16::from(colored)); c.wr8(font+0x10,u8::from(remap)); c.wr16(rt(FONT_X),0xFFFE);
            let p=fb+0xFFFE; c.mem[p as usize..p as usize+643].fill(42);
            c.r[ESP]=0x50000; c.r[EAX]=text; c.set_flags(DF); c.push32(0x1234); h_text_str(&mut m,&mut c).unwrap();
            let fg=if colored {10} else {7}; let zero=if colored {0} else {7};
            let outline=if mode==0 || mode==2 {42} else {8}; let bg=if mode>=2 {42} else if remap {43} else {9};
            assert_eq!(c.read_bytes(p,3),&[fg,bg,outline]); assert_eq!(c.read_bytes(p+640,3),&[zero,outline,bg]);
            assert_eq!((c.rd16(rt(FONT_X)),c.r[EAX],c.r[ESP],c.eip,c.flags()&DF),(0,text,0x50000,0x1234,0));
        }}}
        c.write_bytes(text,&[0xFF,0]); c.wr16(data+14+255*8,0xFFFE);
        c.r[EAX]=text; c.r[EDX]=0; c.push32(0x1234); h_text_len(&mut m,&mut c).unwrap(); assert_eq!(c.r[EAX],(-3i32) as u32);
    }
    #[test]
    fn opaque_bitmap_keeps_dword_overlap_direction_flag_and_height_word() {
        let path=std::path::PathBuf::from("__bitmap_no_files__"); let mut m=Machine::new(crate::vfs::Vfs::new(path.clone(),path)); let mut c=Cpu::new();
        let run=|m: &mut Machine,c: &mut Cpu,src,dst,w,h,flags| {
            c.r[ESP]=0x50000; c.r[EAX]=dst; c.r[EDX]=0; c.r[EBX]=0; c.r[ECX]=src; c.set_flags(flags);
            c.push32(h); c.push32(w); c.push32(0x1234); let haddr=c.r[ESP]+8;
            h_draw_hgfx_bitmap(m,c).unwrap(); assert_eq!((c.r[ESP],c.eip),(0x50000,0x1234)); assert_eq!(c.rd32(haddr),h&0xFFFF0000);
        };
        c.write_bytes(0x20000,&[1,2,3,4,5,6,7,8]); run(&mut m,&mut c,0x20000,0x20001,7,0xABCD0001,0);
        assert_eq!(c.read_bytes(0x20000,8),&[1,1,2,3,4,4,4,4]);
        c.write_bytes(0x20000,&[1,2,3,4,5,6,7,8]); c.mem[0x2FFFE..0x30008].fill(42);
        run(&mut m,&mut c,0x20004,0x30004,7,0xABCD0001,DF);
        assert_eq!(c.read_bytes(0x2FFFE,10),&[0,0,1,42,42,42,5,6,7,8]); assert_ne!(c.flags()&DF,0);
        c.write_bytes(0x20000,&[0,2,3,4,5,6,7,8,9,10,11,12,13,14]);
        run(&mut m,&mut c,0x20000,0x30000,7,0xABCD0002,0);
        assert_eq!(c.read_bytes(0x30000,7),&[0,2,3,4,5,6,7]); assert_eq!(c.read_bytes(0x30280,7),&[8,9,10,11,12,13,14]);
        run(&mut m,&mut c,0x20000,0x30000,7,0xABCD0000,0); assert_eq!(c.read_bytes(0x30000,7),&[0,2,3,4,5,6,7]);
    }
    #[test]
    fn packed_aim_and_glow_preserve_black_pixels_transparent_runs_and_hud_tracking() {
        let mut c=Cpu::new(); let this=0x100000u32; let tex=0x200000u32; let fb=0x30000u32;
        for (off,value) in [(TEXTURE_PTR,tex),(TEX_W,1),(TEX_H,2),(VIEW_W,640),(VIEW_H,480),(FB_OFFSET,fb.wrapping_sub(this))] {c.wr32(this+off,value);}
        c.wr32(tex,8); c.wr32(tex+4,8); c.write_bytes(tex+8,&[3,0,0,0xFD,0xFF,0xFF,0xFF]);
        let glow=rt(SPELL_GLOW); c.wr8(glow,5);
        for (map,pixels,mask) in [(None,[0,0,0xFD,0xFD,0xFF,0xFF,42,42],[true,true,true,true,true,true,false,false]),
            (Some(glow),[0,0,42,42,5,5,42,42],[true,true,false,false,true,true,false,false])] {
            c.mem[fb as usize..fb as usize+640*480].fill(42); c.wlog.clear(); c.wlog_on=true;
            draw_xscale(&mut c,this,0,0,0,0,0,map); c.wlog_on=false;
            let mut hud=vec![false;640*480]; for &(a,_) in &c.wlog {hud[(a-fb) as usize]=true;}
            for y in 476..480 {let p=y*640+320; assert_eq!(c.mem[fb as usize+p..fb as usize+p+8],pixels); assert_eq!(hud[p..p+8],mask);}
        }
    }
}
