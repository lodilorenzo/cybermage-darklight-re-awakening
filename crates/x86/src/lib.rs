//! 32-bit protected-mode x86 interpreter (386 + 387 subset) used to run the
//! original CyberMage code while it is progressively replaced by native Rust.
//!
//! Memory is a flat byte array; segment registers carry a base that is added
//! to every effective address (almost always 0 under DOS/4GW's flat model).

pub mod decode;
mod exec;
pub mod fpu;

pub use fpu::Fpu;

pub const EAX: usize = 0;
pub const ECX: usize = 1;
pub const EDX: usize = 2;
pub const EBX: usize = 3;
pub const ESP: usize = 4;
pub const EBP: usize = 5;
pub const ESI: usize = 6;
pub const EDI: usize = 7;

pub const ES: usize = 0;
pub const CS: usize = 1;
pub const SS: usize = 2;
pub const DS: usize = 3;
pub const FS: usize = 4;
pub const GS: usize = 5;

pub const CF: u32 = 1 << 0;
pub const PF: u32 = 1 << 2;
pub const AF: u32 = 1 << 4;
pub const ZF: u32 = 1 << 6;
pub const SF: u32 = 1 << 7;
pub const TF: u32 = 1 << 8;
pub const IF: u32 = 1 << 9;
pub const DF: u32 = 1 << 10;
pub const OF: u32 = 1 << 11;
const ARITH: u32 = CF | PF | AF | ZF | SF | OF;

/// Memory size (power of two). Accesses are masked into this range.
pub const MEM_SIZE: usize = 64 << 20;
const MEM_MASK: u32 = (MEM_SIZE - 1) as u32;

#[derive(Clone, Copy, Debug, Default)]
pub struct Seg {
    pub sel: u16,
    pub base: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Desc {
    pub base: u32,
    pub limit: u32,
    pub present: bool,
    pub code: bool,
    pub big: bool,
    pub access: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    DivideError,
    InvalidOpcode(u32),
    Halt,
    Unimplemented(&'static str),
    /// Raised by the bus to stop execution (e.g. program exit).
    Stop,
}

/// Lazy flag operations.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum Lop {
    None,
    Add,
    Adc,
    Sub,
    Sbb,
    Logic,
    Inc,
    Dec,
    Neg,
    Shl,
    Shr,
    Sar,
}

/// Hardware / OS interface supplied by the runtime.
pub trait Bus {
    fn port_in(&mut self, cpu: &mut Cpu, port: u16, size: u8) -> u32;
    fn port_out(&mut self, cpu: &mut Cpu, port: u16, size: u8, val: u32);
    /// Software interrupt (INT n). The bus may emulate it (HLE) or redirect
    /// the CPU to a protected-mode handler with `cpu.enter_interrupt`.
    fn interrupt(&mut self, cpu: &mut Cpu, n: u8) -> Result<(), Fault>;
    /// HLE trap: opcode `0F FF id16`. EIP already points past the trap.
    fn hle(&mut self, cpu: &mut Cpu, id: u16) -> Result<(), Fault>;
}

pub struct Cpu {
    pub r: [u32; 8],
    pub eip: u32,
    pub seg: [Seg; 6],
    /// Non-lazy flag storage (DF, IF, ... and arithmetic flags when lop==None)
    pub(crate) fl: u32,
    pub(crate) lop: Lop,
    pub(crate) lsz: u8,
    pub(crate) la: u32,
    pub(crate) lb: u32,
    pub(crate) lr: u32,
    pub(crate) lcf: bool,
    pub mem: Vec<u8>,
    pub fpu: Fpu,
    pub descs: Vec<Desc>,
    pub icount: u64,
    /// EIP of the instruction currently executing (for fault reports)
    pub cur_eip: u32,
    // per-instruction prefix state
    pub(crate) o16: bool,
    pub(crate) rep: u8,
    pub(crate) str_seg: usize,
    /// Write log for differential verification: (address, previous byte)
    pub wlog: Vec<(u32, u8)>,
    pub wlog_on: bool,
    /// Decoded-instruction cache for [code_lo, code_lo+code_len)
    pub icache: Vec<decode::Op>,
    pub code_lo: u32,
    pub code_len: u32,
    /// Ring buffer (power-of-two length) of recently executed EIPs for crash
    /// diagnostics; only filled when `trace_on` is set.
    pub trace: Vec<u32>,
    pub trace_on: bool,
    pub trace_pos: usize,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Cpu {
        Cpu {
            r: [0; 8],
            eip: 0,
            seg: [Seg::default(); 6],
            fl: 0x202,
            lop: Lop::None,
            lsz: 4,
            la: 0,
            lb: 0,
            lr: 0,
            lcf: false,
            mem: vec![0u8; MEM_SIZE + 16],
            fpu: Fpu::new(),
            descs: vec![Desc::default(); 8192],
            icount: 0,
            cur_eip: 0,
            o16: false,
            rep: 0,
            str_seg: DS,
            wlog: Vec::new(),
            wlog_on: false,
            icache: Vec::new(),
            code_lo: 0,
            code_len: 0,
            trace: Vec::new(),
            trace_on: false,
            trace_pos: 0,
        }
    }

