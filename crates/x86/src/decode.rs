//! Instruction decoder producing compact `Op` records that are cached per
//! code address, so each instruction is decoded only once.

use crate::*;

/// No register (for base/index fields).
pub const NOREG: u8 = 8;

pub const F_O16: u8 = 1;
pub const F_A16: u8 = 2;
pub const F_MEM: u8 = 4; // ModRM operand is memory
pub const F_REP: u8 = 8; // F3 prefix
pub const F_REPNE: u8 = 16; // F2 prefix
pub const F_SEGOVR: u8 = 32; // explicit segment override present
pub const F_VALID: u8 = 128;

#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Op {
    /// Handler: one-byte opcode, or 0x100 | second byte for 0F xx.
    pub h: u16,
    pub len: u8,
    pub fl: u8,
    /// Segment register index used for the memory operand / string source.
    pub seg: u8,
    /// ModRM reg field (or low opcode bits for register-encoded ops).
    pub reg: u8,
    /// ModRM rm register when mod == 3.
    pub rm: u8,
    pub base: u8,
    pub index: u8,
    pub scale: u8,
    pub _pad: [u8; 2],
    pub disp: u32,
    pub imm: u32,
    pub imm2: u32,
}

impl Op {
    #[inline(always)]
    pub fn o16(&self) -> bool {
        self.fl & F_O16 != 0
    }
    #[inline(always)]
    pub fn is_mem(&self) -> bool {
        self.fl & F_MEM != 0
    }
}

/// One-byte opcodes that take a ModRM byte.
const fn modrm_table() -> [bool; 256] {
    let mut t = [false; 256];
    let mut i = 0;
    while i < 0x40 {
        if i & 7 < 4 {
            t[i] = true;
        }
        i += 1;
    }
    let list = [
        0x62, 0x63, 0x69, 0x6B, 0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x8B, 0x8C, 0x8D,
        0x8E, 0x8F, 0xC0, 0xC1, 0xC4, 0xC5, 0xC6, 0xC7, 0xD0, 0xD1, 0xD2, 0xD3, 0xD8, 0xD9, 0xDA, 0xDB, 0xDC, 0xDD,
        0xDE, 0xDF, 0xF6, 0xF7, 0xFE, 0xFF,
    ];
    let mut k = 0;
    while k < list.len() {
        t[list[k]] = true;
        k += 1;
    }
    t
}
static MODRM1: [bool; 256] = modrm_table();

fn modrm0f(op: u8) -> bool {
    !matches!(op, 0x06 | 0x08 | 0x09 | 0x0B | 0x31 | 0x80..=0x8F | 0xA0 | 0xA1 | 0xA2 | 0xA8 | 0xA9 | 0xC8..=0xCF | 0xFF)
}

