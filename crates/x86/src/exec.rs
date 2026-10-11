//! Instruction executor operating on pre-decoded `Op` records.
use crate::decode::*;
use crate::*;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Rm {
    Reg(usize),
    Mem(u32),
}

impl Cpu {
    /// Linear address of the memory operand.
    #[inline(always)]
    pub(crate) fn ea(&self, op: &Op) -> u32 {
        let mut a = op.disp;
        if op.base != NOREG {
            a = a.wrapping_add(self.r[op.base as usize]);
        }
        if op.index != NOREG {
            a = a.wrapping_add(self.r[op.index as usize] << op.scale);
        }
        if op.fl & F_A16 != 0 {
            a &= 0xFFFF;
        }
        self.seg[op.seg as usize].base.wrapping_add(a)
    }
    /// Offset of the memory operand without segment base (LEA).
    #[inline(always)]
    fn ea_off(&self, op: &Op) -> u32 {
        let mut a = op.disp;
        if op.base != NOREG {
            a = a.wrapping_add(self.r[op.base as usize]);
        }
        if op.index != NOREG {
            a = a.wrapping_add(self.r[op.index as usize] << op.scale);
        }
        if op.fl & F_A16 != 0 {
            a &= 0xFFFF;
        }
        a
    }
    #[inline(always)]
    pub(crate) fn loc(&self, op: &Op) -> Rm {
        if op.is_mem() {
            Rm::Mem(self.ea(op))
        } else {
            Rm::Reg(op.rm as usize)
        }
    }

    #[inline(always)]
    pub(crate) fn rd_rm(&self, rm: Rm, sz: u8) -> u32 {
        match rm {
            Rm::Reg(r) => match sz {
                1 => self.reg8(r) as u32,
                2 => self.r[r] & 0xFFFF,
                _ => self.r[r],
            },
            Rm::Mem(a) => match sz {
                1 => self.rd8(a) as u32,
                2 => self.rd16(a) as u32,
                _ => self.rd32(a),
            },
        }
    }
    #[inline(always)]
    pub(crate) fn wr_rm(&mut self, rm: Rm, sz: u8, v: u32) {
        match rm {
            Rm::Reg(r) => match sz {
                1 => self.set_reg8(r, v as u8),
                2 => self.set_reg16(r, v as u16),
                _ => self.r[r] = v,
            },
            Rm::Mem(a) => match sz {
                1 => self.wr8(a, v as u8),
                2 => self.wr16(a, v as u16),
                _ => self.wr32(a, v),
            },
        }
    }
    #[inline(always)]
    fn rd_reg(&self, r: usize, sz: u8) -> u32 {
        self.rd_rm(Rm::Reg(r), sz)
    }
    #[inline(always)]
    fn wr_reg(&mut self, r: usize, sz: u8, v: u32) {
        self.wr_rm(Rm::Reg(r), sz, v)
    }

    // ------------------------------------------------------------ ALU
    #[inline(always)]
    fn alu(&mut self, op: usize, sz: u8, a: u32, b: u32) -> u32 {
        let m = mask(sz);
        match op {
            0 => {
                let r = a.wrapping_add(b) & m;
                self.set_lazy(Lop::Add, sz, a, b, r);
                r
            }
            1 => {
                let r = (a | b) & m;
                self.set_lazy(Lop::Logic, sz, a, b, r);
                r
            }
            2 => {
                let c = self.get_cf();
                let r = a.wrapping_add(b).wrapping_add(c as u32) & m;
                self.set_lazy(Lop::Adc, sz, a, b, r);
                self.lcf = c;
                r
            }
            3 => {
                let c = self.get_cf();
                let r = a.wrapping_sub(b).wrapping_sub(c as u32) & m;
                self.set_lazy(Lop::Sbb, sz, a, b, r);
                self.lcf = c;
                r
            }
            4 => {
                let r = a & b & m;
                self.set_lazy(Lop::Logic, sz, a, b, r);
                r
            }
            5 | 7 => {
                let r = a.wrapping_sub(b) & m;
                self.set_lazy(Lop::Sub, sz, a, b, r);
                r
            }
            _ => {
                let r = (a ^ b) & m;
                self.set_lazy(Lop::Logic, sz, a, b, r);
                r
            }
        }
    }

    fn inc_dec(&mut self, sz: u8, a: u32, dec: bool) -> u32 {
        let c = self.get_cf();
        let m = mask(sz);
        let r = if dec { a.wrapping_sub(1) } else { a.wrapping_add(1) } & m;
        self.set_lazy(if dec { Lop::Dec } else { Lop::Inc }, sz, a, 1, r);
        self.lcf = c;
        r
    }

    fn set_flag_bits(&mut self, bits: u32, on: bool) {
        self.materialize();
        if on {
            self.fl |= bits
        } else {
            self.fl &= !bits
        }
    }

    fn set_szp(&mut self, sz: u8, r: u32) {
        // Set SF/ZF/PF from r; clears nothing else (flags must be materialized).
        let m = mask(sz);
        let s = sbit(sz);
        let r = r & m;
        self.fl &= !(SF | ZF | PF);
        if r == 0 {
            self.fl |= ZF
        }
        if r & s != 0 {
            self.fl |= SF
        }
        if (r as u8).count_ones() % 2 == 0 {
            self.fl |= PF
        }
    }