    // ---------------------------------------------------------------- memory
    // Safety: every address is masked to < MEM_SIZE and the allocation has
    // 16 bytes of padding, so accesses of up to 8 bytes stay in bounds.
    #[inline(always)]
    pub fn rd8(&self, a: u32) -> u8 {
        unsafe { *self.mem.get_unchecked((a & MEM_MASK) as usize) }
    }
    #[inline(always)]
    pub fn rd16(&self, a: u32) -> u16 {
        unsafe { (self.mem.as_ptr().add((a & MEM_MASK) as usize) as *const u16).read_unaligned() }
    }
    #[inline(always)]
    pub fn rd32(&self, a: u32) -> u32 {
        unsafe { (self.mem.as_ptr().add((a & MEM_MASK) as usize) as *const u32).read_unaligned() }
    }
    #[inline(always)]
    pub fn wr8(&mut self, a: u32, v: u8) {
        self.smc_check(a, 1);
        unsafe { *self.mem.get_unchecked_mut((a & MEM_MASK) as usize) = v }
    }
    #[inline(always)]
    pub fn wr16(&mut self, a: u32, v: u16) {
        self.smc_check(a, 2);
        unsafe { (self.mem.as_mut_ptr().add((a & MEM_MASK) as usize) as *mut u16).write_unaligned(v) }
    }
    #[inline(always)]
    pub fn wr32(&mut self, a: u32, v: u32) {
        self.smc_check(a, 4);
        unsafe { (self.mem.as_mut_ptr().add((a & MEM_MASK) as usize) as *mut u32).write_unaligned(v) }
    }

    /// Invalidate cached decodes overlapping a write (self-modifying code).
    #[inline(always)]
    fn smc_check(&mut self, a: u32, n: u32) {
        if a.wrapping_sub(self.code_lo).wrapping_add(16) < self.code_len + 16 {
            self.invalidate(a, n);
        }
        if self.wlog_on {
            self.log_write(a, n);
        }
    }
    #[cold]
    fn log_write(&mut self, a: u32, n: u32) {
        for k in 0..n {
            let x = a.wrapping_add(k);
            let old = self.mem[(x & MEM_MASK) as usize];
            self.wlog.push((x & MEM_MASK, old));
        }
    }
    #[cold]
    fn invalidate(&mut self, a: u32, n: u32) {
        let lo = a.saturating_sub(15).max(self.code_lo);
        let hi = (a + n).min(self.code_lo + self.code_len);
        for x in lo..hi {
            self.icache[(x - self.code_lo) as usize].fl = 0;
        }
    }

