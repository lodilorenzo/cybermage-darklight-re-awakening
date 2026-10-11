//! LE executable loader (DOS/4GW format, as used by CM_HI.EXE).
//!
//! Objects are loaded at their link base + `RELOC_DELTA`, keeping the first
//! megabyte free for conventional memory / VGA like under a real extender.

pub const RELOC_DELTA: u32 = 0x40_0000;

/// Link address (as in labels.csv / debug info) -> runtime linear address.
#[inline]
pub const fn rt(link: u32) -> u32 {
    link + RELOC_DELTA
}
/// Runtime linear address -> link address.
#[inline]
pub const fn link(rt: u32) -> u32 {
    rt.wrapping_sub(RELOC_DELTA)
}

pub struct LeObject {
    pub base: u32,
    pub vsize: u32,
    pub flags: u32,
}

pub struct LeImage {
    pub objects: Vec<LeObject>,
    pub entry: u32,
    pub stack: u32,
    /// (runtime base, bytes) of each object's initialised contents
    pub data: Vec<(u32, Vec<u8>)>,
}

fn u32at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}
fn u16at(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}

pub fn load(d: &[u8]) -> Result<LeImage, String> {
    let le = u32at(d, 0x3C) as usize;
    if &d[le..le + 2] != b"LE" {
        return Err("not an LE executable".into());
    }
    let h = |o: usize| u32at(d, le + o);
    let page_size = h(0x28) as usize;
    let last_page = h(0x2C) as usize;
    let npages = h(0x14) as usize;
    let nobj = h(0x44) as usize;
    let objtab = h(0x40) as usize;
    let objpagetab = h(0x48) as usize;
    let fixpage = h(0x68) as usize;
    let fixrec = h(0x6C) as usize;
    let data_pages = h(0x80) as usize;

    let mut objs = Vec::new();
    let mut raw = Vec::new();
    for k in 0..nobj {
        let o = le + objtab + k * 24;
        let vsize = u32at(d, o);
        let base = u32at(d, o + 4) + RELOC_DELTA;
        let flags = u32at(d, o + 8);
        let pmi = u32at(d, o + 12) as usize;
        let pmc = u32at(d, o + 16) as usize;
        objs.push(LeObject { base, vsize, flags });
        raw.push((pmi, pmc));
    }
    let mut data = Vec::new();
    for (k, &(pmi, pmc)) in raw.iter().enumerate() {
        let mut buf = vec![0u8; pmc * page_size];
        for i in 0..pmc {
            let pg = pmi + i;
            let e = le + objpagetab + (pg - 1) * 4;
            let num = ((d[e] as usize) << 16) | ((d[e + 1] as usize) << 8) | d[e + 2] as usize;
            let off = data_pages + (num - 1) * page_size;
            let size = if pg == npages { last_page } else { page_size };
            buf[i * page_size..i * page_size + size].copy_from_slice(&d[off..off + size]);
        }
        // fixups for this object's pages
        for i in 0..pmc {
            let pg = pmi + i;
            let a = u32at(d, le + fixpage + (pg - 1) * 4) as usize;
            let b = u32at(d, le + fixpage + pg * 4) as usize;
            let mut p = le + fixrec + a;
            let end = le + fixrec + b;
            while p < end {
                let src = d[p];
                let flg = d[p + 1];
                p += 2;
                let (cnt, single) = if src & 0x20 != 0 {
                    let c = d[p] as usize;
                    p += 1;
                    (c, 0i16)
                } else {
                    let so = u16at(d, p) as i16;
                    p += 2;
                    (0, so)
                };
                if flg & 3 != 0 {
                    return Err(format!("unsupported fixup flags {flg:#x}"));
                }
                let tobj = if flg & 0x40 != 0 {
                    let t = u16at(d, p) as usize;
                    p += 2;
                    t
                } else {
                    let t = d[p] as usize;
                    p += 1;
                    t
                };
                let stype = src & 0xF;
                let toff = if stype == 2 {
                    0
                } else if flg & 0x10 != 0 {
                    let t = u32at(d, p);
                    p += 4;
                    t
                } else {
                    let t = u16at(d, p) as u32;
                    p += 2;
                    t
                };
                let mut offs = Vec::new();
                if src & 0x20 != 0 {
                    for _ in 0..cnt {
                        offs.push(u16at(d, p) as i16);
                        p += 2;
                    }
                } else {
                    offs.push(single);
                }
                let target = objs[tobj - 1].base.wrapping_add(toff);
                for so in offs {
                    let at = (i * page_size) as isize + so as isize;
                    if stype != 7 {
                        return Err(format!("unsupported fixup type {stype}"));
                    }
                    let bytes = target.to_le_bytes();
                    for (j, &bt) in bytes.iter().enumerate() {
                        let q = at + j as isize;
                        if q >= 0 && (q as usize) < buf.len() {
                            buf[q as usize] = bt;
                        }
                    }
                }
            }
        }
        data.push((objs[k].base, buf));
    }
    let eip_obj = h(0x18) as usize;
    let esp_obj = h(0x20) as usize;
    Ok(LeImage {
        entry: objs[eip_obj - 1].base + h(0x1C),
        stack: objs[esp_obj - 1].base + h(0x24),
        objects: objs,
        data,
    })
}
