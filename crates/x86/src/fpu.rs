//! x87 FPU (f64 precision internally).
use crate::exec::Rm;
use crate::*;

#[derive(Clone, Debug)]
pub struct Fpu {
    pub st: [f64; 8],
    pub top: usize,
    pub empty: u8,
    pub cw: u16,
    pub sw: u16,
}

const C0: u16 = 0x100;
const C1: u16 = 0x200;
const C2: u16 = 0x400;
const C3: u16 = 0x4000;

impl Fpu {
    pub fn new() -> Fpu {
        Fpu { st: [0.0; 8], top: 0, empty: 0xFF, cw: 0x037F, sw: 0 }
    }
    #[inline]
    pub fn get(&self, i: usize) -> f64 {
        self.st[(self.top + i) & 7]
    }
    #[inline]
    pub fn set(&mut self, i: usize, v: f64) {
        let k = (self.top + i) & 7;
        self.st[k] = v;
        self.empty &= !(1 << k);
    }
    pub fn push(&mut self, v: f64) {
        self.top = (self.top + 7) & 7;
        self.st[self.top] = v;
        self.empty &= !(1 << self.top);
    }
    pub fn pop(&mut self) -> f64 {
        let v = self.st[self.top];
        self.empty |= 1 << self.top;
        self.top = (self.top + 1) & 7;
        v
    }
    pub fn status(&self) -> u16 {
        (self.sw & !0x3800) | ((self.top as u16) << 11)
    }
    pub fn tag_word(&self) -> u16 {
        let mut t = 0u16;
        for k in 0..8 {
            let v = if self.empty & (1 << k) != 0 {
                3
            } else if self.st[k] == 0.0 {
                1
            } else {
                0
            };
            t |= v << (k * 2);
        }
        t
    }
    fn set_cc(&mut self, c: u16) {
        self.sw = (self.sw & !(C0 | C1 | C2 | C3)) | c;
    }
    fn compare(&mut self, a: f64, b: f64) {
        let c = if a.is_nan() || b.is_nan() {
            C0 | C2 | C3
        } else if a > b {
            0
        } else if a < b {
            C0
        } else {
            C3
        };
        self.set_cc(c);
    }
    pub fn round(&self, x: f64) -> f64 {
        match (self.cw >> 10) & 3 {
            0 => x.round_ties_even(),
            1 => x.floor(),
            2 => x.ceil(),
            _ => x.trunc(),
        }
    }
}

impl Default for Fpu {
    fn default() -> Self {
        Self::new()
    }
}

pub fn f80_to_f64(b: &[u8]) -> f64 {
    let mant = u64::from_le_bytes(b[0..8].try_into().unwrap());
    let se = u16::from_le_bytes([b[8], b[9]]);
    let sign = se & 0x8000 != 0;
    let exp = (se & 0x7FFF) as i32;
    let v = if exp == 0 && mant == 0 {
        0.0
    } else if exp == 0x7FFF {
        if mant << 1 == 0 {
            f64::INFINITY
        } else {
            f64::NAN
        }
    } else {
        (mant as f64) * 2f64.powi(exp - 16383 - 63)
    };
    if sign {
        -v
    } else {
        v
    }
}

pub fn f64_to_f80(v: f64) -> [u8; 10] {
    let mut out = [0u8; 10];
    let bits = v.to_bits();
    let sign = (bits >> 63) as u16;
    let exp = ((bits >> 52) & 0x7FF) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    let (e80, m80) = if exp == 0 && frac == 0 {
        (0u16, 0u64)
    } else if exp == 0x7FF {
        (0x7FFF, if frac == 0 { 1u64 << 63 } else { (1u64 << 63) | (1u64 << 62) })
    } else if exp == 0 {
        // denormal: normalize
        let lz = frac.leading_zeros() as i32 - 11;
        let m = frac << (lz + 11);
        ((1 - 1023 - lz + 16383) as u16, m)
    } else {
        ((exp - 1023 + 16383) as u16, (1u64 << 63) | (frac << 11))
    };
    out[0..8].copy_from_slice(&m80.to_le_bytes());
    out[8..10].copy_from_slice(&(e80 | (sign << 15)).to_le_bytes());
    out
}

impl Cpu {
    fn fpu_arith(&mut self, sub: usize, a: f64, b: f64) -> f64 {
        match sub {
            0 => a + b,
            1 => a * b,
            4 => a - b,
            5 => b - a,
            6 => a / b,
            7 => b / a,
            _ => a,
        }
    }