    /// Enable the decoded-instruction cache for a code range.
    pub fn set_code_range(&mut self, lo: u32, len: u32) {
        self.code_lo = lo;
        self.code_len = len;
        self.icache = vec![decode::Op::default(); len as usize];
    }
    pub fn rd64(&self, a: u32) -> u64 {
        self.rd32(a) as u64 | ((self.rd32(a.wrapping_add(4)) as u64) << 32)
    }
    pub fn wr64(&mut self, a: u32, v: u64) {
        self.wr32(a, v as u32);
        self.wr32(a.wrapping_add(4), (v >> 32) as u32);
    }
    pub fn read_bytes(&self, a: u32, n: usize) -> &[u8] {
        let i = (a & MEM_MASK) as usize;
        &self.mem[i..i + n]
    }
    pub fn write_bytes(&mut self, a: u32, b: &[u8]) {
        if self.wlog_on {
            self.log_write(a, b.len() as u32);
        }
        let i = (a & MEM_MASK) as usize;
        self.mem[i..i + b.len()].copy_from_slice(b);
        if a < self.code_lo + self.code_len && a + b.len() as u32 + 16 > self.code_lo {
            self.invalidate(a, b.len() as u32);
        }
    }
    /// Read NUL-terminated string at linear address.
    pub fn read_cstr(&self, a: u32) -> String {
        let mut s = Vec::new();
        let mut p = a;
        loop {
            let c = self.rd8(p);
            if c == 0 || s.len() > 4096 {
                break;
            }
            s.push(c);
            p = p.wrapping_add(1);
        }
        String::from_utf8_lossy(&s).into_owned()
    }

    // ---------------------------------------------------------------- regs
    #[inline(always)]
    pub fn reg8(&self, r: usize) -> u8 {
        if r < 4 {
            self.r[r] as u8
        } else {
            (self.r[r - 4] >> 8) as u8
        }
    }
    #[inline(always)]
    pub fn set_reg8(&mut self, r: usize, v: u8) {
        if r < 4 {
            self.r[r] = (self.r[r] & !0xFF) | v as u32;
        } else {
            self.r[r - 4] = (self.r[r - 4] & !0xFF00) | ((v as u32) << 8);
        }
    }
    #[inline(always)]
    pub fn reg16(&self, r: usize) -> u16 {
        self.r[r] as u16
    }
    #[inline(always)]
    pub fn set_reg16(&mut self, r: usize, v: u16) {
        self.r[r] = (self.r[r] & !0xFFFF) | v as u32;
    }
    pub fn ax(&self) -> u16 {
        self.r[EAX] as u16
    }
    pub fn set_ax(&mut self, v: u16) {
        self.set_reg16(EAX, v)
    }

    // ---------------------------------------------------------------- segments
    pub fn load_seg(&mut self, s: usize, sel: u16) {
        let idx = (sel >> 3) as usize;
        let base = if sel & !3 == 0 { 0 } else { self.descs.get(idx).map(|d| d.base).unwrap_or(0) };
        self.seg[s] = Seg { sel, base };
    }
    pub fn desc_for(&self, sel: u16) -> Option<&Desc> {
        self.descs.get((sel >> 3) as usize)
    }

    // ---------------------------------------------------------------- stack
    #[inline(always)]
    pub fn push32(&mut self, v: u32) {
        let sp = self.r[ESP].wrapping_sub(4);
        self.r[ESP] = sp;
        let a = self.seg[SS].base.wrapping_add(sp);
        self.wr32(a, v);
    }
    #[inline(always)]
    pub fn pop32(&mut self) -> u32 {
        let sp = self.r[ESP];
        let v = self.rd32(self.seg[SS].base.wrapping_add(sp));
        self.r[ESP] = sp.wrapping_add(4);
        v
    }
    #[inline(always)]
    pub fn push16(&mut self, v: u16) {
        let sp = self.r[ESP].wrapping_sub(2);
        self.r[ESP] = sp;
        let a = self.seg[SS].base.wrapping_add(sp);
        self.wr16(a, v);
    }
    #[inline(always)]
    pub fn pop16(&mut self) -> u16 {
        let sp = self.r[ESP];
        let v = self.rd16(self.seg[SS].base.wrapping_add(sp));
        self.r[ESP] = sp.wrapping_add(2);
        v
    }
    /// Read the n-th 32-bit stack argument at [ESP + 4 + 4*n] (after a near call).
    pub fn stack_arg(&self, n: u32) -> u32 {
        self.rd32(self.seg[SS].base.wrapping_add(self.r[ESP]).wrapping_add(4 + 4 * n))
    }

