//! Differential verification of native replacements.
//!
//! With `CM_VERIFY=Name1,Name2` (or `all`), every call to a hooked function
//! is executed twice from the same machine state: first the original guest
//! code (entry bytes temporarily restored), then the native implementation.
//! Memory effects are captured with the CPU write log, so the cost is
//! proportional to the bytes written.  Registers and written memory are
//! compared; the original's results are kept so the run stays faithful.
//! `CM_VERIFY_EVERY=N` verifies only every Nth call.

use crate::le;
use crate::machine::{Machine, STUB_BASE, TRAP_HOOK_BASE};
use std::collections::BTreeMap;
use x86::*;

pub struct Verifier {
    pub names: Vec<String>,
    pub all: bool,
    pub every: u64,
    pub calls: Vec<u64>,
    pub mismatches: u64,
    pub checked: u64,
    pub max_reports: u64,
    /// per hook: (checked, mismatched)
    pub per: std::collections::BTreeMap<&'static str, (u64, u64)>,
}

impl Verifier {
    pub fn from_env() -> Option<Verifier> {
        let v = std::env::var("CM_VERIFY").ok()?;
        let names: Vec<String> = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        let every = std::env::var("CM_VERIFY_EVERY").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
        Some(Verifier { all: names.iter().any(|n| n == "all"), names, every, calls: Vec::new(), mismatches: 0, checked: 0, max_reports: 40, per: Default::default() })
    }
    pub fn wants(&self, name: &str) -> bool {
        self.all || self.names.iter().any(|n| n == name)
    }
}

/// Undo logged writes (newest first).
fn undo(cpu: &mut Cpu, log: &[(u32, u8)]) {
    for &(a, old) in log.iter().rev() {
        cpu.mem[a as usize] = old;
    }
}

/// Final bytes for each written address: addr -> (original value, final value)
fn effects(cpu: &Cpu, log: &[(u32, u8)]) -> BTreeMap<u32, (u8, u8)> {
    let mut m = BTreeMap::new();
    for &(a, old) in log {
        m.entry(a).or_insert((old, 0));
    }
    for (a, v) in m.iter_mut() {
        v.1 = cpu.mem[*a as usize];
    }
    m
}