    fn fist(&mut self, a: u32, sz: u8, v: f64) {
        let r = self.fpu.round(v);
        match sz {
            2 => {
                let x = if r.is_nan() || r < -32768.0 || r > 32767.0 { 0x8000 } else { r as i16 as u16 };
                self.wr16(a, x)
            }
            4 => {
                let x = if r.is_nan() || r < -2147483648.0 || r > 2147483647.0 { 0x8000_0000 } else { r as i32 as u32 };
                self.wr32(a, x)
            }
            _ => {
                let x = if r.is_nan() || r.abs() > 9.2e18 { 0x8000_0000_0000_0000 } else { r as i64 as u64 };
                self.wr64(a, x)
            }
        }
    }

    fn fsave_env(&mut self, a: u32) {
        let cw = self.fpu.cw as u32;
        let sw = self.fpu.status() as u32;
        let tw = self.fpu.tag_word() as u32;
        self.wr32(a, cw | 0xFFFF_0000);
        self.wr32(a + 4, sw | 0xFFFF_0000);
        self.wr32(a + 8, tw | 0xFFFF_0000);
        for i in 3..7 {
            self.wr32(a + i * 4, 0);
        }
    }
    fn fload_env(&mut self, a: u32) {
        self.fpu.cw = self.rd16(a);
        let sw = self.rd16(a + 4);
        self.fpu.sw = sw & !0x3800;
        self.fpu.top = ((sw >> 11) & 7) as usize;
        let tw = self.rd16(a + 8);
        self.fpu.empty = 0;
        for k in 0..8 {
            if (tw >> (k * 2)) & 3 == 3 {
                self.fpu.empty |= 1 << k;
            }
        }
    }

    pub(crate) fn exec_fpu(&mut self, op: &crate::decode::Op) -> Result<(), Fault> {
        let reg = op.reg as usize;
        let code = op.h as u8;
        match self.loc(op) {
            Rm::Mem(a) => self.fpu_mem(code, reg, a),
            Rm::Reg(i) => self.fpu_reg(code, reg, i),
        }
    }

    fn fpu_mem(&mut self, op: u8, reg: usize, a: u32) -> Result<(), Fault> {
        match op {
            0xD8 | 0xDA | 0xDC | 0xDE => {
                let b = match op {
                    0xD8 => f32::from_bits(self.rd32(a)) as f64,
                    0xDA => self.rd32(a) as i32 as f64,
                    0xDC => f64::from_bits(self.rd64(a)),
                    _ => self.rd16(a) as i16 as f64,
                };
                let st0 = self.fpu.get(0);
                match reg {
                    2 => self.fpu.compare(st0, b),
                    3 => {
                        self.fpu.compare(st0, b);
                        self.fpu.pop();
                    }
                    _ => {
                        let r = self.fpu_arith(reg, st0, b);
                        self.fpu.set(0, r)
                    }
                }
            }
            0xD9 => match reg {
                0 => {
                    let v = f32::from_bits(self.rd32(a)) as f64;
                    self.fpu.push(v)
                }
                2 | 3 => {
                    let v = self.fpu.get(0) as f32;
                    self.wr32(a, v.to_bits());
                    if reg == 3 {
                        self.fpu.pop();
                    }
                }
                4 => self.fload_env(a),
                5 => self.fpu.cw = self.rd16(a),
                6 => self.fsave_env(a),
                7 => {
                    let cw = self.fpu.cw;
                    self.wr16(a, cw)
                }
                _ => return Err(Fault::Unimplemented("D9 mem")),
            },
            0xDB => match reg {
                0 => {
                    let v = self.rd32(a) as i32 as f64;
                    self.fpu.push(v)
                }
                1 | 2 | 3 => {
                    let v = self.fpu.get(0);
                    if reg == 1 {
                        let t = v.trunc();
                        let x = if t.is_nan() || t.abs() > 2147483647.0 { 0x8000_0000 } else { t as i32 as u32 };
                        self.wr32(a, x);
                    } else {
                        self.fist(a, 4, v);
                    }
                    if reg != 2 {
                        self.fpu.pop();
                    }
                }
                5 => {
                    let b: Vec<u8> = self.read_bytes(a, 10).to_vec();
                    self.fpu.push(f80_to_f64(&b))
                }
                7 => {
                    let v = self.fpu.pop();
                    let b = f64_to_f80(v);
                    self.write_bytes(a, &b);
                }
                _ => return Err(Fault::Unimplemented("DB mem")),
            },
            0xDD => match reg {
                0 => {
                    let v = f64::from_bits(self.rd64(a));
                    self.fpu.push(v)
                }
                1 => {
                    let t = self.fpu.get(0).trunc();
                    let x = if t.is_nan() || t.abs() > 9.2e18 { 0x8000_0000_0000_0000 } else { t as i64 as u64 };
                    self.wr64(a, x);
                    self.fpu.pop();
                }
                2 | 3 => {
                    let v = self.fpu.get(0);
                    self.wr64(a, v.to_bits());
                    if reg == 3 {
                        self.fpu.pop();
                    }
                }
                4 => {
                    // FRSTOR
                    self.fload_env(a);
                    for i in 0..8 {
                        let b: Vec<u8> = self.read_bytes(a + 28 + i * 10, 10).to_vec();
                        let k = (self.fpu.top + i as usize) & 7;
                        self.fpu.st[k] = f80_to_f64(&b);
                    }
                }
                6 => {
                    // FNSAVE
                    self.fsave_env(a);
                    for i in 0..8 {
                        let v = self.fpu.get(i);
                        let b = f64_to_f80(v);
                        self.write_bytes(a + 28 + i as u32 * 10, &b);
                    }
                    self.fpu = Fpu::new();
                }
                7 => {
                    let sw = self.fpu.status();
                    self.wr16(a, sw)
                }
                _ => return Err(Fault::Unimplemented("DD mem")),
            },
            0xDF => match reg {
                0 => {
                    let v = self.rd16(a) as i16 as f64;
                    self.fpu.push(v)
                }
                1 => {
                    let t = self.fpu.get(0).trunc();
                    let x = if t.is_nan() || t.abs() > 32767.0 { 0x8000 } else { t as i16 as u16 };
                    self.wr16(a, x);
                    self.fpu.pop();
                }
                2 | 3 => {
                    let v = self.fpu.get(0);
                    self.fist(a, 2, v);
                    if reg == 3 {
                        self.fpu.pop();
                    }
                }
                5 => {
                    let v = self.rd64(a) as i64 as f64;
                    self.fpu.push(v)
                }
                7 => {
                    let v = self.fpu.get(0);
                    self.fist(a, 8, v);
                    self.fpu.pop();
                }
                _ => return Err(Fault::Unimplemented("DF mem (BCD)")),
            },
            _ => unreachable!(),
        }
        Ok(())
    }