    /// Push an interrupt frame and transfer to sel:off (32-bit gate).
    pub fn enter_interrupt(&mut self, sel: u16, off: u32) {
        let fl = self.flags();
        self.push32(fl);
        self.push32(self.seg[CS].sel as u32);
        self.push32(self.eip);
        self.fl &= !(IF | TF);
        self.load_seg(CS, sel);
        self.eip = off;
    }
    /// Emulate a near RET (used after HLE replacement of a function).
    pub fn ret_near(&mut self, pop_bytes: u32) {
        self.eip = self.pop32();
        self.r[ESP] = self.r[ESP].wrapping_add(pop_bytes);
    }

    // ---------------------------------------------------------------- flags
    #[inline(always)]
    fn szmask(&self) -> (u32, u32) {
        match self.lsz {
            1 => (0xFF, 0x80),
            2 => (0xFFFF, 0x8000),
            _ => (0xFFFF_FFFF, 0x8000_0000),
        }
    }
    #[inline(always)]
    pub(crate) fn set_lazy(&mut self, op: Lop, sz: u8, a: u32, b: u32, r: u32) {
        self.lop = op;
        self.lsz = sz;
        self.la = a;
        self.lb = b;
        self.lr = r;
    }
    pub fn get_cf(&self) -> bool {
        let (m, _) = self.szmask();
        let (a, b, r) = (self.la & m, self.lb & m, self.lr & m);
        match self.lop {
            Lop::None => self.fl & CF != 0,
            Lop::Add => r < a,
            Lop::Adc => {
                if self.lcf {
                    r <= a
                } else {
                    r < a
                }
            }
            Lop::Sub => a < b,
            Lop::Sbb => {
                if self.lcf {
                    a <= b
                } else {
                    a < b
                }
            }
            Lop::Logic => false,
            Lop::Inc | Lop::Dec => self.lcf,
            Lop::Neg => a != 0,
            Lop::Shl => {
                let bits = (self.lsz as u32) * 8;
                if b > bits {
                    false
                } else {
                    (a as u64) >> (bits - b) & 1 != 0
                }
            }
            Lop::Shr => (a as u64) >> (b - 1) & 1 != 0,
            Lop::Sar => {
                let sa = sext(a, self.lsz) as i64;
                (sa >> (b.min(63) - 1)) & 1 != 0
            }
        }
    }
    pub fn get_zf(&self) -> bool {
        if self.lop == Lop::None {
            return self.fl & ZF != 0;
        }
        let (m, _) = self.szmask();
        self.lr & m == 0
    }
    pub fn get_sf(&self) -> bool {
        if self.lop == Lop::None {
            return self.fl & SF != 0;
        }
        let (_, s) = self.szmask();
        self.lr & s != 0
    }
    pub fn get_pf(&self) -> bool {
        if self.lop == Lop::None {
            return self.fl & PF != 0;
        }
        (self.lr as u8).count_ones() % 2 == 0
    }
    pub fn get_af(&self) -> bool {
        match self.lop {
            Lop::None => self.fl & AF != 0,
            Lop::Add | Lop::Adc | Lop::Sub | Lop::Sbb => (self.la ^ self.lb ^ self.lr) & 0x10 != 0,
            Lop::Inc => self.lr & 0xF == 0,
            Lop::Dec => self.lr & 0xF == 0xF,
            Lop::Neg => self.lr & 0xF != 0,
            _ => false,
        }
    }
    pub fn get_of(&self) -> bool {
        let (m, s) = self.szmask();
        let (a, b, r) = (self.la & m, self.lb & m, self.lr & m);
        match self.lop {
            Lop::None => self.fl & OF != 0,
            Lop::Add | Lop::Adc => (a ^ r) & (b ^ r) & s != 0,
            Lop::Sub | Lop::Sbb => (a ^ b) & (a ^ r) & s != 0,
            Lop::Logic => false,
            Lop::Inc => r == s,
            Lop::Dec => r == s - 1,
            Lop::Neg => r == s,
            Lop::Shl => ((r & s) != 0) != self.get_cf(),
            Lop::Shr => b == 1 && a & s != 0,
            Lop::Sar => false,
        }
    }
    /// Materialize full EFLAGS.
    pub fn flags(&self) -> u32 {
        if self.lop == Lop::None {
            return self.fl | 2;
        }
        let mut f = self.fl & !ARITH;
        if self.get_cf() {
            f |= CF;
        }
        if self.get_pf() {
            f |= PF;
        }
        if self.get_af() {
            f |= AF;
        }
        if self.get_zf() {
            f |= ZF;
        }
        if self.get_sf() {
            f |= SF;
        }
        if self.get_of() {
            f |= OF;
        }
        f | 2
    }
    pub fn set_flags(&mut self, f: u32) {
        self.fl = (f & 0x003F_7FD5) | 2;
        self.lop = Lop::None;
    }
    #[inline(always)]
    pub(crate) fn materialize(&mut self) {
        if self.lop != Lop::None {
            self.fl = self.flags();
            self.lop = Lop::None;
        }
    }
    pub fn set_cf(&mut self, v: bool) {
        self.materialize();
        if v {
            self.fl |= CF
        } else {
            self.fl &= !CF
        }
    }
    pub fn set_zf(&mut self, v: bool) {
        self.materialize();
        if v {
            self.fl |= ZF
        } else {
            self.fl &= !ZF
        }
    }
    #[inline(always)]
    pub fn df(&self) -> bool {
        self.fl & DF != 0
    }
    pub fn iflag(&self) -> bool {
        self.fl & IF != 0
    }

