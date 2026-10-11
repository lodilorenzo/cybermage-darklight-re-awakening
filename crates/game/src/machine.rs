//! The emulated DOS/4GW machine: memory layout, selectors, interrupts,
//! ports and the HLE (high level emulation) of DOS, DPMI and BIOS services.

use crate::le;
use crate::vfs::Vfs;
use crate::vga::Vga;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use x86::*;

// ---- memory layout (linear) ------------------------------------------------
pub const PSP_LIN: u32 = 0x2000;
pub const ENV_LIN: u32 = 0x3000;
pub const DOS_HEAP_START: u32 = 0x10000; // conventional memory handed out by DPMI 0100h
pub const DOS_HEAP_END: u32 = 0x9F000;
pub const STUB_BASE: u32 = 0xF0000; // HLE trap stubs (BIOS ROM area)
pub const HEAP_START: u32 = 0x0080_0000; // DPMI 0501h memory
pub const HEAP_END: u32 = MEM_SIZE as u32 - 0x10000;

// ---- selectors -------------------------------------------------------------
pub const SEL_CODE: u16 = 0x08;
pub const SEL_DATA: u16 = 0x10;
pub const SEL_PSP: u16 = 0x18;
pub const SEL_ENV: u16 = 0x20;
const FIRST_DYN_SEL: usize = 8;

// ---- HLE trap ids ----------------------------------------------------------
/// Default handler for interrupt vector n: id = n (stub ends with IRET).
pub const TRAP_RETURN: u16 = 0x100; // return-to-host sentinel for guest calls
pub const TRAP_EVENT_DONE: u16 = 0x110; // + event index: timer event callback finished
pub const TRAP_HOOK_BASE: u16 = 0x200;
const EVENT_STUBS: u32 = STUB_BASE + 0x900;
const EVENT_SLOTS: u32 = STUB_BASE + 0xC00;
pub const MAX_EVENTS: usize = 16;

/// Periodic callback into guest code (replacement for HMI sosTIMERRegisterEvent).
#[derive(Clone, Copy, Debug)]
pub struct TimerEvent {
    pub hz: f64,
    pub addr: u32,
    pub accum: f64,
    pub busy: bool,
    pub active: bool,
}

pub type HookFn = fn(&mut Machine, &mut Cpu) -> Result<(), Fault>;

pub struct Hook {
    pub name: &'static str,
    pub link_addr: u32,
    pub f: HookFn,
    pub orig: [u8; 4],
    /// Number of register arguments (EAX, EDX, EBX, ECX): these may be clobbered.
    pub reg_args: u8,
    /// Function returns a value in EAX.
    pub returns: bool,
}

pub struct DosFile {
    pub f: File,
    pub name: String,
    /// Substitutions applied on read; see `rebrand`.
    pub patch: Option<crate::rebrand::Patch>,
}

#[derive(Default)]
pub struct FindState {
    pub entries: Vec<(String, u64, bool)>,
    pub pos: usize,
}

pub struct Mouse {
    pub x: i32,
    pub y: i32,
    pub buttons: u16,
    pub min_x: i32,
    pub max_x: i32,
    pub min_y: i32,
    pub max_y: i32,
    pub mickey_x: i32,
    pub mickey_y: i32,
    pub press_count: [u16; 3],
    pub release_count: [u16; 3],
}

pub struct Machine {
    pub vfs: Vfs,
    pub files: Vec<Option<DosFile>>,
    pub find: FindState,
    pub dta: u32,
    pub dos_next: u32,
    pub dos_blocks: Vec<(u32, u32)>, // (linear, size)
    pub heap_next: u32,
    pub heap_blocks: Vec<(u32, u32)>, // (linear, size) handle = index+1
    pub pm_vec: [(u16, u32); 256],
    pub rm_vec: [u32; 256],
    pub vga: Vga,
    pub display: crate::display::Display,
    pub gamepad: crate::gamepad::Pad,
    pub desktop: crate::desktop::Input,
    pub pic_mask: u8,
    pub pic_isr: u8,
    pub pit_div: u32,
    pub pit_mode_latch: u8,
    pub pit_lohi: bool,
    pub pit_chan0_write_lo: Option<u8>,
    pub kbd_queue: VecDeque<u8>,
    pub port60: u8,
    pub kbd_bios: VecDeque<u16>,
    pub mouse: Mouse,
    pub hooks: Vec<Hook>,
    pub exit_code: Option<i32>,
    pub log_unknown: bool,
    pub trace_ints: bool,
    pub trace_all_ints: bool,
    pub bios_ticks_frac: f64,
    /// Time in seconds of emulation (driven by the host).
    pub time: f64,
    pub irq0_accum: f64,
    pub spawn_request: Option<(String, Vec<String>)>,
    pub stdout: String,
    pub events: Vec<TimerEvent>,
    pub audio: crate::audio::Audio,
    pub verifier: Option<crate::verify::Verifier>,
}

impl Machine {
    pub fn new(vfs: Vfs) -> Machine {
        let display = crate::display::Display::new(&vfs.overlay.join("NATIVE.CNF"));
        Machine {
            vfs,
            files: (0..64).map(|_| None).collect(),
            find: FindState::default(),
            dta: PSP_LIN + 0x80,
            dos_next: DOS_HEAP_START,
            dos_blocks: Vec::new(),
            heap_next: HEAP_START,
            heap_blocks: Vec::new(),
            pm_vec: [(0, 0); 256],
            rm_vec: [0; 256],
            vga: Vga::new(),
            display,
            gamepad: crate::gamepad::Pad::default(),
            desktop: crate::desktop::Input::default(),
            pic_mask: 0,
            pic_isr: 0,
            pit_div: 65536,
            pit_mode_latch: 0,
            pit_lohi: false,
            pit_chan0_write_lo: None,
            kbd_queue: VecDeque::new(),
            port60: 0,
            kbd_bios: VecDeque::new(),
            mouse: Mouse {
                x: 320,
                y: 240,
                buttons: 0,
                min_x: 0,
                max_x: 639,
                min_y: 0,
                max_y: 479,
                mickey_x: 0,
                mickey_y: 0,
                press_count: [0; 3],
                release_count: [0; 3],
            },
            hooks: Vec::new(),
            exit_code: None,
            log_unknown: true,
            trace_ints: std::env::var("CM_TRACE_IRQ").is_ok(),
            trace_all_ints: std::env::var("CM_TRACE_INTS").is_ok(),
            bios_ticks_frac: 0.0,
            time: 0.0,
            irq0_accum: 0.0,
            spawn_request: None,
            stdout: String::new(),
            events: Vec::new(),
            audio: crate::audio::Audio::new(),
            verifier: crate::verify::Verifier::from_env(),
        }
    }

    /// Register a periodic guest callback; returns its handle.
    pub fn add_timer_event(&mut self, cpu: &mut Cpu, hz: f64, addr: u32) -> usize {
        let k = self.events.iter().position(|e| !e.active).unwrap_or(self.events.len());
        assert!(k < MAX_EVENTS);
        let ev = TimerEvent { hz, addr, accum: 0.0, busy: false, active: true };
        if k == self.events.len() {
            self.events.push(ev);
        } else {
            self.events[k] = ev;
        }
        // stub: call dword [slot] ; trap EVENT_DONE+k ; iretd
        let slot = EVENT_SLOTS + k as u32 * 4;
        cpu.wr32(slot, addr);
        let s = EVENT_STUBS + k as u32 * 16;
        let id = TRAP_EVENT_DONE + k as u16;
        let mut code = vec![0xFF, 0x15];
        code.extend_from_slice(&slot.to_le_bytes());
        code.extend_from_slice(&[0x0F, 0xFF, id as u8, (id >> 8) as u8, 0xCF]);
        cpu.write_bytes(s, &code);
        k
    }