    fn shift(&mut self, op: usize, sz: u8, a: u32, cnt: u32) -> u32 {
        let cnt = cnt & 31;
        if cnt == 0 {
            return a;
        }
        let bits = sz as u32 * 8;
        let m = mask(sz);
        let s = sbit(sz);
        match op {
            0 => {
                // ROL
                let c = cnt % bits;
                let r = if c == 0 { a } else { ((a << c) | (a >> (bits - c))) & m };
                self.materialize();
                let cf = r & 1 != 0;
                self.set_flag_bits(CF, cf);
                self.set_flag_bits(OF, ((r & s) != 0) != cf);
                r
            }
            1 => {
                let c = cnt % bits;
                let r = if c == 0 { a } else { ((a >> c) | (a << (bits - c))) & m };
                self.materialize();
                self.set_flag_bits(CF, r & s != 0);
                self.set_flag_bits(OF, ((r ^ (r << 1)) & s) != 0);
                r
            }
            2 => {
                // RCL
                let c = cnt % (bits + 1);
                let mut r = a as u64;
                let mut cf = self.get_cf() as u64;
                for _ in 0..c {
                    let nc = (r >> (bits - 1)) & 1;
                    r = ((r << 1) | cf) & m as u64;
                    cf = nc;
                }
                self.materialize();
                self.set_flag_bits(CF, cf != 0);
                self.set_flag_bits(OF, ((r as u32 & s) != 0) != (cf != 0));
                r as u32
            }
            3 => {
                // RCR
                let c = cnt % (bits + 1);
                let mut r = a as u64;
                let mut cf = self.get_cf() as u64;
                let of = ((a & s) != 0) != (cf != 0);
                for _ in 0..c {
                    let nc = r & 1;
                    r = (r >> 1) | (cf << (bits - 1));
                    cf = nc;
                }
                self.materialize();
                self.set_flag_bits(CF, cf != 0);
                self.set_flag_bits(OF, of);
                r as u32
            }
            4 | 6 => {
                let r = if cnt >= 32 { 0 } else { (a << cnt) & m };
                self.set_lazy(Lop::Shl, sz, a, cnt, r);
                r
            }
            5 => {
                let r = if cnt >= 32 { 0 } else { (a & m) >> cnt };
                self.set_lazy(Lop::Shr, sz, a, cnt, r);
                r
            }
            _ => {
                let r = (sext(a, sz) >> cnt.min(31)) as u32 & m;
                self.set_lazy(Lop::Sar, sz, a, cnt, r);
                r
            }
        }
    }

    // ------------------------------------------------------------ string ops
    fn str_src(&self) -> u32 {
        self.seg[self.str_seg].base.wrapping_add(self.r[ESI])
    }
    fn str_dst(&self) -> u32 {
        self.seg[ES].base.wrapping_add(self.r[EDI])
    }
    fn str_step(&self, sz: u8) -> u32 {
        if self.df() {
            (sz as u32).wrapping_neg()
        } else {
            sz as u32
        }
    }

    #[inline(always)]
    fn overlaps_code(&self, a: u32, n: u64) -> bool {
        (a as u64) < self.code_lo as u64 + self.code_len as u64 && a as u64 + n + 16 > self.code_lo as u64
    }

    fn movs(&mut self, sz: u8) {
        let st = self.str_step(sz);
        if self.rep != 0 {
            let n = self.r[ECX];
            if n == 0 {
                return;
            }
            let src = self.str_src();
            let dst = self.str_dst();
            let bytes = n as u64 * sz as u64;
            // Fast path: forward, non-overlapping-in-bad-direction, in range
            if !self.df()
                && !self.wlog_on
                && !self.overlaps_code(dst, bytes)
                && bytes < MEM_SIZE as u64
                && (src as u64 + bytes) <= MEM_SIZE as u64
                && (dst as u64 + bytes) <= MEM_SIZE as u64
                && (dst <= src || dst as u64 >= src as u64 + bytes)
            {
                self.mem.copy_within(src as usize..(src as u64 + bytes) as usize, dst as usize);
                self.r[ESI] = self.r[ESI].wrapping_add(bytes as u32);
                self.r[EDI] = self.r[EDI].wrapping_add(bytes as u32);
                self.r[ECX] = 0;
                return;
            }
            while self.r[ECX] != 0 {
                let v = self.rd_m(self.str_src(), sz);
                let d = self.str_dst();
                self.wr_m(d, sz, v);
                self.r[ESI] = self.r[ESI].wrapping_add(st);
                self.r[EDI] = self.r[EDI].wrapping_add(st);
                self.r[ECX] = self.r[ECX].wrapping_sub(1);
            }
        } else {
            let v = self.rd_m(self.str_src(), sz);
            let d = self.str_dst();
            self.wr_m(d, sz, v);
            self.r[ESI] = self.r[ESI].wrapping_add(st);
            self.r[EDI] = self.r[EDI].wrapping_add(st);
        }
    }

    fn stos(&mut self, sz: u8) {
        let st = self.str_step(sz);
        let v = self.rd_reg(EAX, sz);
        if self.rep != 0 {
            let n = self.r[ECX];
            let dst = self.str_dst();
            let bytes = n as u64 * sz as u64;
            if !self.df() && sz == 1 && !self.wlog_on && !self.overlaps_code(dst, bytes) && (dst as u64 + bytes) <= MEM_SIZE as u64 {
                self.mem[dst as usize..(dst as u64 + bytes) as usize].fill(v as u8);
                self.r[EDI] = self.r[EDI].wrapping_add(bytes as u32);
                self.r[ECX] = 0;
                return;
            }
            while self.r[ECX] != 0 {
                let d = self.str_dst();
                self.wr_m(d, sz, v);
                self.r[EDI] = self.r[EDI].wrapping_add(st);
                self.r[ECX] = self.r[ECX].wrapping_sub(1);
            }
        } else {
            let d = self.str_dst();
            self.wr_m(d, sz, v);
            self.r[EDI] = self.r[EDI].wrapping_add(st);
        }
    }

    fn lods(&mut self, sz: u8) {
        let st = self.str_step(sz);
        if self.rep != 0 {
            while self.r[ECX] != 0 {
                let v = self.rd_m(self.str_src(), sz);
                self.wr_reg(EAX, sz, v);
                self.r[ESI] = self.r[ESI].wrapping_add(st);
                self.r[ECX] = self.r[ECX].wrapping_sub(1);
            }
        } else {
            let v = self.rd_m(self.str_src(), sz);
            self.wr_reg(EAX, sz, v);
            self.r[ESI] = self.r[ESI].wrapping_add(st);
        }
    }

