//! Native ports of misc.cpp helpers.

use crate::machine::Machine;
use x86::*;

pub fn install(m: &mut Machine, cpu: &mut Cpu) {
    m.hook_sig(cpu, 0x077D97, "__STOSD", 4, true, h_stosd);
    m.hook_sig(cpu, 0x068AD0, "RANDOM", 0, true, h_random);
    m.hook_sig(cpu, 0x06731C, "RND", 1, true, h_rnd);
    m.hook_sig(cpu, 0x0664C4, "memset2", 4, false, h_memset2);
    m.hook_sig(cpu, 0x0664EC, "memcpy2", 4, false, h_memcpy2);
}

/// Original three-word generator, including signed 16-bit intermediate wraps.
fn random(c: &mut Cpu) -> u32 {
    let mut value = 0;
    for (addr, divisor, multiplier, quotient_scale, modulus) in [
        (0x9747F,177,171,2,30269), (0x97481,176,172,35,30307), (0x97483,178,170,63,30323),
    ] {
        let seed = c.rd16(crate::le::rt(addr)) as i16 as i32;
        let mut next = (multiplier * (seed % divisor) - quotient_scale * (seed / divisor)) as i16;
        if next < 0 { next = next.wrapping_add(modulus as i16); }
        c.wr16(crate::le::rt(addr), next as u16);
        value += (next as u16 as u32 % modulus) * 10000 / modulus;
    }
    value & 0x7FFF
}

fn h_random(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    c.r[EAX] = random(c);
    c.ret_near(0);
    Ok(())
}

fn h_rnd(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let bound = c.r[EAX] as i32;
    c.r[EAX] = if bound == 0 {0} else {(random(c) as i32 % bound) as u32};
    c.ret_near(0);
    Ok(())
}

/// Watcom's dword fill helper used by __STOSB. Preserve its end pointer and
/// the unusual ECX=1 after a three-dword tail; CPU writes retain SMC/logging.
fn h_stosd(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (dst,value,count) = (c.r[EAX],c.r[EDX],c.r[ECX]);
    for i in 0..count { c.wr32(dst.wrapping_add(i.wrapping_mul(4)),value); }
    let leading = if dst & 3 == 0 { (32 - (dst & 31)) % 32 / 4 } else { count };
    c.r[EAX] = dst.wrapping_add(count.wrapping_mul(4));
    c.r[ECX] = u32::from(count.saturating_sub(leading) & 3 == 3);
    c.ret_near(0);
    Ok(())
}

/// Fill a `w` x `h` rectangle with `val`; rows are `stride` bytes apart.
pub fn memset2(c: &mut Cpu, dst: u32, val: u8, w: u32, h: u32, stride: u32) {
    let mut p = dst;
    for _ in 0..h {
        for i in 0..w {
            c.wr8(p.wrapping_add(i), val);
        }
        p = p.wrapping_add(stride);
    }
}

/// Copy a `w` x `h` rectangle between buffers with independent strides.
pub fn memcpy2(c: &mut Cpu, dst: u32, src: u32, w: u32, h: u32, dst_stride: u32, src_stride: u32) {
    let (mut d, mut s) = (dst, src);
    for _ in 0..h {
        for i in 0..w {
            let b = c.rd8(s.wrapping_add(i));
            c.wr8(d.wrapping_add(i), b);
        }
        d = d.wrapping_add(dst_stride);
        s = s.wrapping_add(src_stride);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn random_matches_original_sequence_and_zero_bound_does_not_advance() {
        let path = std::path::PathBuf::from("__random_no_files__");
        let mut m = Machine::new(crate::vfs::Vfs::new(path.clone(),path));
        let mut c = Cpu::new();
        for (addr,seed) in [(0x9747F,1),(0x97481,2),(0x97483,3)] { c.wr16(crate::le::rt(addr),seed); }
        assert_eq!(random(&mut c),337);
        assert_eq!(random(&mut c),27774);
        assert_eq!((c.rd16(crate::le::rt(0x9747F)),c.rd16(crate::le::rt(0x97481)),c.rd16(crate::le::rt(0x97483))),(29241,28861,26054));
        c.r[ESP] = 0x30000; c.r[EAX] = 0; c.r[EDX] = 0xAA55; c.r[EBX] = 0xBB66;
        let seeds = c.read_bytes(crate::le::rt(0x9747F),6).to_vec();
        c.push32(0x1234); c.wlog_on = true;
        h_rnd(&mut m,&mut c).unwrap();
        assert!(c.wlog.is_empty()); assert_eq!(c.r[EAX],0);
        assert_eq!(c.read_bytes(crate::le::rt(0x9747F),6),seeds);
        assert_eq!((c.r[EDX],c.r[EBX],c.eip,c.r[ESP]),(0xAA55,0xBB66,0x1234,0x30000));
        for bound in [1,7,100,-7,i32::MIN] {
            c.r[EAX] = bound as u32; c.push32(0x1234);
            h_rnd(&mut m,&mut c).unwrap();
            assert!((c.r[EAX] as i32) >= 0 && (c.r[EAX] as i64) < (bound as i64).abs());
        }
    }
    #[test]
    fn dword_fill_preserves_alignment_tail_logging_and_code_invalidation() {
        let path = std::path::PathBuf::from("__stosd_no_files__");
        let mut m = Machine::new(crate::vfs::Vfs::new(path.clone(),path));
        let mut c = Cpu::new();
        for offset in 0..32 { for count in 0..20 {
            let dst = 0x2000 + offset;
            c.mem[0x2000..0x2080].fill(0xAA);
            c.r[ESP] = 0x10000; c.push32(0x1234);
            c.r[EAX] = dst; c.r[EDX] = 0x98765432; c.r[ECX] = count;
            h_stosd(&mut m,&mut c).unwrap();
            assert_eq!(c.r[EAX],dst + count*4);
            assert_eq!((c.eip,c.r[ESP]),(0x1234,0x10000));
            for i in 0..count { assert_eq!(c.rd32(dst+i*4),0x98765432); }
            assert_eq!(c.rd8(dst+count*4),0xAA);
            let leading = if offset % 4 == 0 {(32-offset)%32/4} else {count};
            assert_eq!(c.r[ECX],u32::from(count.saturating_sub(leading)%4 == 3));
        } }
        c.set_code_range(0x2000,64); c.icache[0].fl = 1;
        c.r[ESP] = 0x10000; c.push32(0x1234);
        c.r[EAX] = 0x2000; c.r[ECX] = 4;
        c.wlog_on = true;
        h_stosd(&mut m,&mut c).unwrap();
        assert_eq!(c.wlog.len(),16);
        assert_eq!(c.icache[0].fl,0);
    }
}

fn h_memset2(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let stride = c.stack_arg(0);
    memset2(c, c.r[EAX], c.r[EDX] as u8, c.r[EBX], c.r[ECX], stride);
    c.ret_near(4);
    Ok(())
}

fn h_memcpy2(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (ds, ss) = (c.stack_arg(0), c.stack_arg(1));
    memcpy2(c, c.r[EAX], c.r[EDX], c.r[EBX], c.r[ECX], ds, ss);
    c.ret_near(8);
    Ok(())
}