impl Cpu {
    /// Decode the instruction at linear address `a`.
    pub fn decode_at(&self, a: u32) -> Op {
        let mut p = a;
        let mut op = Op { base: NOREG, index: NOREG, seg: DS as u8, ..Default::default() };
        let mut sovr: Option<u8> = None;
        let b = loop {
            let b = self.rd8(p);
            p = p.wrapping_add(1);
            match b {
                0x26 => sovr = Some(ES as u8),
                0x2E => sovr = Some(CS as u8),
                0x36 => sovr = Some(SS as u8),
                0x3E => sovr = Some(DS as u8),
                0x64 => sovr = Some(FS as u8),
                0x65 => sovr = Some(GS as u8),
                0x66 => op.fl |= F_O16,
                0x67 => op.fl |= F_A16,
                0xF0 => {}
                0xF2 => op.fl = (op.fl & !F_REP) | F_REPNE,
                0xF3 => op.fl = (op.fl & !F_REPNE) | F_REP,
                _ => break b,
            }
            if p.wrapping_sub(a) > 14 {
                break 0x90;
            }
        };
        let vs: u32 = if op.fl & F_O16 != 0 { 2 } else { 4 };
        let has_modrm;
        if b == 0x0F {
            let b2 = self.rd8(p);
            p = p.wrapping_add(1);
            op.h = 0x100 | b2 as u16;
            has_modrm = modrm0f(b2);
        } else {
            op.h = b as u16;
            has_modrm = MODRM1[b as usize];
            op.reg = b & 7;
        }
        if has_modrm {
            let m = self.rd8(p);
            p = p.wrapping_add(1);
            let md = m >> 6;
            op.reg = (m >> 3) & 7;
            let rm = m & 7;
            if md == 3 {
                op.rm = rm;
            } else {
                op.fl |= F_MEM;
                let mut ss = false;
                if op.fl & F_A16 != 0 {
                    // 16-bit addressing: encode base/index pairs; disp16
                    let (bs, ix, s) = match rm {
                        0 => (EBX as u8, ESI as u8, false),
                        1 => (EBX as u8, EDI as u8, false),
                        2 => (EBP as u8, ESI as u8, true),
                        3 => (EBP as u8, EDI as u8, true),
                        4 => (ESI as u8, NOREG, false),
                        5 => (EDI as u8, NOREG, false),
                        6 => {
                            if md == 0 {
                                (NOREG, NOREG, false)
                            } else {
                                (EBP as u8, NOREG, true)
                            }
                        }
                        _ => (EBX as u8, NOREG, false),
                    };
                    op.base = bs;
                    op.index = ix;
                    ss = s;
                    if md == 0 && rm == 6 {
                        op.disp = self.rd16(p) as u32;
                        p = p.wrapping_add(2);
                    } else if md == 1 {
                        op.disp = self.rd8(p) as i8 as u16 as u32;
                        p = p.wrapping_add(1);
                    } else if md == 2 {
                        op.disp = self.rd16(p) as u32;
                        p = p.wrapping_add(2);
                    }
                } else {
                    if rm == 4 {
                        let sib = self.rd8(p);
                        p = p.wrapping_add(1);
                        op.scale = sib >> 6;
                        let idx = (sib >> 3) & 7;
                        let base = sib & 7;
                        if idx != 4 {
                            op.index = idx;
                        }
                        if base == 5 && md == 0 {
                            op.disp = self.rd32(p);
                            p = p.wrapping_add(4);
                        } else {
                            op.base = base;
                            if base == ESP as u8 || base == EBP as u8 {
                                ss = true;
                            }
                        }
                    } else if rm == 5 && md == 0 {
                        op.disp = self.rd32(p);
                        p = p.wrapping_add(4);
                    } else {
                        op.base = rm;
                        if rm == EBP as u8 {
                            ss = true;
                        }
                    }
                    if md == 1 {
                        op.disp = op.disp.wrapping_add(self.rd8(p) as i8 as u32);
                        p = p.wrapping_add(1);
                    } else if md == 2 {
                        op.disp = op.disp.wrapping_add(self.rd32(p));
                        p = p.wrapping_add(4);
                    }
                }
                op.seg = if ss { SS as u8 } else { DS as u8 };
            }
        }
        if let Some(s) = sovr {
            op.seg = s;
            op.fl |= F_SEGOVR;
        }
        // immediates
        let mut imm = |n: u32, p: &mut u32| -> u32 {
            let v = match n {
                1 => self.rd8(*p) as u32,
                2 => self.rd16(*p) as u32,
                _ => self.rd32(*p),
            };
            *p = p.wrapping_add(n);
            v
        };
        match op.h {
            0x00..=0x3F if b & 7 == 4 => op.imm = imm(1, &mut p),
            0x00..=0x3F if b & 7 == 5 => op.imm = imm(vs, &mut p),
            0x68 | 0x69 | 0xA9 | 0xB8..=0xBF | 0xC7 | 0x81 | 0xE8 | 0xE9 => {
                op.imm = imm(vs, &mut p);
                if matches!(op.h, 0xE8 | 0xE9) && vs == 2 {
                    op.imm = op.imm as u16 as i16 as u32;
                }
            }
            0x6A | 0x6B | 0x70..=0x7F | 0x80 | 0x82 | 0x83 | 0xA8 | 0xB0..=0xB7 | 0xC0 | 0xC1 | 0xC6 | 0xCD
            | 0xD4 | 0xD5 | 0xE0..=0xE7 | 0xEB => op.imm = imm(1, &mut p),
            0xA0..=0xA3 => op.imm = imm(if op.fl & F_A16 != 0 { 2 } else { 4 }, &mut p),
            0xC2 | 0xCA => op.imm = imm(2, &mut p),
            0xC8 => {
                op.imm = imm(2, &mut p);
                op.imm2 = imm(1, &mut p);
            }
            0x9A | 0xEA => {
                op.imm = imm(vs, &mut p);
                op.imm2 = imm(2, &mut p);
            }
            0xF6 | 0xF7 => {
                if op.reg < 2 {
                    op.imm = imm(if op.h == 0xF6 { 1 } else { vs }, &mut p);
                }
            }
            0x180..=0x18F => {
                op.imm = imm(vs, &mut p);
                if vs == 2 {
                    op.imm = op.imm as u16 as i16 as u32;
                }
            }
            0x1A4 | 0x1AC | 0x1BA => op.imm = imm(1, &mut p),
            0x1FF => op.imm = imm(2, &mut p),
            _ => {}
        }
        op.len = p.wrapping_sub(a) as u8;
        op.fl |= F_VALID;
        op
    }
}