    fn cmps(&mut self, sz: u8) {
        let st = self.str_step(sz);
        loop {
            if self.rep != 0 && self.r[ECX] == 0 {
                break;
            }
            let a = self.rd_m(self.str_src(), sz);
            let b = self.rd_m(self.str_dst(), sz);
            self.alu(7, sz, a, b);
            self.r[ESI] = self.r[ESI].wrapping_add(st);
            self.r[EDI] = self.r[EDI].wrapping_add(st);
            if self.rep == 0 {
                break;
            }
            self.r[ECX] = self.r[ECX].wrapping_sub(1);
            let z = self.get_zf();
            if (self.rep == 0xF3 && !z) || (self.rep == 0xF2 && z) {
                break;
            }
        }
    }

    fn scas(&mut self, sz: u8) {
        let st = self.str_step(sz);
        let a = self.rd_reg(EAX, sz);
        loop {
            if self.rep != 0 && self.r[ECX] == 0 {
                break;
            }
            let b = self.rd_m(self.str_dst(), sz);
            self.alu(7, sz, a, b);
            self.r[EDI] = self.r[EDI].wrapping_add(st);
            if self.rep == 0 {
                break;
            }
            self.r[ECX] = self.r[ECX].wrapping_sub(1);
            let z = self.get_zf();
            if (self.rep == 0xF3 && !z) || (self.rep == 0xF2 && z) {
                break;
            }
        }
    }

    #[inline(always)]
    pub(crate) fn rd_m(&self, a: u32, sz: u8) -> u32 {
        match sz {
            1 => self.rd8(a) as u32,
            2 => self.rd16(a) as u32,
            _ => self.rd32(a),
        }
    }
    #[inline(always)]
    pub(crate) fn wr_m(&mut self, a: u32, sz: u8, v: u32) {
        match sz {
            1 => self.wr8(a, v as u8),
            2 => self.wr16(a, v as u16),
            _ => self.wr32(a, v),
        }
    }

    fn push_v(&mut self, v: u32) {
        if self.o16 {
            self.push16(v as u16)
        } else {
            self.push32(v)
        }
    }
    fn pop_v(&mut self) -> u32 {
        if self.o16 {
            self.pop16() as u32
        } else {
            self.pop32()
        }
    }

    fn jump_rel(&mut self, rel: u32) {
        self.eip = self.eip.wrapping_add(rel);
        if self.o16 {
            self.eip &= 0xFFFF;
        }
    }

    // ------------------------------------------------------------ mul / div
    fn mul_group(&mut self, op: usize, sz: u8, src: u32) -> Result<(), Fault> {
        match op {
            4 => {
                // MUL
                let (hi_nz, ) = match sz {
                    1 => {
                        let r = (self.r[EAX] & 0xFF) * src;
                        self.set_reg16(EAX, r as u16);
                        (r > 0xFF,)
                    }
                    2 => {
                        let r = (self.r[EAX] & 0xFFFF) * src;
                        self.set_reg16(EAX, r as u16);
                        self.set_reg16(EDX, (r >> 16) as u16);
                        (r > 0xFFFF,)
                    }
                    _ => {
                        let r = self.r[EAX] as u64 * src as u64;
                        self.r[EAX] = r as u32;
                        self.r[EDX] = (r >> 32) as u32;
                        (r > 0xFFFF_FFFF,)
                    }
                };
                self.materialize();
                self.set_flag_bits(CF | OF, hi_nz);
            }
            5 => {
                // IMUL (one operand)
                let ovf = match sz {
                    1 => {
                        let r = (self.r[EAX] as u8 as i8 as i32) * (src as u8 as i8 as i32);
                        self.set_reg16(EAX, r as u16);
                        r != r as i8 as i32
                    }
                    2 => {
                        let r = (self.r[EAX] as u16 as i16 as i32) * (src as u16 as i16 as i32);
                        self.set_reg16(EAX, r as u16);
                        self.set_reg16(EDX, (r >> 16) as u16);
                        r != r as i16 as i32
                    }
                    _ => {
                        let r = (self.r[EAX] as i32 as i64) * (src as i32 as i64);
                        self.r[EAX] = r as u32;
                        self.r[EDX] = (r >> 32) as u32;
                        r != r as i32 as i64
                    }
                };
                self.materialize();
                self.set_flag_bits(CF | OF, ovf);
            }
            6 => {
                // DIV
                if src == 0 {
                    return Err(Fault::DivideError);
                }
                match sz {
                    1 => {
                        let n = self.r[EAX] & 0xFFFF;
                        let q = n / src;
                        if q > 0xFF {
                            return Err(Fault::DivideError);
                        }
                        self.set_reg8(0, q as u8);
                        self.set_reg8(4, (n % src) as u8);
                    }
                    2 => {
                        let n = ((self.r[EDX] & 0xFFFF) << 16) | (self.r[EAX] & 0xFFFF);
                        let q = n / src;
                        if q > 0xFFFF {
                            return Err(Fault::DivideError);
                        }
                        self.set_reg16(EAX, q as u16);
                        self.set_reg16(EDX, (n % src) as u16);
                    }
                    _ => {
                        let n = ((self.r[EDX] as u64) << 32) | self.r[EAX] as u64;
                        let q = n / src as u64;
                        if q > 0xFFFF_FFFF {
                            return Err(Fault::DivideError);
                        }
                        self.r[EAX] = q as u32;
                        self.r[EDX] = (n % src as u64) as u32;
                    }
                }
            }
            _ => {
                // IDIV
                if src == 0 {
                    return Err(Fault::DivideError);
                }
                match sz {
                    1 => {
                        let n = self.r[EAX] as u16 as i16 as i32;
                        let d = src as u8 as i8 as i32;
                        let q = n / d;
                        if q != q as i8 as i32 {
                            return Err(Fault::DivideError);
                        }
                        self.set_reg8(0, q as u8);
                        self.set_reg8(4, (n % d) as u8);
                    }
                    2 => {
                        let n = (((self.r[EDX] & 0xFFFF) << 16) | (self.r[EAX] & 0xFFFF)) as i32;
                        let d = src as u16 as i16 as i32;
                        let q = n.wrapping_div(d);
                        if q != q as i16 as i32 {
                            return Err(Fault::DivideError);
                        }
                        self.set_reg16(EAX, q as u16);
                        self.set_reg16(EDX, n.wrapping_rem(d) as u16);
                    }
                    _ => {
                        let n = (((self.r[EDX] as u64) << 32) | self.r[EAX] as u64) as i64;
                        let d = src as i32 as i64;
                        let q = n.wrapping_div(d);
                        if q != q as i32 as i64 {
                            return Err(Fault::DivideError);
                        }
                        self.r[EAX] = q as u32;
                        self.r[EDX] = n.wrapping_rem(d) as u32;
                    }
                }
            }
        }
        Ok(())
    }