    /// Fire due timer events. `dt` is elapsed emulated seconds.
    pub fn service_events(&mut self, cpu: &mut Cpu, dt: f64) {
        for k in 0..self.events.len() {
            let e = &mut self.events[k];
            if !e.active {
                continue;
            }
            e.accum = (e.accum + dt * e.hz).min(4.0);
            if e.accum >= 1.0 && !e.busy && cpu.iflag() {
                e.accum -= 1.0;
                e.busy = true;
                cpu.enter_interrupt(SEL_CODE, EVENT_STUBS + k as u32 * 16);
                return; // one at a time
            }
        }
    }

    // ------------------------------------------------------------- setup
    pub fn load_program(&mut self, cpu: &mut Cpu, exe: &[u8], args: &str) -> Result<(), String> {
        let img = le::load(exe)?;
        for (base, data) in &img.data {
            cpu.write_bytes(*base, data);
        }
        // decoded-instruction cache over the code object
        let code = &img.objects[0];
        cpu.set_code_range(code.base, code.vsize);
        // descriptors
        let flat = Desc { base: 0, limit: 0xFFFF_FFFF, present: true, code: false, big: true, access: 0x93 };
        cpu.descs[(SEL_CODE >> 3) as usize] = Desc { code: true, access: 0x9B, ..flat };
        cpu.descs[(SEL_DATA >> 3) as usize] = flat;
        cpu.descs[(SEL_PSP >> 3) as usize] = Desc { base: PSP_LIN, limit: 0xFF, ..flat };
        cpu.descs[(SEL_ENV >> 3) as usize] = Desc { base: ENV_LIN, limit: 0xFFF, ..flat };
        // PSP
        cpu.wr16(PSP_LIN, 0x20CD);
        cpu.wr16(PSP_LIN + 2, 0x9FFF);
        cpu.wr16(PSP_LIN + 0x2C, SEL_ENV);
        // DOS/4GW stores the top of the program's memory at PSP:5C
        cpu.wr32(PSP_LIN + 0x5C, img.stack);
        let tail = format!(" {}", args);
        let tail = if args.is_empty() { String::new() } else { tail };
        cpu.wr8(PSP_LIN + 0x80, tail.len() as u8);
        cpu.write_bytes(PSP_LIN + 0x81, tail.as_bytes());
        cpu.wr8(PSP_LIN + 0x81 + tail.len() as u32, 0x0D);
        // environment
        let env = b"PATH=C:\\\0COMSPEC=C:\\COMMAND.COM\0DOS4G=QUIET\0\0\x01\0C:\\CYBERMAG\\CM_HI.EXE\0";
        cpu.write_bytes(ENV_LIN, env);
        // BIOS data area
        cpu.wr16(0x463, 0x3D4);
        cpu.wr8(0x449, 3);
        cpu.wr16(0x44A, 80);
        cpu.wr8(0x484, 24);
        // interrupt stubs: for each vector n, `0F FF n 00 ; IRET`
        for n in 0..256u32 {
            let a = STUB_BASE + n * 8;
            cpu.write_bytes(a, &[0x0F, 0xFF, n as u8, 0, 0xCF]);
            self.pm_vec[n as usize] = (SEL_CODE, a);
            self.rm_vec[n as usize] = (0xF000 << 16) | (n * 8);
        }
        // return sentinel
        let a = STUB_BASE + 0x800;
        cpu.write_bytes(a, &[0x0F, 0xFF, (TRAP_RETURN & 0xFF) as u8, (TRAP_RETURN >> 8) as u8, 0xF4]);
        // registers per DOS/4GW
        cpu.load_seg(CS, SEL_CODE);
        cpu.load_seg(DS, SEL_DATA);
        cpu.load_seg(SS, SEL_DATA);
        cpu.load_seg(ES, SEL_PSP);
        cpu.load_seg(FS, 0);
        cpu.load_seg(GS, 0);
        cpu.eip = img.entry;
        cpu.r[ESP] = img.stack;
        cpu.set_flags(0x202);
        Ok(())
    }

    pub fn is_default_vec(&self, n: u8) -> bool {
        let (sel, off) = self.pm_vec[n as usize];
        sel == SEL_CODE && off == STUB_BASE + n as u32 * 8
    }

    // ------------------------------------------------------------- hooks
    /// Replace the guest function at `link_addr` with a native implementation.
    pub fn hook(&mut self, cpu: &mut Cpu, link_addr: u32, name: &'static str, f: HookFn) {
        self.hook_sig(cpu, link_addr, name, 4, true, f);
    }

    /// Hook with a signature: `reg_args` register arguments, `returns` a value.
    pub fn hook_sig(&mut self, cpu: &mut Cpu, link_addr: u32, name: &'static str, reg_args: u8, returns: bool, f: HookFn) {
        // CM_NO_HOOK=Name,Name keeps the original guest code for those functions
        if let Ok(v) = std::env::var("CM_NO_HOOK") {
            if v.split(',').any(|n| n.trim() == name) {
                return;
            }
        }
        let id = TRAP_HOOK_BASE + self.hooks.len() as u16;
        let a = le::rt(link_addr);
        let mut orig = [0u8; 4];
        orig.copy_from_slice(cpu.read_bytes(a, 4));
        cpu.write_bytes(a, &[0x0F, 0xFF, id as u8, (id >> 8) as u8]);
        self.hooks.push(Hook { name, link_addr, f, orig, reg_args, returns });
    }

    // ------------------------------------------------------------- selectors
    pub fn alloc_sel(&mut self, cpu: &mut Cpu, count: usize) -> Option<u16> {
        let mut i = FIRST_DYN_SEL;
        'outer: while i + count <= cpu.descs.len() {
            for k in 0..count {
                if cpu.descs[i + k].present {
                    i += k + 1;
                    continue 'outer;
                }
            }
            for k in 0..count {
                cpu.descs[i + k] = Desc { base: 0, limit: 0, present: true, code: false, big: true, access: 0x93 };
            }
            return Some((i << 3) as u16 | 0); // RPL 0 / GDT
        }
        None
    }

    // ------------------------------------------------------------- IRQs
    /// Deliver hardware IRQ `irq` (0..7). Returns true if delivered.
    pub fn raise_irq(&mut self, cpu: &mut Cpu, irq: u8) -> bool {
        if !cpu.iflag() || self.pic_isr & (1 << irq) != 0 || self.pic_mask & (1 << irq) != 0 {
            return false;
        }
        let vec = 8 + irq;
        if self.is_default_vec(vec) {
            // Handle in host without entering the guest
            self.default_irq(cpu, irq);
            return true;
        }
        self.pic_isr |= 1 << irq;
        if irq == 1 && self.trace_ints {
            eprintln!("[irq1] sc={:02X} at {:06X}", self.port60, le::link(cpu.eip));
        }
        let (sel, off) = self.pm_vec[vec as usize];
        cpu.enter_interrupt(sel, off);
        true
    }

    fn default_irq(&mut self, cpu: &mut Cpu, irq: u8) {
        // the BIOS handler acknowledges the PIC
        self.pic_isr &= !(1 << irq);
        match irq {
            0 => {
                let t = cpu.rd32(0x46C).wrapping_add(1);
                cpu.wr32(0x46C, t);
            }
            1 => {
                // BIOS keyboard handler: translate the latched scancode into the BIOS buffer
                let sc = self.port60;
                if sc & 0x80 == 0 && sc != 0xE0 {
                    let ascii = crate::input::scancode_ascii(sc);
                    self.kbd_bios.push_back(((sc as u16) << 8) | ascii as u16);
                }
            }
            _ => {}
        }
    }

    pub fn pit_hz(&self) -> f64 {
        1193182.0 / self.pit_div as f64
    }

    // ------------------------------------------------------------- guest calls
    /// Call a guest function at runtime address `addr` with Watcom register
    /// arguments, running the CPU until it returns. Used by native code.
    pub fn call_guest(&mut self, cpu: &mut Cpu, addr: u32, args: &[u32]) -> Result<u32, Fault> {
        let saved = (cpu.r, cpu.eip);
        let regs = [EAX, EDX, EBX, ECX];
        for (i, &a) in args.iter().enumerate().take(4) {
            cpu.r[regs[i]] = a;
        }
        for &a in args.iter().skip(4).rev() {
            cpu.push32(a);
        }
        cpu.push32(STUB_BASE + 0x800);
        cpu.eip = addr;
        let depth_sp = cpu.r[ESP];
        loop {
            match cpu.step(self) {
                Ok(()) => {}
                Err(Fault::Stop) if cpu.r[ESP] >= depth_sp => break,
                Err(e) => return Err(e),
            }
        }
        let ret = cpu.r[EAX];
        let mut r = saved.0;
        r[EAX] = ret;
        r[ESP] = saved.0[ESP];
        cpu.r = r;
        cpu.eip = saved.1;
        Ok(ret)
    }

    fn unknown(&self, what: &str, cpu: &Cpu) {
        if self.log_unknown {
            eprintln!("[hle] unhandled {} at {:06X} {}", what, le::link(cpu.cur_eip), cpu.dump());
        }
    }

    pub fn print(&mut self, s: &str) {
        self.stdout.push_str(s);
        eprint!("{}", s);
    }
}