/// Run hook `idx` in verification mode. Called instead of the native hook.
pub fn run(m: &mut Machine, cpu: &mut Cpu, idx: usize) -> Result<(), Fault> {
    let (entry, f, name, orig, reg_args, returns) = {
        let h = &m.hooks[idx];
        (le::rt(h.link_addr), h.f, h.name, h.orig, h.reg_args, h.returns)
    };
    let mut ignore = [false; 8];
    for (k, &r) in [EAX, EDX, EBX, ECX].iter().enumerate() {
        if (k as u8) < reg_args {
            ignore[r] = true;
        }
    }
    ignore[EAX] = !returns;
    {
        let v = m.verifier.as_mut().unwrap();
        if v.calls.len() <= idx {
            v.calls.resize(idx + 1, 0);
        }
        v.calls[idx] += 1;
        if v.calls[idx] % v.every != 0 || cpu.wlog_on {
            return f(m, cpu);
        }
    }
    let r0 = cpu.r;
    let fl0 = cpu.flags();
    let seg0 = cpu.seg;
    let fpu0 = cpu.fpu.clone();
    // Voice handles are host state, too: otherwise the first pass occupies a
    // sample slot and the second pass spuriously chooses a different channel.
    let voices0 = m.audio.mixer.voices.clone();
    let esp0 = cpu.r[ESP];
    let ret_addr = cpu.rd32(esp0);

    // ---- original guest code
    let id = TRAP_HOOK_BASE + idx as u16;
    cpu.write_bytes(entry, &orig);
    cpu.wlog.clear();
    cpu.wlog_on = true;
    cpu.wr32(esp0, STUB_BASE + 0x800);
    cpu.eip = entry;
    let res = loop {
        match cpu.step(m) {
            Ok(()) => {}
            Err(Fault::Stop) => break Ok(()),
            Err(e) => break Err(e),
        }
    };
    cpu.wlog_on = false;
    let log_a = std::mem::take(&mut cpu.wlog);
    cpu.write_bytes(entry, &[0x0F, 0xFF, id as u8, (id >> 8) as u8]);
    res?;
    let ra = cpu.r;
    let fla = cpu.flags();
    let fpua = cpu.fpu.clone();
    let voicesa = m.audio.mixer.voices.clone();
    let eff_a = effects(cpu, &log_a);
    undo(cpu, &log_a);

    // ---- native
    cpu.r = r0;
    cpu.set_flags(fl0);
    cpu.seg = seg0;
    cpu.fpu = fpu0;
    m.audio.mixer.voices = voices0;
    cpu.eip = entry;
    cpu.wlog.clear();
    cpu.wlog_on = true;
    let res = f(m, cpu);
    cpu.wlog_on = false;
    let log_b = std::mem::take(&mut cpu.wlog);
    res?;
    let eff_b = effects(cpu, &log_b);

    // ---- compare
    let mut report = String::new();
    if m.audio.mixer.voices != voicesa { report.push_str("  sample mixer voices differ\n"); }
    const NAMES: [&str; 8] = ["EAX", "ECX", "EDX", "EBX", "ESP", "EBP", "ESI", "EDI"];
    for i in 0..8 {
        if cpu.r[i] != ra[i] && !ignore[i] {
            report.push_str(&format!("  {} guest={:08X} native={:08X}\n", NAMES[i], ra[i], cpu.r[i]));
        }
    }
    if cpu.eip != ret_addr {
        report.push_str(&format!("  EIP native={:08X} expected return {:08X}\n", cpu.eip, ret_addr));
    }
    // memory: compare final values over the union of written addresses,
    // ignoring the scratch stack below the entry ESP.
    // (plus the callee's own stack-argument slots, which some functions reuse as locals)
    let in_stack = |a: u32| a >= esp0.saturating_sub(0x20000) && a < esp0 + 4 + 0x20;
    let mut diffs: Vec<(u32, u8, u8)> = Vec::new();
    for (&a, &(old, va)) in &eff_a {
        if in_stack(a) {
            continue;
        }
        let vb = eff_b.get(&a).map(|x| x.1).unwrap_or(old);
        if va != vb {
            diffs.push((a, va, vb));
        }
    }
    for (&a, &(old, vb)) in &eff_b {
        if in_stack(a) || eff_a.contains_key(&a) {
            continue;
        }
        if vb != old {
            diffs.push((a, old, vb));
        }
    }
    diffs.sort();
    if !diffs.is_empty() {
        report.push_str(&format!("  {} bytes differ (guest wrote {}, native wrote {})\n", diffs.len(), eff_a.len(), eff_b.len()));
        for &(a, va, vb) in diffs.iter().take(12) {
            report.push_str(&format!("    link {:06X}: guest {:02X} native {:02X}\n", le::link(a), va, vb));
        }
    }
    let v = m.verifier.as_mut().unwrap();
    v.checked += 1;
    let e = v.per.entry(name).or_insert((0, 0));
    e.0 += 1;
    if !report.is_empty() {
        e.1 += 1;
        v.mismatches += 1;
        if v.mismatches <= v.max_reports {
            eprintln!("[verify] MISMATCH {} call #{}:\n{}", name, v.calls[idx], report);
        }
    }
    // keep the original's results: undo native, replay original
    undo(cpu, &log_b);
    for (&a, &(_, va)) in &eff_a {
        cpu.mem[a as usize] = va;
    }
    cpu.r = ra;
    cpu.set_flags(fla);
    cpu.fpu = fpua;
    m.audio.mixer.voices = voicesa;
    cpu.eip = ret_addr;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn allocate_voice(m: &mut Machine,c: &mut Cpu)->Result<(),Fault> {
        let slot=m.audio.mixer.voices.iter().position(Option::is_none).unwrap();
        m.audio.mixer.voices[slot]=Some(crate::audio::Voice {data:std::sync::Arc::new(vec![128,255]),pos:0.0,rate:11025.0,vol_l:1.0,vol_r:1.0,loops:0,id:slot as u32});
        c.r[EAX]=slot as u32; c.wr32(0x300000,slot as u32); c.ret_near(0); Ok(())
    }
    fn wrong_gain(m: &mut Machine,c: &mut Cpu)->Result<(),Fault> {
        allocate_voice(m,c)?; m.audio.mixer.voices[c.r[EAX] as usize].as_mut().unwrap().vol_l=0.25; Ok(())
    }
    #[test]
    fn paired_calls_start_with_identical_voice_slots_and_keep_only_the_original_pass() {
        let path=std::path::PathBuf::from("__verify_no_files__");
        let mut m=Machine::new(crate::vfs::Vfs::new(path.clone(),path)); let mut c=Cpu::new();
        c.r[ESP]=0x30000; c.write_bytes(STUB_BASE+0x800,&[0x0F,0xFF,0,1]);
        c.write_bytes(0x410000,&[0xE8,0xFB,0xFF,0,0,0xC3]); // CALL 420000; RET
        m.hook_sig(&mut c,0x20000,"allocate voice",0,true,allocate_voice);
        m.hook_sig(&mut c,0x10000,"checked voice allocation",0,true,allocate_voice);
        m.verifier=Some(Verifier {names:vec!["checked voice allocation".into()],all:false,every:1,calls:Vec::new(),mismatches:0,checked:0,max_reports:40,per:Default::default()});
        for slot in 0..4 {
            c.push32(0x1234); run(&mut m,&mut c,1).unwrap();
            assert_eq!((c.r[EAX],c.rd32(0x300000),c.r[ESP],c.eip),(slot,slot,0x30000,0x1234));
            assert_eq!(m.audio.mixer.voices.iter().flatten().count(),slot as usize+1);
        }
        let v=m.verifier.as_ref().unwrap(); assert_eq!((v.checked,v.mismatches),(4,0));
        m.hooks[1].f=wrong_gain; c.push32(0x1234); run(&mut m,&mut c,1).unwrap();
        let v=m.verifier.as_ref().unwrap(); assert_eq!((v.checked,v.mismatches),(5,1));
        assert_eq!(m.audio.mixer.voices[4].as_ref().unwrap().vol_l,1.0);
    }
}