    fn imul3(&mut self, sz: u8, a: u32, b: u32) -> u32 {
        let (r, ovf) = if sz == 2 {
            let r = (a as u16 as i16 as i32) * (b as u16 as i16 as i32);
            (r as u32 & 0xFFFF, r != r as i16 as i32)
        } else {
            let r = (a as i32 as i64) * (b as i32 as i64);
            (r as u32, r != r as i32 as i64)
        };
        self.materialize();
        self.set_flag_bits(CF | OF, ovf);
        r
    }

    fn far_ptr(&mut self, rm: Rm) -> (u32, u16) {
        match rm {
            Rm::Mem(a) => {
                if self.o16 {
                    (self.rd16(a) as u32, self.rd16(a.wrapping_add(2)))
                } else {
                    (self.rd32(a), self.rd16(a.wrapping_add(4)))
                }
            }
            Rm::Reg(_) => (0, 0),
        }
    }

    // ------------------------------------------------------------ step
    #[inline(always)]
    pub fn step(&mut self, bus: &mut dyn Bus) -> Result<(), Fault> {
        let eip = self.eip;
        self.cur_eip = eip;
        if self.trace_on {
            let n = self.trace.len();
            self.trace[self.trace_pos & (n - 1)] = eip;
            self.trace_pos = self.trace_pos.wrapping_add(1);
        }
        let lin = self.seg[CS].base.wrapping_add(eip);
        let idx = lin.wrapping_sub(self.code_lo);
        let op = if idx < self.code_len {
            let o = unsafe { *self.icache.get_unchecked(idx as usize) };
            if o.fl & F_VALID != 0 {
                o
            } else {
                let o = self.decode_at(lin);
                self.icache[idx as usize] = o;
                o
            }
        } else {
            self.decode_at(lin)
        };
        self.eip = eip.wrapping_add(op.len as u32);
        self.icount += 1;
        self.exec(bus, &op)
    }