// ============================================================================
// Register view for HLE services shared between protected-mode INT and
// DPMI 0300h (simulate real-mode interrupt) calls.
// ============================================================================
#[derive(Clone, Copy, Default, Debug)]
pub struct Regs {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
    pub esi: u32,
    pub edi: u32,
    pub ebp: u32,
    pub ds: u16,
    pub es: u16,
    pub flags: u32,
    /// Real-mode semantics: segment registers are paragraphs.
    pub rm: bool,
}

impl Regs {
    pub fn from_cpu(cpu: &Cpu) -> Regs {
        Regs {
            eax: cpu.r[EAX],
            ebx: cpu.r[EBX],
            ecx: cpu.r[ECX],
            edx: cpu.r[EDX],
            esi: cpu.r[ESI],
            edi: cpu.r[EDI],
            ebp: cpu.r[EBP],
            ds: cpu.seg[DS].sel,
            es: cpu.seg[ES].sel,
            flags: cpu.flags(),
            rm: false,
        }
    }
    pub fn to_cpu(&self, cpu: &mut Cpu) {
        cpu.r[EAX] = self.eax;
        cpu.r[EBX] = self.ebx;
        cpu.r[ECX] = self.ecx;
        cpu.r[EDX] = self.edx;
        cpu.r[ESI] = self.esi;
        cpu.r[EDI] = self.edi;
        cpu.r[EBP] = self.ebp;
        if cpu.seg[ES].sel != self.es {
            cpu.load_seg(ES, self.es);
        }
        if cpu.seg[DS].sel != self.ds {
            cpu.load_seg(DS, self.ds);
        }
        let f = cpu.flags();
        cpu.set_flags((f & !(CF | ZF)) | (self.flags & (CF | ZF)));
    }
    pub fn ah(&self) -> u8 {
        (self.eax >> 8) as u8
    }
    pub fn al(&self) -> u8 {
        self.eax as u8
    }
    pub fn ax(&self) -> u16 {
        self.eax as u16
    }
    pub fn bx(&self) -> u16 {
        self.ebx as u16
    }
    pub fn cx(&self) -> u16 {
        self.ecx as u16
    }
    pub fn dx(&self) -> u16 {
        self.edx as u16
    }
    pub fn set_ax(&mut self, v: u16) {
        self.eax = (self.eax & !0xFFFF) | v as u32
    }
    pub fn set_al(&mut self, v: u8) {
        self.eax = (self.eax & !0xFF) | v as u32
    }
    pub fn set_ah(&mut self, v: u8) {
        self.eax = (self.eax & !0xFF00) | ((v as u32) << 8)
    }
    pub fn set_bx(&mut self, v: u16) {
        self.ebx = (self.ebx & !0xFFFF) | v as u32
    }
    pub fn set_cx(&mut self, v: u16) {
        self.ecx = (self.ecx & !0xFFFF) | v as u32
    }
    pub fn set_dx(&mut self, v: u16) {
        self.edx = (self.edx & !0xFFFF) | v as u32
    }
    pub fn set_si(&mut self, v: u16) {
        self.esi = (self.esi & !0xFFFF) | v as u32
    }
    pub fn set_di(&mut self, v: u16) {
        self.edi = (self.edi & !0xFFFF) | v as u32
    }
    pub fn cf(&mut self, v: bool) {
        if v {
            self.flags |= CF
        } else {
            self.flags &= !CF
        }
    }
    pub fn zf(&mut self, v: bool) {
        if v {
            self.flags |= ZF
        } else {
            self.flags &= !ZF
        }
    }
    /// Linear address of seg:off, where off is 32-bit in PM, 16-bit in RM.
    pub fn ptr(&self, cpu: &Cpu, seg: u16, off: u32) -> u32 {
        if self.rm {
            ((seg as u32) << 4) + (off & 0xFFFF)
        } else {
            let base = if seg & !3 == 0 { 0 } else { cpu.descs[(seg >> 3) as usize].base };
            base.wrapping_add(off)
        }
    }
    pub fn ds_dx(&self, cpu: &Cpu) -> u32 {
        self.ptr(cpu, self.ds, self.edx)
    }
    pub fn es_di(&self, cpu: &Cpu) -> u32 {
        self.ptr(cpu, self.es, self.edi)
    }
    pub fn ds_si(&self, cpu: &Cpu) -> u32 {
        self.ptr(cpu, self.ds, self.esi)
    }
    pub fn es_dx(&self, cpu: &Cpu) -> u32 {
        self.ptr(cpu, self.es, self.edx)
    }
    pub fn count(&self) -> u32 {
        if self.rm {
            self.ecx & 0xFFFF
        } else {
            self.ecx
        }
    }
}

// ============================================================================
// Bus implementation
// ============================================================================
impl Bus for Machine {
    fn port_in(&mut self, cpu: &mut Cpu, port: u16, size: u8) -> u32 {
        let _ = cpu;
        let v = match port {
            0x20 => 0,
            0x21 => self.pic_mask as u32,
            0x40 => {
                // PIT counter read (approximation from emulated time)
                let t = (self.time * 1193182.0) as u64 % self.pit_div as u64;
                (self.pit_div as u64 - t) as u32 & 0xFF
            }
            0x60 => self.port60 as u32,
            0x61 => 0x20,
            0x64 => {
                if self.kbd_queue.is_empty() {
                    0x1C
                } else {
                    0x1D
                }
            }
            0x201 => 0xF0, // no joystick
            0x3B0..=0x3DF => self.vga.port_in(port) as u32,
            _ => 0xFF,
        };
        match size {
            1 => v & 0xFF,
            2 => v & 0xFFFF,
            _ => v,
        }
    }

    fn port_out(&mut self, cpu: &mut Cpu, port: u16, size: u8, val: u32) {
        let _ = cpu;
        match port {
            0x20 => {
                if val & 0xFF == 0x20 {
                    // non-specific EOI: clear highest-priority in-service IRQ
                    for i in 0..8 {
                        if self.pic_isr & (1 << i) != 0 {
                            self.pic_isr &= !(1 << i);
                            break;
                        }
                    }
                }
            }
            0x21 => self.pic_mask = val as u8,
            0x43 => {
                self.pit_mode_latch = val as u8;
                if (val >> 6) & 3 == 0 && (val >> 4) & 3 == 3 {
                    self.pit_chan0_write_lo = None;
                    self.pit_lohi = true;
                }
            }
            0x40 => {
                if let Some(lo) = self.pit_chan0_write_lo.take() {
                    let d = ((val as u32 & 0xFF) << 8) | lo as u32;
                    self.pit_div = if d == 0 { 65536 } else { d };
                } else {
                    self.pit_chan0_write_lo = Some(val as u8);
                }
            }
            0x3B0..=0x3DF => {
                if size == 2 {
                    self.vga.port_out(port, val as u8);
                    self.vga.port_out(port + 1, (val >> 8) as u8);
                } else {
                    self.vga.port_out(port, val as u8);
                }
            }
            _ => {}
        }
    }

