"""LE (DOS4GW) executable loader for CM_HI.EXE.

Builds a flat memory image with fixups applied, using the link-time object
bases (code 0x10000, data 0x90000) so addresses match the debug symbols.

  python tools/le.py info
  python tools/le.py dump OUT.bin      # writes image starting at 0x10000
"""
import struct, sys, os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, 'iso', 'CYBERMAG', 'CM_HI.EXE')


class LE:
    def __init__(self, path=EXE, bases=None):
        d = open(path, 'rb').read()
        self.d = d
        le = struct.unpack_from('<I', d, 0x3C)[0]
        assert d[le:le + 2] == b'LE'
        self.le = le
        u32 = lambda o: struct.unpack_from('<I', d, le + o)[0]
        self.page_size = u32(0x28)
        self.last_page = u32(0x2C)
        self.npages = u32(0x14)
        self.eip_obj, self.eip = u32(0x18), u32(0x1C)
        self.esp_obj, self.esp = u32(0x20), u32(0x24)
        nobj, objtab = u32(0x44), u32(0x40)
        self.objpagetab = u32(0x48)
        self.fixpage = u32(0x68)
        self.fixrec = u32(0x6C)
        self.data_pages = u32(0x80)
        self.objs = []
        for k in range(nobj):
            vs, base, flags, pmi, pmc, _ = struct.unpack_from('<6I', d, le + objtab + k * 24)
            self.objs.append(dict(vsize=vs, base=base, flags=flags, pmi=pmi, pmc=pmc))
        if bases:
            for o, b in zip(self.objs, bases):
                o['base'] = b
        self.fixups = []  # (addr, kind, value)
        self.build()

    def page_file_off(self, page):  # page is 1-based
        # LE object page table entry: 4 bytes: 3-byte page number (big-endian-ish), 1 byte flags
        e = self.d[self.le + self.objpagetab + (page - 1) * 4: self.le + self.objpagetab + page * 4]
        num = (e[0] << 16) | (e[1] << 8) | e[2]
        return self.data_pages + (num - 1) * self.page_size, e[3]

    def build(self):
        lo = min(o['base'] for o in self.objs)
        hi = max(o['base'] + o['vsize'] for o in self.objs)
        self.lo = lo
        self.mem = bytearray(hi - lo)
        for o in self.objs:
            for i in range(o['pmc']):
                pg = o['pmi'] + i
                off, fl = self.page_file_off(pg)
                size = self.page_size if pg != self.npages else self.last_page
                dst = o['base'] + i * self.page_size - lo
                self.mem[dst:dst + size] = self.d[off:off + size]
        # fixups
        d = self.d
        le = self.le
        for o in self.objs:
            for i in range(o['pmc']):
                pg = o['pmi'] + i
                a, b = struct.unpack_from('<II', d, le + self.fixpage + (pg - 1) * 4)
                p = le + self.fixrec + a
                end = le + self.fixrec + b
                page_base = o['base'] + i * self.page_size
                while p < end:
                    src, flg = d[p], d[p + 1]
                    p += 2
                    if src & 0x20:
                        cnt = d[p]; p += 1
                        srcoffs = None
                    else:
                        srcoff = struct.unpack_from('<h', d, p)[0]; p += 2
                    ftype = flg & 3
                    assert ftype == 0, 'only internal fixups supported (flg=%x)' % flg
                    if flg & 0x40:
                        tobj = struct.unpack_from('<H', d, p)[0]; p += 2
                    else:
                        tobj = d[p]; p += 1
                    stype = src & 0xF
                    if stype == 2:  # 16-bit selector fixup: no target offset
                        toff = 0
                    elif flg & 0x10:
                        toff = struct.unpack_from('<I', d, p)[0]; p += 4
                    else:
                        toff = struct.unpack_from('<H', d, p)[0]; p += 2
                    if src & 0x20:
                        srcoffs = struct.unpack_from('<%dh' % cnt, d, p); p += 2 * cnt
                    else:
                        srcoffs = [srcoff]
                    tgt = self.objs[tobj - 1]['base'] + toff
                    for so in srcoffs:
                        addr = page_base + so
                        self.apply(addr, stype, tgt, tobj)
        self.entry = self.objs[self.eip_obj - 1]['base'] + self.eip
        self.stack = self.objs[self.esp_obj - 1]['base'] + self.esp

    def apply(self, addr, stype, tgt, tobj):
        m = addr - self.lo
        self.fixups.append((addr, stype, tgt, tobj))
        if stype == 7:  # 32-bit offset
            if 0 <= m <= len(self.mem) - 4:
                struct.pack_into('<I', self.mem, m, tgt & 0xFFFFFFFF)
            elif 0 <= m < len(self.mem):  # straddles page end handled by next page's record
                v = struct.pack('<I', tgt & 0xFFFFFFFF)
                n = len(self.mem) - m
                self.mem[m:m + n] = v[:n]
        elif stype == 8:  # 32-bit self-relative
            v = (tgt - (addr + 4)) & 0xFFFFFFFF
            struct.pack_into('<I', self.mem, m, v)
        elif stype == 2:  # selector
            pass  # handled at runtime (flat selectors)
        elif stype == 5:  # 16-bit offset
            struct.pack_into('<H', self.mem, m, tgt & 0xFFFF)
        else:
            raise ValueError('fixup type %d at %x' % (stype, addr))

    def read(self, addr, n):
        m = addr - self.lo
        return bytes(self.mem[m:m + n])


def main():
    le = LE()
    cmd = sys.argv[1] if len(sys.argv) > 1 else 'info'
    if cmd == 'info':
        for i, o in enumerate(le.objs):
            print('obj%d base=%06X vsize=%06X flags=%04X pages=%d..%d' % (i + 1, o['base'], o['vsize'], o['flags'], o['pmi'], o['pmi'] + o['pmc'] - 1))
        print('entry %06X stack %06X fixups %d' % (le.entry, le.stack, len(le.fixups)))
        from collections import Counter
        print('fixup types', Counter(f[1] for f in le.fixups))
        print('fixup types by target obj', Counter((f[1], f[3]) for f in le.fixups))
    elif cmd == 'dump':
        open(sys.argv[2], 'wb').write(le.mem)
        print('wrote %d bytes, base %06X' % (len(le.mem), le.lo))


if __name__ == '__main__':
    main()