    #[inline(always)]
    fn exec(&mut self, bus: &mut dyn Bus, op: &Op) -> Result<(), Fault> {
        let o16 = op.fl & F_O16 != 0;
        self.o16 = o16;
        let v: u8 = if o16 { 2 } else { 4 };
        let h = op.h;
        match h {
            // ALU r/m,r ; r,r/m ; acc,imm
            0x00..=0x3F if h & 7 < 6 => {
                let alu_op = (h >> 3) as usize;
                let sz = if h & 1 == 0 { 1 } else { v };
                match h & 7 {
                    0 | 1 => {
                        let rm = self.loc(op);
                        let a = self.rd_rm(rm, sz);
                        let b = self.rd_reg(op.reg as usize, sz);
                        let r = self.alu(alu_op, sz, a, b);
                        if alu_op != 7 {
                            self.wr_rm(rm, sz, r);
                        }
                    }
                    2 | 3 => {
                        let rm = self.loc(op);
                        let a = self.rd_reg(op.reg as usize, sz);
                        let b = self.rd_rm(rm, sz);
                        let r = self.alu(alu_op, sz, a, b);
                        if alu_op != 7 {
                            self.wr_reg(op.reg as usize, sz, r);
                        }
                    }
                    _ => {
                        let a = self.rd_reg(EAX, sz);
                        let r = self.alu(alu_op, sz, a, op.imm);
                        if alu_op != 7 {
                            self.wr_reg(EAX, sz, r);
                        }
                    }
                }
            }
            0x06 => self.push_v(self.seg[ES].sel as u32),
            0x07 => {
                let s = self.pop_v() as u16;
                self.load_seg(ES, s)
            }
            0x0E => self.push_v(self.seg[CS].sel as u32),
            0x16 => self.push_v(self.seg[SS].sel as u32),
            0x17 => {
                let s = self.pop_v() as u16;
                self.load_seg(SS, s)
            }
            0x1E => self.push_v(self.seg[DS].sel as u32),
            0x1F => {
                let s = self.pop_v() as u16;
                self.load_seg(DS, s)
            }
            0x27 | 0x2F | 0x37 | 0x3F => return Err(Fault::Unimplemented("BCD adjust")),
            0x40..=0x47 => {
                let r = (h & 7) as usize;
                let a = self.rd_reg(r, v);
                let x = self.inc_dec(v, a, false);
                self.wr_reg(r, v, x);
            }
            0x48..=0x4F => {
                let r = (h & 7) as usize;
                let a = self.rd_reg(r, v);
                let x = self.inc_dec(v, a, true);
                self.wr_reg(r, v, x);
            }
            0x50..=0x57 => {
                let val = self.r[(h & 7) as usize];
                self.push_v(val)
            }
            0x58..=0x5F => {
                let val = self.pop_v();
                self.wr_reg((h & 7) as usize, v, val)
            }
            0x60 => {
                let sp = self.r[ESP];
                for i in 0..8 {
                    let val = if i == ESP { sp } else { self.r[i] };
                    self.push_v(val);
                }
            }
            0x61 => {
                for i in (0..8).rev() {
                    let val = self.pop_v();
                    if i != ESP {
                        self.wr_reg(i, v, val);
                    }
                }
            }
            0x62 | 0x63 => {} // BOUND / ARPL: ignored
            0x68 => self.push_v(op.imm),
            0x6A => self.push_v(op.imm as u8 as i8 as u32),
            0x69 | 0x6B => {
                let rm = self.loc(op);
                let a = self.rd_rm(rm, v);
                let b = if h == 0x6B { op.imm as u8 as i8 as u32 } else { op.imm };
                let r = self.imul3(v, a, b);
                self.wr_reg(op.reg as usize, v, r);
            }
            0x6C..=0x6F => {
                self.rep = rep_kind(op);
                self.str_seg = op.seg as usize;
                let sz = if h & 1 == 0 { 1 } else { v };
                let st = self.str_step(sz);
                loop {
                    if self.rep != 0 && self.r[ECX] == 0 {
                        break;
                    }
                    let port = self.r[EDX] as u16;
                    if h < 0x6E {
                        let val = bus.port_in(self, port, sz);
                        let d = self.str_dst();
                        self.wr_m(d, sz, val);
                        self.r[EDI] = self.r[EDI].wrapping_add(st);
                    } else {
                        let val = self.rd_m(self.str_src(), sz);
                        bus.port_out(self, port, sz, val);
                        self.r[ESI] = self.r[ESI].wrapping_add(st);
                    }
                    if self.rep == 0 {
                        break;
                    }
                    self.r[ECX] = self.r[ECX].wrapping_sub(1);
                }
            }
            0x70..=0x7F => {
                if self.cond((h & 15) as u8) {
                    self.jump_rel(op.imm as u8 as i8 as u32);
                }
            }
            0x80..=0x83 => {
                let sz = if h & 1 == 0 { 1 } else { v };
                let rm = self.loc(op);
                let b = if h == 0x83 { op.imm as u8 as i8 as u32 & mask(sz) } else { op.imm };
                let a = self.rd_rm(rm, sz);
                let r = self.alu(op.reg as usize, sz, a, b);
                if op.reg != 7 {
                    self.wr_rm(rm, sz, r);
                }
            }
            0x84 | 0x85 => {
                let sz = if h & 1 == 0 { 1 } else { v };
                let rm = self.loc(op);
                let a = self.rd_rm(rm, sz);
                let b = self.rd_reg(op.reg as usize, sz);
                self.alu(4, sz, a, b);
            }
            0x86 | 0x87 => {
                let sz = if h & 1 == 0 { 1 } else { v };
                let rm = self.loc(op);
                let a = self.rd_rm(rm, sz);
                let b = self.rd_reg(op.reg as usize, sz);
                self.wr_rm(rm, sz, b);
                self.wr_reg(op.reg as usize, sz, a);
            }
            0x88 => {
                let b = self.reg8(op.reg as usize) as u32;
                let rm = self.loc(op);
                self.wr_rm(rm, 1, b);
            }
            0x89 => {
                let b = self.rd_reg(op.reg as usize, v);
                if op.is_mem() {
                    let a = self.ea(op);
                    self.wr_m(a, v, b);
                } else {
                    self.wr_reg(op.rm as usize, v, b);
                }
            }
            0x8A => {
                let rm = self.loc(op);
                let b = self.rd_rm(rm, 1);
                self.set_reg8(op.reg as usize, b as u8);
            }
            0x8B => {
                let b = if op.is_mem() { self.rd_m(self.ea(op), v) } else { self.rd_reg(op.rm as usize, v) };
                self.wr_reg(op.reg as usize, v, b);
            }
            0x8C => {
                let s = self.seg[(op.reg & 7) as usize % 6].sel as u32;
                if op.is_mem() {
                    let a = self.ea(op);
                    self.wr_m(a, 2, s);
                } else {
                    self.wr_reg(op.rm as usize, v, s);
                }
            }
            0x8D => {
                let a = self.ea_off(op);
                self.wr_reg(op.reg as usize, v, a);
            }
            0x8E => {
                let rm = self.loc(op);
                let s = self.rd_rm(rm, 2) as u16;
                self.load_seg((op.reg as usize) % 6, s);
            }
            0x8F => {
                // POP r/m: ESP is incremented before the address is computed
                let val = self.pop_v();
                let rm = self.loc(op);
                self.wr_rm(rm, v, val);
            }
            0x90 => {}
            0x91..=0x97 => {
                let r = (h & 7) as usize;
                let a = self.rd_reg(EAX, v);
                let b = self.rd_reg(r, v);
                self.wr_reg(EAX, v, b);
                self.wr_reg(r, v, a);
            }
            0x98 => {
                if o16 {
                    let al = self.r[EAX] as u8 as i8 as i16 as u16;
                    self.set_reg16(EAX, al)
                } else {
                    self.r[EAX] = self.r[EAX] as u16 as i16 as i32 as u32
                }
            }
            0x99 => {
                if o16 {
                    let hi = if self.r[EAX] & 0x8000 != 0 { 0xFFFF } else { 0 };
                    self.set_reg16(EDX, hi)
                } else {
                    self.r[EDX] = if self.r[EAX] & 0x8000_0000 != 0 { 0xFFFF_FFFF } else { 0 }
                }
            }
            0x9A => {
                self.push_v(self.seg[CS].sel as u32);
                self.push_v(self.eip);
                self.load_seg(CS, op.imm2 as u16);
                self.eip = op.imm;
            }
            0x9B => {}
            0x9C => {
                let f = self.flags();
                self.push_v(f & !0x30000)
            }
            0x9D => {
                let f = self.pop_v();
                let f = if o16 { (self.flags() & 0xFFFF_0000) | f } else { f };
                self.set_flags(f);
            }
            0x9E => {
                self.materialize();
                let ah = self.reg8(4) as u32;
                self.fl = (self.fl & !0xD5) | (ah & 0xD5) | 2;
            }
            0x9F => {
                let f = self.flags();
                self.set_reg8(4, f as u8)
            }
            0xA0..=0xA3 => {
                let sz = if h & 1 == 0 { 1 } else { v };
                let a = self.seg[op.seg as usize].base.wrapping_add(op.imm);
                if h < 0xA2 {
                    let val = self.rd_m(a, sz);
                    self.wr_reg(EAX, sz, val);
                } else {
                    let val = self.rd_reg(EAX, sz);
                    self.wr_m(a, sz, val);
                }
            }
            0xA4..=0xA7 | 0xAA..=0xAF => {
                if op.fl & F_A16 != 0 {
                    return Err(Fault::Unimplemented("a16 string op"));
                }
                self.rep = rep_kind(op);
                self.str_seg = op.seg as usize;
                let sz = if h & 1 == 0 { 1 } else { v };
                match h {
                    0xA4 | 0xA5 => self.movs(sz),
                    0xA6 | 0xA7 => self.cmps(sz),
                    0xAA | 0xAB => self.stos(sz),
                    0xAC | 0xAD => self.lods(sz),
                    _ => self.scas(sz),
                }
            }
            0xA8 | 0xA9 => {
                let sz = if h & 1 == 0 { 1 } else { v };
                let a = self.rd_reg(EAX, sz);
                self.alu(4, sz, a, op.imm);
            }
            0xB0..=0xB7 => self.set_reg8((h & 7) as usize, op.imm as u8),
            0xB8..=0xBF => self.wr_reg((h & 7) as usize, v, op.imm),
            0xC0 | 0xC1 | 0xD0..=0xD3 => {
                let sz = if h & 1 == 0 { 1 } else { v };
                let rm = self.loc(op);
                let cnt = match h {
                    0xC0 | 0xC1 => op.imm,
                    0xD0 | 0xD1 => 1,
                    _ => self.r[ECX] & 0xFF,
                };
                let a = self.rd_rm(rm, sz);
                let r = self.shift(op.reg as usize, sz, a, cnt);
                self.wr_rm(rm, sz, r);
            }
            0xC2 => {
                self.eip = self.pop_v();
                self.r[ESP] = self.r[ESP].wrapping_add(op.imm);
            }
            0xC3 => self.eip = self.pop_v(),
            0xC4 | 0xC5 => {
                let rm = self.loc(op);
                let (off, sel) = self.far_ptr(rm);
                self.wr_reg(op.reg as usize, v, off);
                self.load_seg(if h == 0xC4 { ES } else { DS }, sel);
            }
            0xC6 | 0xC7 => {
                let sz = if h & 1 == 0 { 1 } else { v };
                let rm = self.loc(op);
                self.wr_rm(rm, sz, op.imm);
            }
            0xC8 => {
                let size = op.imm;
                let level = op.imm2 & 31;
                self.push32(self.r[EBP]);
                let frame = self.r[ESP];
                if level > 0 {
                    for _ in 1..level {
                        self.r[EBP] = self.r[EBP].wrapping_sub(4);
                        let val = self.rd32(self.seg[SS].base.wrapping_add(self.r[EBP]));
                        self.push32(val);
                    }
                    self.push32(frame);
                }
                self.r[EBP] = frame;
                self.r[ESP] = self.r[ESP].wrapping_sub(size);
            }
            0xC9 => {
                self.r[ESP] = self.r[EBP];
                let bp = self.pop_v();
                self.wr_reg(EBP, v, bp);
            }
            0xCA | 0xCB => {
                let n = if h == 0xCA { op.imm } else { 0 };
                self.eip = self.pop_v();
                let cs = self.pop_v() as u16;
                self.load_seg(CS, cs);
                self.r[ESP] = self.r[ESP].wrapping_add(n);
            }
            0xCC => return bus.interrupt(self, 3),
            0xCD => return bus.interrupt(self, op.imm as u8),
            0xCE => {
                if self.get_of() {
                    return bus.interrupt(self, 4);
                }
            }
            0xCF => {
                if o16 {
                    self.eip = self.pop16() as u32;
                    let cs = self.pop16();
                    self.load_seg(CS, cs);
                    let f = self.pop16() as u32;
                    self.set_flags((self.flags() & 0xFFFF_0000) | f);
                } else {
                    self.eip = self.pop32();
                    let cs = self.pop32() as u16;
                    self.load_seg(CS, cs);
                    let f = self.pop32();
                    self.set_flags(f);
                }
            }
            0xD4 => {
                let b = op.imm as u8;
                if b == 0 {
                    return Err(Fault::DivideError);
                }
                let al = self.reg8(0);
                self.set_reg8(4, al / b);
                self.set_reg8(0, al % b);
                self.materialize();
                let r = self.r[EAX] & 0xFF;
                self.set_szp(1, r);
            }
            0xD5 => {
                let b = op.imm as u8;
                let al = self.reg8(0).wrapping_add(self.reg8(4).wrapping_mul(b));
                self.set_reg16(EAX, al as u16);
                self.materialize();
                self.set_szp(1, al as u32);
            }
            0xD6 => {
                let c = self.get_cf();
                self.set_reg8(0, if c { 0xFF } else { 0 })
            }
            0xD7 => {
                let a = self.seg[op.seg as usize].base.wrapping_add(self.r[EBX].wrapping_add(self.r[EAX] & 0xFF));
                let val = self.rd8(a);
                self.set_reg8(0, val);
            }
            0xD8..=0xDF => return self.exec_fpu(op),
            0xE0..=0xE3 => {
                let rel = op.imm as u8 as i8 as u32;
                let a16 = op.fl & F_A16 != 0;
                let take = if h == 0xE3 {
                    if a16 {
                        self.r[ECX] & 0xFFFF == 0
                    } else {
                        self.r[ECX] == 0
                    }
                } else {
                    let c = if a16 {
                        let c = (self.r[ECX] as u16).wrapping_sub(1);
                        self.set_reg16(ECX, c);
                        c as u32
                    } else {
                        self.r[ECX] = self.r[ECX].wrapping_sub(1);
                        self.r[ECX]
                    };
                    match h {
                        0xE0 => c != 0 && !self.get_zf(),
                        0xE1 => c != 0 && self.get_zf(),
                        _ => c != 0,
                    }
                };
                if take {
                    self.jump_rel(rel);
                }
            }
            0xE4 | 0xE5 => {
                let sz = if h & 1 == 0 { 1 } else { v };
                let val = bus.port_in(self, op.imm as u8 as u16, sz);
                self.wr_reg(EAX, sz, val);
            }
            0xE6 | 0xE7 => {
                let sz = if h & 1 == 0 { 1 } else { v };
                let val = self.rd_reg(EAX, sz);
                bus.port_out(self, op.imm as u8 as u16, sz, val);
            }
            0xE8 => {
                self.push_v(self.eip);
                self.jump_rel(op.imm);
            }
            0xE9 => self.jump_rel(op.imm),
            0xEA => {
                self.load_seg(CS, op.imm2 as u16);
                self.eip = op.imm;
            }
            0xEB => self.jump_rel(op.imm as u8 as i8 as u32),
            0xEC | 0xED => {
                let sz = if h & 1 == 0 { 1 } else { v };
                let val = bus.port_in(self, self.r[EDX] as u16, sz);
                self.wr_reg(EAX, sz, val);
            }
            0xEE | 0xEF => {
                let sz = if h & 1 == 0 { 1 } else { v };
                let val = self.rd_reg(EAX, sz);
                bus.port_out(self, self.r[EDX] as u16, sz, val);
            }
            0xF4 => return Err(Fault::Halt),
            0xF5 => {
                let c = self.get_cf();
                self.set_cf(!c)
            }
            0xF6 | 0xF7 => {
                let sz = if h & 1 == 0 { 1 } else { v };
                let rm = self.loc(op);
                match op.reg {
                    0 | 1 => {
                        let a = self.rd_rm(rm, sz);
                        self.alu(4, sz, a, op.imm);
                    }
                    2 => {
                        let a = self.rd_rm(rm, sz);
                        self.wr_rm(rm, sz, !a & mask(sz));
                    }
                    3 => {
                        let a = self.rd_rm(rm, sz);
                        let r = a.wrapping_neg() & mask(sz);
                        self.set_lazy(Lop::Neg, sz, a, 0, r);
                        self.wr_rm(rm, sz, r);
                    }
                    _ => {
                        let src = self.rd_rm(rm, sz);
                        self.mul_group(op.reg as usize, sz, src)?;
                    }
                }
            }
            0xF8 => self.set_cf(false),
            0xF9 => self.set_cf(true),
            0xFA => self.fl &= !IF,
            0xFB => self.fl |= IF,
            0xFC => self.fl &= !DF,
            0xFD => self.fl |= DF,
            0xFE | 0xFF => {
                let sz = if h == 0xFE { 1 } else { v };
                let rm = self.loc(op);
                match op.reg {
                    0 | 1 => {
                        let a = self.rd_rm(rm, sz);
                        let r = self.inc_dec(sz, a, op.reg == 1);
                        self.wr_rm(rm, sz, r);
                    }
                    2 => {
                        let t = self.rd_rm(rm, v);
                        self.push_v(self.eip);
                        self.eip = t;
                    }
                    3 => {
                        let (off, sel) = self.far_ptr(rm);
                        self.push_v(self.seg[CS].sel as u32);
                        self.push_v(self.eip);
                        self.load_seg(CS, sel);
                        self.eip = off;
                    }
                    4 => self.eip = self.rd_rm(rm, v),
                    5 => {
                        let (off, sel) = self.far_ptr(rm);
                        self.load_seg(CS, sel);
                        self.eip = off;
                    }
                    6 => {
                        let val = self.rd_rm(rm, v);
                        self.push_v(val);
                    }
                    _ => return Err(Fault::InvalidOpcode(0xFF)),
                }
            }
            0x100..=0x1FF => return self.op0f(bus, op),
            _ => return Err(Fault::InvalidOpcode(h as u32)),
        }
        Ok(())
    }