    pub fn cond(&self, cc: u8) -> bool {
        let r = match cc >> 1 {
            0 => self.get_of(),
            1 => self.get_cf(),
            2 => self.get_zf(),
            3 => self.get_cf() || self.get_zf(),
            4 => self.get_sf(),
            5 => self.get_pf(),
            6 => self.get_sf() != self.get_of(),
            _ => self.get_zf() || (self.get_sf() != self.get_of()),
        };
        r != (cc & 1 != 0)
    }

    pub fn dump(&self) -> String {
        format!(
            "EAX={:08X} EBX={:08X} ECX={:08X} EDX={:08X} ESI={:08X} EDI={:08X} EBP={:08X} ESP={:08X} EIP={:08X} FL={:08X} CS={:04X} DS={:04X} ES={:04X} SS={:04X} FS={:04X} GS={:04X}",
            self.r[EAX], self.r[EBX], self.r[ECX], self.r[EDX], self.r[ESI], self.r[EDI], self.r[EBP], self.r[ESP],
            self.cur_eip, self.flags(), self.seg[CS].sel, self.seg[DS].sel, self.seg[ES].sel, self.seg[SS].sel,
            self.seg[FS].sel, self.seg[GS].sel
        )
    }

    /// Run up to `n` instructions.
    /// Enable the EIP history ring buffer (n rounded up to a power of two).
    pub fn enable_trace(&mut self, n: usize) {
        self.trace = vec![0; n.next_power_of_two()];
        self.trace_on = true;
    }

    /// Recently executed EIPs, oldest first.
    pub fn trace_history(&self) -> Vec<u32> {
        let n = self.trace.len();
        (0..n).map(|i| self.trace[(self.trace_pos + i) & (n - 1)]).collect()
    }

    pub fn run(&mut self, bus: &mut dyn Bus, n: u64) -> Result<(), Fault> {
        for _ in 0..n {
            self.step(bus)?;
        }
        Ok(())
    }
}

#[inline(always)]
pub(crate) fn sext(v: u32, sz: u8) -> i32 {
    match sz {
        1 => v as u8 as i8 as i32,
        2 => v as u16 as i16 as i32,
        _ => v as i32,
    }
}