    fn fpu_reg(&mut self, op: u8, reg: usize, i: usize) -> Result<(), Fault> {
        let f = &mut self.fpu;
        match op {
            0xD8 => {
                let (a, b) = (f.get(0), f.get(i));
                match reg {
                    2 => f.compare(a, b),
                    3 => {
                        f.compare(a, b);
                        f.pop();
                    }
                    _ => {
                        let r = match reg {
                            0 => a + b,
                            1 => a * b,
                            4 => a - b,
                            5 => b - a,
                            6 => a / b,
                            _ => b / a,
                        };
                        f.set(0, r)
                    }
                }
            }
            0xD9 => match reg {
                0 => {
                    let v = f.get(i);
                    f.push(v)
                }
                1 => {
                    let (a, b) = (f.get(0), f.get(i));
                    f.set(0, b);
                    f.set(i, a);
                }
                2 => {}
                3 => {
                    // FSTP1 (undocumented alias)
                    let v = f.get(0);
                    f.set(i, v);
                    f.pop();
                }
                4 => {
                    let x = f.get(0);
                    match i {
                        0 => f.set(0, -x),
                        1 => f.set(0, x.abs()),
                        4 => f.compare(x, 0.0),
                        5 => {
                            // FXAM
                            let k = (f.top) & 7;
                            let mut c = if x.is_sign_negative() { C1 } else { 0 };
                            c |= if f.empty & (1 << k) != 0 {
                                C3 | C0
                            } else if x.is_nan() {
                                C0
                            } else if x.is_infinite() {
                                C2 | C0
                            } else if x == 0.0 {
                                C3
                            } else {
                                C2
                            };
                            f.set_cc(c);
                        }
                        _ => return Err(Fault::Unimplemented("D9 E0+")),
                    }
                }
                5 => {
                    let v = match i {
                        0 => 1.0,
                        1 => std::f64::consts::LOG2_10,
                        2 => std::f64::consts::LOG2_E,
                        3 => std::f64::consts::PI,
                        4 => std::f64::consts::LOG10_2,
                        5 => std::f64::consts::LN_2,
                        6 => 0.0,
                        _ => return Err(Fault::Unimplemented("D9 EF")),
                    };
                    f.push(v)
                }
                6 => {
                    let x = f.get(0);
                    match i {
                        0 => f.set(0, x.exp2() - 1.0),
                        1 => {
                            let y = f.get(1);
                            f.set(1, y * x.log2());
                            f.pop();
                        }
                        2 => {
                            f.set(0, x.tan());
                            f.push(1.0);
                            f.set_cc(0);
                        }
                        3 => {
                            let y = f.get(1);
                            f.set(1, y.atan2(x));
                            f.pop();
                        }
                        4 => {
                            // FXTRACT
                            if x == 0.0 {
                                f.set(0, f64::NEG_INFINITY);
                                f.push(x);
                            } else {
                                let e = x.abs().log2().floor();
                                f.set(0, e);
                                f.push(x / 2f64.powf(e));
                            }
                        }
                        5 => {
                            let y = f.get(1);
                            let q = (x / y).round_ties_even();
                            f.set(0, x - q * y);
                            f.set_cc(0);
                        }
                        6 => f.top = (f.top + 7) & 7,
                        _ => f.top = (f.top + 1) & 7,
                    }
                }
                _ => {
                    let x = f.get(0);
                    match i {
                        0 => {
                            // FPREM (truncating)
                            let y = f.get(1);
                            let q = (x / y).trunc();
                            f.set(0, x - q * y);
                            let qi = q.abs() as u64;
                            let mut c = 0;
                            if qi & 1 != 0 {
                                c |= C1
                            }
                            if qi & 2 != 0 {
                                c |= C3
                            }
                            if qi & 4 != 0 {
                                c |= C0
                            }
                            f.set_cc(c);
                        }
                        1 => {
                            let y = f.get(1);
                            f.set(1, y * (x + 1.0).log2());
                            f.pop();
                        }
                        2 => f.set(0, x.sqrt()),
                        3 => {
                            f.set(0, x.sin());
                            f.push(x.cos());
                            f.set_cc(0);
                        }
                        4 => {
                            let r = f.round(x);
                            f.set(0, r)
                        }
                        5 => {
                            let y = f.get(1).trunc();
                            f.set(0, x * 2f64.powf(y));
                        }
                        6 => {
                            f.set(0, x.sin());
                            f.set_cc(0);
                        }
                        _ => {
                            f.set(0, x.cos());
                            f.set_cc(0);
                        }
                    }
                }
            },
            0xDA => {
                if reg == 5 && i == 1 {
                    // FUCOMPP
                    let (a, b) = (f.get(0), f.get(1));
                    f.compare(a, b);
                    f.pop();
                    f.pop();
                } else {
                    return Err(Fault::Unimplemented("DA reg"));
                }
            }
            0xDB => match (reg, i) {
                (4, 2) => f.sw &= !0x80FF,
                (4, 3) => *f = Fpu::new(),
                (4, _) => {}
                _ => return Err(Fault::Unimplemented("DB reg")),
            },
            0xDC => {
                let (a, b) = (f.get(i), f.get(0));
                match reg {
                    2 => f.compare(b, a),
                    3 => {
                        f.compare(b, a);
                        f.pop();
                    }
                    _ => {
                        let r = match reg {
                            0 => a + b,
                            1 => a * b,
                            4 => b - a, // FSUBR st(i) = st0 - st(i)
                            5 => a - b, // FSUB  st(i) = st(i) - st0
                            6 => b / a,
                            _ => a / b,
                        };
                        f.set(i, r)
                    }
                }
            }
            0xDD => match reg {
                0 => f.empty |= 1 << ((f.top + i) & 7),
                2 => {
                    let v = f.get(0);
                    f.set(i, v)
                }
                3 => {
                    let v = f.get(0);
                    f.set(i, v);
                    f.pop();
                }
                4 => {
                    let (a, b) = (f.get(0), f.get(i));
                    f.compare(a, b)
                }
                5 => {
                    let (a, b) = (f.get(0), f.get(i));
                    f.compare(a, b);
                    f.pop();
                }
                _ => return Err(Fault::Unimplemented("DD reg")),
            },
            0xDE => {
                if reg == 3 {
                    if i == 1 {
                        let (a, b) = (f.get(0), f.get(1));
                        f.compare(a, b);
                        f.pop();
                        f.pop();
                    } else {
                        return Err(Fault::Unimplemented("DE D8+"));
                    }
                } else if reg == 2 {
                    let (a, b) = (f.get(0), f.get(i));
                    f.compare(a, b);
                    f.pop();
                } else {
                    let (a, b) = (f.get(i), f.get(0));
                    let r = match reg {
                        0 => a + b,
                        1 => a * b,
                        4 => b - a,
                        5 => a - b,
                        6 => b / a,
                        _ => a / b,
                    };
                    f.set(i, r);
                    f.pop();
                }
            }
            0xDF => {
                if reg == 4 && i == 0 {
                    let sw = f.status();
                    self.set_reg16(EAX, sw);
                } else if reg == 0 {
                    // FFREEP
                    f.empty |= 1 << ((f.top + i) & 7);
                    f.pop();
                } else {
                    return Err(Fault::Unimplemented("DF reg"));
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}