    fn interrupt(&mut self, cpu: &mut Cpu, n: u8) -> Result<(), Fault> {
        if !self.is_default_vec(n) && n != 0x21 && n != 0x31 && n != 0x10 && n != 0x33 {
            let (sel, off) = self.pm_vec[n as usize];
            cpu.enter_interrupt(sel, off);
            return Ok(());
        }
        self.hle_int(cpu, n)
    }

    fn hle(&mut self, cpu: &mut Cpu, id: u16) -> Result<(), Fault> {
        if id < 0x100 {
            // default handler for vector `id` reached via guest chaining / IRQ entry
            let n = id as u8;
            if (8..16).contains(&n) {
                self.default_irq(cpu, n - 8);
                // an IRET follows the trap
                return Ok(());
            }
            return self.hle_int(cpu, n);
        }
        if id == TRAP_RETURN {
            return Err(Fault::Stop);
        }
        if (TRAP_EVENT_DONE..TRAP_EVENT_DONE + MAX_EVENTS as u16).contains(&id) {
            let k = (id - TRAP_EVENT_DONE) as usize;
            if let Some(e) = self.events.get_mut(k) {
                e.busy = false;
            }
            return Ok(());
        }
        let idx = (id - TRAP_HOOK_BASE) as usize;
        if idx < self.hooks.len() {
            if self.verifier.as_ref().map(|v| v.wants(self.hooks[idx].name)).unwrap_or(false) {
                return crate::verify::run(self, cpu, idx);
            }
            let f = self.hooks[idx].f;
            return f(self, cpu);
        }
        Err(Fault::InvalidOpcode(0x0FFF))
    }
}

impl Machine {
    fn hle_int(&mut self, cpu: &mut Cpu, n: u8) -> Result<(), Fault> {
        if self.trace_all_ints && !(n == 0x21 && cpu.r[EAX] & 0xFF00 == 0x2C00) {
            eprintln!("[int {:02X}] at {:06X} EAX={:08X}", n, le::link(cpu.cur_eip), cpu.r[EAX]);
        }
        match n {
            0x21 => {
                let mut r = Regs::from_cpu(cpu);
                self.int21(cpu, &mut r)?;
                r.to_cpu(cpu);
            }
            0x31 => self.int31(cpu)?,
            0x10 | 0x16 | 0x33 | 0x1A | 0x15 | 0x2F | 0x11 | 0x12 => {
                let mut r = Regs::from_cpu(cpu);
                self.bios_int(cpu, n, &mut r);
                r.to_cpu(cpu);
            }
            0x03 => {}
            0x1C | 0x23 | 0x24 => {}
            _ => self.unknown(&format!("int {:02X}", n), cpu),
        }
        Ok(())
    }