    #[inline(never)]
    fn op0f(&mut self, bus: &mut dyn Bus, op: &Op) -> Result<(), Fault> {
        let v: u8 = if op.o16() { 2 } else { 4 };
        let h = (op.h & 0xFF) as u8;
        match h {
            0x00 => {
                if op.reg < 2 {
                    let rm = self.loc(op);
                    self.wr_rm(rm, 2, 0); // SLDT / STR
                }
            }
            0x01 => {
                let rm = self.loc(op);
                match op.reg {
                    0 | 1 => {
                        if let Rm::Mem(a) = rm {
                            self.wr16(a, 0xFFFF);
                            self.wr32(a.wrapping_add(2), 0);
                        }
                    }
                    4 => self.wr_rm(rm, 2, 0x0001), // SMSW: PE set
                    _ => {}
                }
            }
            0x02 | 0x03 => {
                // LAR / LSL
                let rm = self.loc(op);
                let sel = self.rd_rm(rm, 2) as u16;
                let d = self.desc_for(sel).copied();
                match d {
                    Some(d) if d.present || sel & !3 != 0 => {
                        let val = if h == 0x03 { d.limit } else { ((d.access as u32) << 8) | 0x00C0_0000 };
                        self.wr_reg(op.reg as usize, v, val);
                        self.set_zf(true);
                    }
                    _ => self.set_zf(false),
                }
            }
            0x06 | 0x08 | 0x09 | 0x0B => {}
            0x31 => {
                let t = self.icount;
                self.r[EAX] = t as u32;
                self.r[EDX] = (t >> 32) as u32;
            }
            0x80..=0x8F => {
                if self.cond(h & 15) {
                    self.jump_rel(op.imm);
                }
            }
            0x90..=0x9F => {
                let rm = self.loc(op);
                let c = self.cond(h & 15) as u32;
                self.wr_rm(rm, 1, c);
            }
            0xA0 => self.push_v(self.seg[FS].sel as u32),
            0xA1 => {
                let s = self.pop_v() as u16;
                self.load_seg(FS, s)
            }
            0xA2 => {
                // CPUID: report a 486
                self.r[EAX] = 0x0000_0400;
                self.r[EBX] = 0;
                self.r[ECX] = 0;
                self.r[EDX] = 0x0000_0001;
            }
            0xA8 => self.push_v(self.seg[GS].sel as u32),
            0xA9 => {
                let s = self.pop_v() as u16;
                self.load_seg(GS, s)
            }
            0xA3 | 0xAB | 0xB3 | 0xBB | 0xBA => {
                let mut rm = self.loc(op);
                let bit = if h == 0xBA {
                    op.imm
                } else {
                    let b = self.rd_reg(op.reg as usize, v);
                    if let Rm::Mem(a) = rm {
                        let bo = if v == 2 { b as u16 as i16 as i32 } else { b as i32 };
                        let byte_off = (bo >> 3) & !((v as i32) - 1);
                        rm = Rm::Mem(a.wrapping_add(byte_off as u32));
                    }
                    b
                };
                let bits = v as u32 * 8;
                let bit = bit % bits;
                let val = self.rd_rm(rm, v);
                let cf = (val >> bit) & 1 != 0;
                let kind = if h == 0xBA { op.reg as usize } else { [4, 5, 6, 7][((h >> 3) & 3) as usize] };
                let nv = match kind {
                    5 => Some(val | (1 << bit)),
                    6 => Some(val & !(1 << bit)),
                    7 => Some(val ^ (1 << bit)),
                    _ => None,
                };
                if let Some(nv) = nv {
                    self.wr_rm(rm, v, nv);
                }
                self.set_cf(cf);
            }
            0xA4 | 0xA5 | 0xAC | 0xAD => {
                let rm = self.loc(op);
                let cnt = if h & 1 == 0 { op.imm } else { self.r[ECX] } & 31;
                if cnt != 0 {
                    let a = self.rd_rm(rm, v);
                    let b = self.rd_reg(op.reg as usize, v);
                    let bits = v as u32 * 8;
                    let (r, cf) = if h < 0xAC {
                        // SHLD
                        let wide = ((a as u64) << bits) | (b as u64 & mask(v) as u64);
                        let r = (wide << cnt) >> bits;
                        (r as u32 & mask(v), (wide >> (2 * bits - cnt)) & 1 != 0)
                    } else {
                        let wide = ((b as u64 & mask(v) as u64) << bits) | a as u64;
                        let r = wide >> cnt;
                        (r as u32 & mask(v), (wide >> (cnt - 1)) & 1 != 0)
                    };
                    self.materialize();
                    let s = sbit(v);
                    self.set_szp(v, r);
                    if cf {
                        self.fl |= CF
                    } else {
                        self.fl &= !CF
                    }
                    if (a ^ r) & s != 0 {
                        self.fl |= OF
                    } else {
                        self.fl &= !OF
                    }
                    self.wr_rm(rm, v, r);
                }
            }
            0xAF => {
                let rm = self.loc(op);
                let a = self.rd_reg(op.reg as usize, v);
                let b = self.rd_rm(rm, v);
                let r = self.imul3(v, a, b);
                self.wr_reg(op.reg as usize, v, r);
            }
            0xB2 | 0xB4 | 0xB5 => {
                let rm = self.loc(op);
                let (off, sel) = self.far_ptr(rm);
                self.wr_reg(op.reg as usize, v, off);
                self.load_seg(
                    match h {
                        0xB2 => SS,
                        0xB4 => FS,
                        _ => GS,
                    },
                    sel,
                );
            }
            0xB6 | 0xB7 | 0xBE | 0xBF => {
                let rm = self.loc(op);
                let ssz = if h & 1 == 0 { 1 } else { 2 };
                let x = self.rd_rm(rm, ssz);
                let val = if h >= 0xBE { sext(x, ssz) as u32 } else { x };
                self.wr_reg(op.reg as usize, v, val & mask(v));
            }
            0xBC | 0xBD => {
                let rm = self.loc(op);
                let x = self.rd_rm(rm, v);
                if x == 0 {
                    self.set_zf(true);
                } else {
                    let i = if h == 0xBC { x.trailing_zeros() } else { 31 - x.leading_zeros() };
                    self.wr_reg(op.reg as usize, v, i);
                    self.set_zf(false);
                }
            }
            0xC8..=0xCF => {
                let r = (h & 7) as usize;
                self.r[r] = self.r[r].swap_bytes();
            }
            0xFF => return bus.hle(self, op.imm as u16),
            _ => return Err(Fault::InvalidOpcode(0x0F00 | h as u32)),
        }
        Ok(())
    }
}

#[inline(always)]
fn rep_kind(op: &Op) -> u8 {
    if op.fl & F_REP != 0 {
        0xF3
    } else if op.fl & F_REPNE != 0 {
        0xF2
    } else {
        0
    }
}

#[inline(always)]
pub(crate) fn mask(sz: u8) -> u32 {
    match sz {
        1 => 0xFF,
        2 => 0xFFFF,
        _ => 0xFFFF_FFFF,
    }
}
#[inline(always)]
pub(crate) fn sbit(sz: u8) -> u32 {
    match sz {
        1 => 0x80,
        2 => 0x8000,
        _ => 0x8000_0000,
    }
}