    // ---------------------------------------------------------------- DOS
    pub fn int21(&mut self, cpu: &mut Cpu, r: &mut Regs) -> Result<(), Fault> {
        r.cf(false);
        match r.ah() {
            0x02 => {
                let c = r.dx() as u8 as char;
                self.print(&c.to_string());
            }
            0x06 => {
                if r.dx() as u8 == 0xFF {
                    if let Some(k) = self.kbd_bios.pop_front() {
                        r.set_al(k as u8);
                        r.zf(false);
                    } else {
                        r.set_al(0);
                        r.zf(true);
                    }
                } else {
                    let c = r.dx() as u8 as char;
                    self.print(&c.to_string());
                }
            }
            0x07 | 0x08 => {
                let k = self.kbd_bios.pop_front().unwrap_or(0x1C0D);
                r.set_al(k as u8);
            }
            0x09 => {
                let mut a = r.ds_dx(cpu);
                let mut s = String::new();
                loop {
                    let c = cpu.rd8(a);
                    if c == b'$' || s.len() > 2000 {
                        break;
                    }
                    s.push(c as char);
                    a += 1;
                }
                self.print(&s);
            }
            0x0B => r.set_al(if self.kbd_bios.is_empty() { 0 } else { 0xFF }),
            0x0E => r.set_al(26),
            0x19 => r.set_al(2), // C:
            0x1A => self.dta = r.ds_dx(cpu),
            0x2F => {
                // DTA as ES:EBX (flat)
                r.es = SEL_DATA;
                r.ebx = self.dta;
            }
            0x25 => {
                let n = r.al();
                self.pm_vec[n as usize] = (r.ds, r.edx);
                if n == 9 {
                    // On real hardware the release of the Enter key used to start the
                    // game arrives after CM_ISR9 is installed; it primes the ISR's
                    // key-state latch (otherwise the first key press is swallowed).
                    self.kbd_queue.push_back(0x9C);
                }
            }
            0x35 => {
                let n = r.al();
                let (sel, off) = self.pm_vec[n as usize];
                r.es = sel;
                r.ebx = off;
            }
            0x2A => {
                r.set_cx(1995);
                r.set_dx((11 << 8) | 29);
                r.set_al(3);
            }
            0x2C => {
                let t = self.time;
                let h = (t / 3600.0) as u32 % 24;
                let m = (t / 60.0) as u32 % 60;
                let s = t as u32 % 60;
                let cs = ((t.fract()) * 100.0) as u32;
                r.set_cx(((h << 8) | m) as u16);
                r.set_dx(((s << 8) | cs) as u16);
            }
            0x30 => {
                r.set_ax(0x0006); // DOS 6.0
                r.ebx &= !0xFFFF;
            }
            0x33 => r.set_dx(0),
            0x36 => {
                // free disk space
                r.set_ax(64);
                r.set_bx(0x7FFF);
                r.set_cx(512);
                r.set_dx(0xFFFF);
            }
            0x3B => {}
            0x3C | 0x5B => {
                let name = cpu.read_cstr(r.ds_dx(cpu));
                let p = self.vfs.resolve_write(&name);
                match File::create(&p) {
                    Ok(f) => {
                        let h = self.alloc_handle(DosFile { f, name: name.clone(), patch: None });
                        r.set_ax(h);
                    }
                    Err(_) => {
                        r.set_ax(3);
                        r.cf(true)
                    }
                }
            }
            0x3D => {
                let name = cpu.read_cstr(r.ds_dx(cpu));
                let mode = r.al() & 3;
                let res = if mode == 0 {
                    self.vfs.resolve_read(&name).and_then(|p| File::open(p).ok())
                } else {
                    // read/write: copy-on-write into overlay
                    let w = self.vfs.resolve_write(&name);
                    if !w.exists() {
                        if let Some(src) = self.vfs.resolve_read(&name) {
                            std::fs::copy(src, &w).ok();
                        }
                    }
                    std::fs::OpenOptions::new().read(true).write(true).open(&w).ok()
                };
                match res {
                    Some(mut f) => {
                        let patch = if mode == 0 { crate::rebrand::title_patch(&name, &mut f) } else { None };
                        let h = self.alloc_handle(DosFile { f, name: name.clone(), patch });
                        if self.trace_ints {
                            eprintln!("[dos] open {} -> {}", name, h);
                        }
                        r.set_ax(h);
                    }
                    None => {
                        if self.log_unknown {
                            eprintln!("[dos] open failed: {}", name);
                        }
                        r.set_ax(2);
                        r.cf(true)
                    }
                }
            }
            0x3E => {
                let h = r.bx() as usize;
                if h >= 5 && h < self.files.len() {
                    self.files[h] = None;
                }
            }
            0x3F => {
                let h = r.bx() as usize;
                let n = r.count() as usize;
                let dst = r.ds_dx(cpu);
                if let Some(Some(df)) = self.files.get_mut(h) {
                    let pos = df.f.stream_position().unwrap_or(0);
                    let mut buf = vec![0u8; n];
                    let mut got = 0;
                    while got < n {
                        match df.f.read(&mut buf[got..]) {
                            Ok(0) => break,
                            Ok(k) => got += k,
                            Err(_) => break,
                        }
                    }
                    if let Some(p) = &df.patch { crate::rebrand::apply(p, cpu, pos, &mut buf[..got]); }
                    cpu.write_bytes(dst, &buf[..got]);
                    r.eax = got as u32;
                } else if h == 0 {
                    r.eax = 0;
                } else {
                    r.set_ax(6);
                    r.cf(true);
                }
            }
            0x40 => {
                let h = r.bx() as usize;
                let n = r.count() as usize;
                let src = r.ds_dx(cpu);
                let data = cpu.read_bytes(src, n).to_vec();
                if h == 1 || h == 2 {
                    self.print(&String::from_utf8_lossy(&data));
                    r.eax = n as u32;
                } else if let Some(Some(df)) = self.files.get_mut(h) {
                    if n == 0 {
                        // truncate at current position
                        let pos = df.f.stream_position().unwrap_or(0);
                        df.f.set_len(pos).ok();
                    } else {
                        df.f.write_all(&data).ok();
                    }
                    r.eax = n as u32;
                } else {
                    r.set_ax(6);
                    r.cf(true);
                }
            }
            0x41 => {
                let name = cpu.read_cstr(r.ds_dx(cpu));
                let p = self.vfs.resolve_write(&name);
                if std::fs::remove_file(p).is_err() {
                    r.set_ax(2);
                    r.cf(true);
                }
            }
            0x42 => {
                let h = r.bx() as usize;
                let off = ((r.cx() as u32) << 16) | r.dx() as u32;
                let off = if r.rm { off } else { off };
                if let Some(Some(df)) = self.files.get_mut(h) {
                    let pos = match r.al() {
                        0 => SeekFrom::Start(off as u64),
                        1 => SeekFrom::Current(off as i32 as i64),
                        _ => SeekFrom::End(off as i32 as i64),
                    };
                    match df.f.seek(pos) {
                        Ok(p) => {
                            r.set_ax(p as u16);
                            r.set_dx((p >> 16) as u16);
                            r.eax = (r.eax & 0xFFFF) | 0; // Watcom uses DX:AX
                        }
                        Err(_) => {
                            r.set_ax(25);
                            r.cf(true);
                        }
                    }
                } else {
                    r.set_ax(6);
                    r.cf(true);
                }
            }
            0x43 => {
                let name = cpu.read_cstr(r.ds_dx(cpu));
                if self.vfs.resolve_read(&name).is_some() {
                    r.set_cx(0x20);
                } else {
                    r.set_ax(2);
                    r.cf(true);
                }
            }
            0x44 => match r.al() {
                0x00 => {
                    let h = r.bx();
                    r.set_dx(if h < 5 { 0x80D3 } else { 0x0002 });
                }
                0x08 => r.set_ax(1),
                0x09 => r.set_dx(0),
                _ => {}
            },
            0x45 => {
                let h = r.bx() as usize;
                let dup = self.files.get(h).and_then(|f| f.as_ref()).and_then(|df| {
                    df.f.try_clone().ok().map(|f| DosFile { f, name: df.name.clone(), patch: df.patch.clone() })
                });
                match dup {
                    Some(d) => {
                        let nh = self.alloc_handle(d);
                        r.set_ax(nh)
                    }
                    None => r.set_ax(if h < 5 { h as u16 + 10 } else { 0 }),
                }
            }
            0x47 => {
                let a = r.ds_si(cpu);
                cpu.write_bytes(a, b"CYBERMAG\0");
            }
            0x48 => {
                // allocate paragraphs (DOS/4GW: returns selector)
                let paras = r.bx() as u32;
                match self.dos_alloc(paras * 16) {
                    Some(lin) => {
                        let sel = self.alloc_sel(cpu, 1).unwrap_or(0);
                        cpu.descs[(sel >> 3) as usize].base = lin;
                        cpu.descs[(sel >> 3) as usize].limit = paras * 16 - 1;
                        r.set_ax(sel);
                    }
                    None => {
                        r.set_ax(8);
                        r.set_bx(0);
                        r.cf(true);
                    }
                }
            }
            0x49 => {}
            0x4A => {}
            0x4B => {
                // EXEC: spawn child program (used for PLAYTGV.EXE)
                let name = cpu.read_cstr(r.ds_dx(cpu));
                let pb = r.ptr(cpu, r.es, r.ebx);
                // DOS/4GW extended block: env (off32:sel16) at +0, tail (off32:sel16) at +6
                let tail_off = cpu.rd32(pb + 6);
                let tail_sel = cpu.rd16(pb + 10);
                let ta = r.ptr(cpu, tail_sel, tail_off);
                let len = cpu.rd8(ta) as usize;
                let tail = String::from_utf8_lossy(cpu.read_bytes(ta + 1, len)).trim().to_string();
                eprintln!("[dos] exec {} {}", name, tail);
                self.spawn_request = Some((name, tail.split_whitespace().map(|s| s.to_string()).collect()));
                return Err(Fault::Stop);
            }
            0x4C => {
                self.exit_code = Some(r.al() as i32);
                return Err(Fault::Stop);
            }
            0x4D => r.set_ax(0),
            0x4E | 0x4F => {
                if r.ah() == 0x4E {
                    let pat = cpu.read_cstr(r.ds_dx(cpu));
                    self.find = FindState { entries: self.vfs.list(&pat), pos: 0 };
                    if (r.cx() & 0x10) == 0 {
                        self.find.entries.retain(|e| !e.2);
                    }
                }
                if self.find.pos < self.find.entries.len() {
                    let (name, size, isdir) = self.find.entries[self.find.pos].clone();
                    self.find.pos += 1;
                    let d = self.dta;
                    cpu.wr8(d + 0x15, if isdir { 0x10 } else { 0x20 });
                    cpu.wr16(d + 0x16, 0);
                    cpu.wr16(d + 0x18, ((1995 - 1980) << 9) | (11 << 5) | 29);
                    cpu.wr32(d + 0x1A, size as u32);
                    let mut nb = name.into_bytes();
                    nb.truncate(12);
                    nb.push(0);
                    cpu.write_bytes(d + 0x1E, &nb);
                } else {
                    r.set_ax(18);
                    r.cf(true);
                }
            }
            0x56 => {
                let a = cpu.read_cstr(r.ds_dx(cpu));
                let b = cpu.read_cstr(r.es_di(cpu));
                let pa = self.vfs.resolve_write(&a);
                let pb = self.vfs.resolve_write(&b);
                if std::fs::rename(pa, pb).is_err() {
                    r.set_ax(2);
                    r.cf(true);
                }
            }
            0x57 => {
                if r.al() == 0 {
                    r.set_cx(0);
                    r.set_dx(((1995 - 1980) << 9) | (11 << 5) | 29);
                }
            }
            0x58 => r.set_ax(0),
            0x59 => r.set_ax(0),
            0x62 => r.set_bx(SEL_PSP),
            0xFF => {
                // DOS/4G presence check (DX=0078h)
                if r.dx() == 0x78 {
                    r.eax = 0x4734_FFFF; // AL != 0
                    cpu.load_seg(GS, 0);
                }
            }
            _ => {
                self.unknown(&format!("int21 AH={:02X} AX={:04X}", r.ah(), r.ax()), cpu);
                r.cf(true);
            }
        }
        Ok(())
    }

    fn alloc_handle(&mut self, f: DosFile) -> u16 {
        for h in 5..self.files.len() {
            if self.files[h].is_none() {
                self.files[h] = Some(f);
                return h as u16;
            }
        }
        self.files.push(Some(f));
        (self.files.len() - 1) as u16
    }

    pub fn dos_alloc(&mut self, bytes: u32) -> Option<u32> {
        let size = (bytes + 15) & !15;
        // first fit in freed blocks is not tracked; simple bump allocator
        if self.dos_next + size > DOS_HEAP_END {
            return None;
        }
        let a = self.dos_next;
        self.dos_next += size;
        self.dos_blocks.push((a, size));
        Some(a)
    }

    pub fn heap_alloc(&mut self, bytes: u32) -> Option<u32> {
        let size = (bytes + 0xFFF) & !0xFFF;
        if self.heap_next + size > HEAP_END {
            return None;
        }
        let a = self.heap_next;
        self.heap_next += size;
        Some(a)
    }

    // ---------------------------------------------------------------- DPMI
    fn int31(&mut self, cpu: &mut Cpu) -> Result<(), Fault> {
        let ax = cpu.ax();
        cpu.set_cf(false);
        let bx = cpu.r[EBX] as u16;
        match ax {
            0x0000 => {
                let n = (cpu.r[ECX] & 0xFFFF).max(1) as usize;
                match self.alloc_sel(cpu, n) {
                    Some(s) => cpu.set_ax(s),
                    None => cpu.set_cf(true),
                }
            }
            0x0001 => {
                let i = (bx >> 3) as usize;
                if i >= FIRST_DYN_SEL && i < cpu.descs.len() {
                    cpu.descs[i] = Desc::default();
                }
            }
            0x0002 => {
                // segment to descriptor
                let seg = bx as u32;
                let s = self.alloc_sel(cpu, 1).unwrap();
                let d = &mut cpu.descs[(s >> 3) as usize];
                d.base = seg << 4;
                d.limit = 0xFFFF;
                cpu.set_ax(s);
            }
            0x0003 => cpu.set_ax(8),
            0x0006 => {
                let b = cpu.desc_for(bx).map(|d| d.base).unwrap_or(0);
                cpu.set_reg16(ECX, (b >> 16) as u16);
                cpu.set_reg16(EDX, b as u16);
            }
            0x0007 => {
                let b = ((cpu.r[ECX] & 0xFFFF) << 16) | (cpu.r[EDX] & 0xFFFF);
                let i = (bx >> 3) as usize;
                cpu.descs[i].base = b;
                cpu.descs[i].present = true;
                // refresh cached bases
                for s in 0..6 {
                    if cpu.seg[s].sel == bx {
                        cpu.load_seg(s, bx);
                    }
                }
            }
            0x0008 => {
                let l = ((cpu.r[ECX] & 0xFFFF) << 16) | (cpu.r[EDX] & 0xFFFF);
                cpu.descs[(bx >> 3) as usize].limit = l;
            }
            0x0009 => {
                let i = (bx >> 3) as usize;
                cpu.descs[i].access = cpu.r[ECX] as u8;
                cpu.descs[i].code = cpu.r[ECX] & 0x08 != 0;
            }
            0x000A => {
                let src = cpu.desc_for(bx).copied().unwrap_or_default();
                let s = self.alloc_sel(cpu, 1).unwrap();
                cpu.descs[(s >> 3) as usize] = Desc { code: false, access: 0x93, ..src };
                cpu.set_ax(s);
            }
            0x000B => {
                let d = cpu.desc_for(bx).copied().unwrap_or_default();
                let a = cpu.seg[ES].base.wrapping_add(cpu.r[EDI]);
                let lim = if d.limit > 0xFFFFF { d.limit >> 12 } else { d.limit };
                let g = if d.limit > 0xFFFFF { 0x80 } else { 0 };
                cpu.wr16(a, lim as u16);
                cpu.wr16(a + 2, d.base as u16);
                cpu.wr8(a + 4, (d.base >> 16) as u8);
                cpu.wr8(a + 5, d.access | 0x80);
                cpu.wr8(a + 6, g | 0x40 | ((lim >> 16) as u8 & 0xF));
                cpu.wr8(a + 7, (d.base >> 24) as u8);
            }
            0x000C => {
                let a = cpu.seg[ES].base.wrapping_add(cpu.r[EDI]);
                let b = cpu.read_bytes(a, 8).to_vec();
                let base = b[2] as u32 | (b[3] as u32) << 8 | (b[4] as u32) << 16 | (b[7] as u32) << 24;
                let mut lim = b[0] as u32 | (b[1] as u32) << 8 | ((b[6] as u32 & 0xF) << 16);
                if b[6] & 0x80 != 0 {
                    lim = (lim << 12) | 0xFFF;
                }
                let i = (bx >> 3) as usize;
                cpu.descs[i] = Desc { base, limit: lim, present: true, code: b[5] & 8 != 0, big: b[6] & 0x40 != 0, access: b[5] };
            }
            0x0100 => {
                let paras = cpu.r[EBX] & 0xFFFF;
                match self.dos_alloc(paras * 16) {
                    Some(lin) => {
                        let s = self.alloc_sel(cpu, 1).unwrap();
                        cpu.descs[(s >> 3) as usize].base = lin;
                        cpu.descs[(s >> 3) as usize].limit = paras * 16 - 1;
                        cpu.set_ax((lin >> 4) as u16);
                        cpu.set_reg16(EDX, s);
                    }
                    None => {
                        cpu.set_ax(8);
                        cpu.set_reg16(EBX, ((DOS_HEAP_END - self.dos_next) >> 4) as u16);
                        cpu.set_cf(true);
                    }
                }
            }
            0x0101 => {
                let s = cpu.r[EDX] as u16;
                let i = (s >> 3) as usize;
                if i >= FIRST_DYN_SEL && i < cpu.descs.len() {
                    cpu.descs[i] = Desc::default();
                }
            }
            0x0102 => {}
            0x0200 => {
                let v = self.rm_vec[(cpu.r[EBX] & 0xFF) as usize];
                cpu.set_reg16(ECX, (v >> 16) as u16);
                cpu.set_reg16(EDX, v as u16);
            }
            0x0201 => {
                let n = (cpu.r[EBX] & 0xFF) as usize;
                self.rm_vec[n] = ((cpu.r[ECX] & 0xFFFF) << 16) | (cpu.r[EDX] & 0xFFFF);
            }
            0x0202 | 0x0203 => {
                // exception handlers: report a dummy / accept
                if ax == 0x0202 {
                    cpu.set_reg16(ECX, SEL_CODE);
                    cpu.r[EDX] = STUB_BASE;
                }
            }
            0x0204 => {
                let (sel, off) = self.pm_vec[(cpu.r[EBX] & 0xFF) as usize];
                cpu.set_reg16(ECX, sel);
                cpu.r[EDX] = off;
            }
            0x0205 => {
                let n = (cpu.r[EBX] & 0xFF) as usize;
                self.pm_vec[n] = (cpu.r[ECX] as u16, cpu.r[EDX]);
                if self.trace_ints {
                    eprintln!("[dpmi] set PM vector {:02X} -> {:04X}:{:06X}", n, cpu.r[ECX] as u16, le::link(cpu.r[EDX]));
                }
            }
            0x0300 | 0x0301 | 0x0302 => {
                // simulate real-mode interrupt / far call with ES:EDI register structure
                let a = cpu.seg[ES].base.wrapping_add(cpu.r[EDI]);
                let mut r = Regs {
                    edi: cpu.rd32(a),
                    esi: cpu.rd32(a + 4),
                    ebp: cpu.rd32(a + 8),
                    ebx: cpu.rd32(a + 16),
                    edx: cpu.rd32(a + 20),
                    ecx: cpu.rd32(a + 24),
                    eax: cpu.rd32(a + 28),
                    flags: cpu.rd16(a + 32) as u32,
                    es: cpu.rd16(a + 34),
                    ds: cpu.rd16(a + 36),
                    rm: true,
                };
                if ax == 0x0300 {
                    let n = cpu.r[EBX] as u8;
                    match n {
                        0x21 => self.int21(cpu, &mut r)?,
                        _ => self.bios_int(cpu, n, &mut r),
                    }
                } else {
                    let cs = cpu.rd16(a + 44);
                    let ip = cpu.rd16(a + 42);
                    self.unknown(&format!("DPMI 0301 call RM {:04X}:{:04X}", cs, ip), cpu);
                }
                cpu.wr32(a, r.edi);
                cpu.wr32(a + 4, r.esi);
                cpu.wr32(a + 8, r.ebp);
                cpu.wr32(a + 16, r.ebx);
                cpu.wr32(a + 20, r.edx);
                cpu.wr32(a + 24, r.ecx);
                cpu.wr32(a + 28, r.eax);
                cpu.wr16(a + 32, r.flags as u16);
                cpu.wr16(a + 34, r.es);
                cpu.wr16(a + 36, r.ds);
            }
            0x0303 => {
                // allocate real-mode callback: give a dummy address
                cpu.set_reg16(ECX, 0xF000);
                cpu.set_reg16(EDX, 0x0900);
            }
            0x0304 => {}
            0x0400 => {
                cpu.set_ax(0x0009);
                cpu.set_reg16(EBX, 0x0005);
                cpu.set_reg8(1, 4); // CL = processor 486
                cpu.set_reg16(EDX, 0x0870);
            }
            0x0500 => {
                let a = cpu.seg[ES].base.wrapping_add(cpu.r[EDI]);
                let free = HEAP_END - self.heap_next;
                for i in 0..12 {
                    cpu.wr32(a + i * 4, 0xFFFF_FFFF);
                }
                cpu.wr32(a, free);
                cpu.wr32(a + 4, free / 4096);
                cpu.wr32(a + 8, free / 4096);
                cpu.wr32(a + 0x0C, free / 4096);
                cpu.wr32(a + 0x14, free / 4096);
                cpu.wr32(a + 0x18, (HEAP_END - HEAP_START) / 4096);
            }
            0x0501 => {
                let size = ((cpu.r[EBX] & 0xFFFF) << 16) | (cpu.r[ECX] & 0xFFFF);
                match self.heap_alloc(size) {
                    Some(lin) => {
                        self.heap_blocks.push((lin, size));
                        let h = self.heap_blocks.len() as u32;
                        cpu.set_reg16(EBX, (lin >> 16) as u16);
                        cpu.set_reg16(ECX, lin as u16);
                        cpu.set_reg16(ESI, (h >> 16) as u16);
                        cpu.set_reg16(EDI, h as u16);
                    }
                    None => {
                        cpu.set_ax(0x8013);
                        cpu.set_cf(true);
                    }
                }
            }
            0x0502 => {}
            0x0503 => {
                // resize: allocate new block and copy
                let size = ((cpu.r[EBX] & 0xFFFF) << 16) | (cpu.r[ECX] & 0xFFFF);
                let h = (((cpu.r[ESI] & 0xFFFF) << 16) | (cpu.r[EDI] & 0xFFFF)) as usize;
                let old = self.heap_blocks.get(h.wrapping_sub(1)).copied();
                match (old, self.heap_alloc(size)) {
                    (Some((oa, os)), Some(lin)) => {
                        let n = os.min(size) as usize;
                        cpu.mem.copy_within(oa as usize..oa as usize + n, lin as usize);
                        self.heap_blocks.push((lin, size));
                        let nh = self.heap_blocks.len() as u32;
                        cpu.set_reg16(EBX, (lin >> 16) as u16);
                        cpu.set_reg16(ECX, lin as u16);
                        cpu.set_reg16(ESI, (nh >> 16) as u16);
                        cpu.set_reg16(EDI, nh as u16);
                    }
                    _ => {
                        cpu.set_ax(0x8013);
                        cpu.set_cf(true);
                    }
                }
            }
            0x0600 | 0x0601 | 0x0602 | 0x0603 | 0x0702 | 0x0703 => {}
            0x0604 => {
                cpu.set_reg16(EBX, 0);
                cpu.set_reg16(ECX, 4096);
            }
            0x0800 => {
                // physical address mapping: identity
            }
            0x0801 => {}
            0x0900 => {
                let old = cpu.iflag() as u8;
                cpu.set_flags(cpu.flags() & !IF);
                cpu.set_reg8(0, old);
            }
            0x0901 => {
                let old = cpu.iflag() as u8;
                cpu.set_flags(cpu.flags() | IF);
                cpu.set_reg8(0, old);
            }
            0x0902 => {
                let v = cpu.iflag() as u8;
                cpu.set_reg8(0, v);
            }
            0x0A00 => cpu.set_cf(true),
            0x0E00 => cpu.set_ax(0x0200),
            _ => {
                self.unknown(&format!("DPMI AX={:04X}", ax), cpu);
                cpu.set_cf(true);
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------- BIOS
    pub fn bios_int(&mut self, cpu: &mut Cpu, n: u8, r: &mut Regs) {
        match n {
            0x10 => self.int10(cpu, r),
            0x16 => match r.ah() {
                0x00 | 0x10 => {
                    let k = self.kbd_bios.pop_front().unwrap_or(0);
                    r.set_ax(k);
                }
                0x01 | 0x11 => match self.kbd_bios.front() {
                    Some(&k) => {
                        r.set_ax(k);
                        r.zf(false);
                    }
                    None => r.zf(true),
                },
                0x02 | 0x12 => r.set_al(0),
                _ => {}
            },
            0x1A => {
                if r.ah() == 0 {
                    let t = cpu.rd32(0x46C);
                    r.set_cx((t >> 16) as u16);
                    r.set_dx(t as u16);
                    r.set_al(0);
                }
            }
            0x33 => self.int33(cpu, r),
            0x11 => r.set_ax(0x0026),
            0x12 => r.set_ax(640),
            0x2F => {
                // multiplex: no XMS / no Windows / no MSCDEX
                if r.ax() == 0x1500 {
                    r.set_bx(1);
                    r.set_cx(3); // D:
                }
            }
            0x15 => r.cf(true),
            _ => self.unknown(&format!("bios int {:02X} AX={:04X}", n, r.ax()), cpu),
        }
    }

    fn int10(&mut self, cpu: &mut Cpu, r: &mut Regs) {
        match r.ah() {
            0x00 => {
                let mode = r.al() as u16 & 0x7F;
                self.vga.set_mode(cpu, mode, true);
                cpu.wr8(0x449, mode as u8);
            }
            0x0F => {
                r.set_al(cpu.rd8(0x449));
                r.set_ah(80);
                r.ebx &= !0xFF00;
            }
            0x02 | 0x01 | 0x05 | 0x06 | 0x07 | 0x09 | 0x0A | 0x0E | 0x08 | 0x03 | 0x0B => {
                if r.ah() == 0x0E {
                    self.print(&(r.al() as char).to_string());
                }
                if r.ah() == 0x03 {
                    r.set_dx(0);
                    r.set_cx(0x0607);
                }
            }
            0x10 => match r.al() {
                0x10 => {
                    let i = r.bx() as usize & 255;
                    self.vga.dac[i] = [(r.dx() >> 8) as u8 & 63, (r.cx() >> 8) as u8 & 63, r.cx() as u8 & 63];
                }
                0x12 => {
                    let start = r.bx() as usize;
                    let cnt = r.cx() as usize;
                    let a = r.es_dx(cpu);
                    for k in 0..cnt {
                        let c = cpu.read_bytes(a + k as u32 * 3, 3);
                        self.vga.dac[(start + k) & 255] = [c[0] & 63, c[1] & 63, c[2] & 63];
                    }
                }
                0x15 => {
                    let i = r.bx() as usize & 255;
                    let c = self.vga.dac[i];
                    r.set_dx((c[0] as u16) << 8);
                    r.set_cx(((c[1] as u16) << 8) | c[2] as u16);
                }
                0x17 => {
                    let start = r.bx() as usize;
                    let cnt = r.cx() as usize;
                    let a = r.es_dx(cpu);
                    for k in 0..cnt {
                        let c = self.vga.dac[(start + k) & 255];
                        cpu.write_bytes(a + k as u32 * 3, &c);
                    }
                }
                _ => {}
            },
            0x11 | 0x12 => {
                if r.ah() == 0x12 && r.bx() as u8 == 0x10 {
                    r.set_bx(0x0003);
                }
            }
            0x1A => {
                r.set_al(0x1A);
                r.set_bx(0x0008);
            }
            0x4F => self.vbe(cpu, r),
            _ => self.unknown(&format!("int10 AX={:04X}", r.ax()), cpu),
        }
    }

    fn vbe(&mut self, cpu: &mut Cpu, r: &mut Regs) {
        let modes: [u16; 4] = [0x100, 0x101, 0x103, 0x105];
        match r.al() {
            0x00 => {
                let a = r.es_di(cpu);
                cpu.write_bytes(a, b"VESA");
                cpu.wr16(a + 4, 0x0102);
                // OEM string & mode list stored in the same buffer (as real-mode far pointers)
                let seg = if r.rm { r.es } else { (a >> 4) as u16 };
                let base_off = if r.rm { r.edi as u16 } else { (a & 15) as u16 };
                cpu.wr16(a + 6, base_off + 0x100);
                cpu.wr16(a + 8, seg);
                cpu.write_bytes(a + 0x100, b"CyberMage Rust VBE\0");
                cpu.wr32(a + 10, 0);
                cpu.wr16(a + 14, base_off + 0x120);
                cpu.wr16(a + 16, seg);
                for (i, &m) in modes.iter().enumerate() {
                    cpu.wr16(a + 0x120 + i as u32 * 2, m);
                }
                cpu.wr16(a + 0x120 + modes.len() as u32 * 2, 0xFFFF);
                cpu.wr16(a + 18, (crate::vga::VRAM_SIZE / 65536) as u16);
                r.set_ax(0x004F);
            }
            0x01 => {
                let m = r.cx() & 0x1FF;
                let (w, h) = match m {
                    0x100 => (640, 400),
                    0x101 => (640, 480),
                    0x103 => (800, 600),
                    0x105 => (1024, 768),
                    _ => {
                        r.set_ax(0x014F);
                        return;
                    }
                };
                let a = r.es_di(cpu);
                for i in 0..256 {
                    cpu.wr8(a + i, 0);
                }
                cpu.wr16(a, 0x009B); // attributes: supported, color, graphics
                cpu.wr8(a + 2, 0x07); // window A: relocatable, readable, writeable
                cpu.wr8(a + 3, 0x00);
                cpu.wr16(a + 4, 64); // granularity KB
                cpu.wr16(a + 6, 64); // window size KB
                cpu.wr16(a + 8, 0xA000);
                cpu.wr16(a + 10, 0);
                cpu.wr32(a + 12, 0); // window function ptr (use int 10h)
                cpu.wr16(a + 16, w);
                cpu.wr16(a + 18, w);
                cpu.wr16(a + 20, h);
                cpu.wr8(a + 22, 8);
                cpu.wr8(a + 23, 16);
                cpu.wr8(a + 24, 1);
                cpu.wr8(a + 25, 8);
                cpu.wr8(a + 26, 1);
                cpu.wr8(a + 27, 4); // packed pixel
                cpu.wr8(a + 28, 0);
                cpu.wr8(a + 29, ((crate::vga::VRAM_SIZE as u32 / (w as u32 * h as u32)) - 1).min(255) as u8);
                cpu.wr8(a + 30, 1);
                r.set_ax(0x004F);
            }
            0x02 => {
                let m = r.bx() & 0x1FF;
                if self.vga.set_mode(cpu, m, r.bx() & 0x8000 == 0) {
                    r.set_ax(0x004F);
                } else {
                    r.set_ax(0x014F);
                }
            }
            0x03 => {
                r.set_bx(self.vga.mode);
                r.set_ax(0x004F);
            }
            0x05 => {
                if r.bx() & 0x100 == 0 {
                    let bank = r.dx() as usize;
                    self.vga.set_bank(cpu, bank);
                } else {
                    r.set_dx(self.vga.bank as u16);
                }
                r.set_ax(0x004F);
            }
            0x06 => {
                if r.bx() as u8 == 0 || r.bx() as u8 == 2 {
                    self.vga.pitch = r.cx() as u32;
                }
                r.set_bx(self.vga.pitch as u16);
                r.set_cx(self.vga.pitch as u16);
                r.set_dx((crate::vga::VRAM_SIZE as u32 / self.vga.pitch.max(1)) as u16);
                r.set_ax(0x004F);
            }
            0x07 => {
                if r.bx() as u8 & 1 == 0 {
                    let x = r.cx() as u32;
                    let y = r.dx() as u32;
                    self.vga.display_start = y * self.vga.pitch + x;
                } else {
                    r.set_cx((self.vga.display_start % self.vga.pitch.max(1)) as u16);
                    r.set_dx((self.vga.display_start / self.vga.pitch.max(1)) as u16);
                }
                r.set_ax(0x004F);
            }
            0x08 => {
                r.set_bx(0x0600);
                r.set_ax(0x004F);
            }
            _ => {
                self.unknown(&format!("VBE AX={:04X}", r.ax()), cpu);
                r.set_ax(0x014F);
            }
        }
    }

    fn int33(&mut self, cpu: &mut Cpu, r: &mut Regs) {
        let m = &mut self.mouse;
        match r.ax() {
            0x00 => {
                r.set_ax(0xFFFF);
                r.set_bx(2);
            }
            0x01 | 0x02 | 0x07 | 0x08 | 0x0A | 0x0F | 0x10 | 0x13 | 0x1A | 0x1D => match r.ax() {
                0x07 => {
                    m.min_x = r.cx() as i16 as i32;
                    m.max_x = r.dx() as i16 as i32;
                }
                0x08 => {
                    m.min_y = r.cx() as i16 as i32;
                    m.max_y = r.dx() as i16 as i32;
                }
                _ => {}
            },
            0x03 => {
                r.set_bx(m.buttons);
                r.set_cx(m.x.clamp(m.min_x, m.max_x) as u16);
                r.set_dx(m.y.clamp(m.min_y, m.max_y) as u16);
            }
            0x04 => {
                m.x = r.cx() as i16 as i32;
                m.y = r.dx() as i16 as i32;
            }
            0x05 | 0x06 => {
                let b = (r.bx() as usize).min(2);
                let pressed = r.ax() == 5;
                r.set_bx(if pressed { m.press_count[b] } else { m.release_count[b] });
                if pressed {
                    m.press_count[b] = 0
                } else {
                    m.release_count[b] = 0
                }
                r.set_ax(m.buttons);
                r.set_cx(m.x as u16);
                r.set_dx(m.y as u16);
            }
            0x0B => {
                r.set_cx(m.mickey_x as i16 as u16);
                r.set_dx(m.mickey_y as i16 as u16);
                m.mickey_x = 0;
                m.mickey_y = 0;
            }
            0x0C | 0x14 => {
                // user callback: not supported (game polls)
                let _ = cpu;
            }
            0x21 => {
                r.set_ax(0xFFFF);
                r.set_bx(2);
            }
            0x24 => {
                r.set_bx(0x0800);
                r.set_cx(0x0400);
            }
            _ => {
                let ax = r.ax();
                self.unknown(&format!("int33 AX={:04X}", ax), cpu);
            }
        }
    }
}
