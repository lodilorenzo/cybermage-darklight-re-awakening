"""Watcom debug info (v3) parser for CM_HI.EXE.

Usage:
  python tools/wdbg.py modules
  python tools/wdbg.py globals [-g regex]
  python tools/wdbg.py labels          # write labels.csv from globals
  python tools/wdbg.py lines MODNAME
  python tools/wdbg.py raw MODNAME locals|types|lines   # hexdump demand blocks
"""
import struct, sys, re, os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, 'iso', 'CYBERMAG', 'CM_HI.EXE')

# Object bases (from LE object table): obj1 code @0x10000, obj2 data @0x90000
OBJ_BASE = {1: 0x10000, 2: 0x90000}


class Dbg:
    def __init__(self, path=EXE):
        d = open(path, 'rb').read()
        self.d = d
        sig, emaj, emin, omaj, omin, lang_size, seg_size, dsize = struct.unpack_from('<HBBBBHHI', d, len(d) - 14)
        assert sig == 0x8386, hex(sig)
        self.ver = (emaj, emin)
        start = len(d) - dsize
        self.langs = d[start:start + lang_size].split(b'\0')[:-1]
        segs = struct.unpack_from('<%dH' % (seg_size // 2), d, start + lang_size)
        self.segs = segs
        s = start + lang_size + seg_size
        self.sec = s
        self.mod_off, self.gbl_off, self.addr_off, self.sec_size, self.sec_id = struct.unpack_from('<IIIIH', d, s)
        self.parse_modules()
        self.parse_globals()

    def name_at(self, p):
        n = self.d[p]
        return self.d[p + 1:p + 1 + n].decode('latin1'), p + 1 + n

    def parse_modules(self):
        d, s = self.d, self.sec
        p = s + self.mod_off
        end = s + self.gbl_off
        self.mods = []
        while p < end:
            lang, = struct.unpack_from('<H', d, p)
            di = []
            q = p + 2
            for k in range(3):
                off, num = struct.unpack_from('<IH', d, q)
                di.append((off, num))
                q += 6
            name, q = self.name_at(q)
            self.mods.append(dict(lang=self.langs[lang].decode() if lang < len(self.langs) else lang,
                                  locals=di[0], types=di[1], lines=di[2], name=name))
            p = q

    def parse_globals(self):
        d, s = self.d, self.sec
        p = s + self.gbl_off
        end = s + self.addr_off
        self.gbls = []
        while p < end:
            off, seg, mod, kind = struct.unpack_from('<IHHB', d, p)
            name, p = self.name_at(p + 9)
            self.gbls.append(dict(off=off, seg=seg, mod=mod, kind=kind, name=name,
                                  lin=OBJ_BASE.get(seg, 0) + off))

    def demand_blocks(self, mod, kind):
        """Return list of (start,end) byte ranges for demand blocks of a module."""
        off, num = self.mods[mod][kind]
        if num == 0:
            return []
        d, s = self.d, self.sec
        # Table of num+1 u32 offsets relative to section start
        tbl = struct.unpack_from('<%dI' % (num + 1), d, s + off)
        return [(s + tbl[i], s + tbl[i + 1]) for i in range(num)]


def demangle(n):
    """Very small Watcom C++ name prettifier: W?name$n(args)ret -> name."""
    m = re.match(r'W\?([^$]+)\$', n)
    if m:
        base = m.group(1)
        cls = re.search(r'\$:([^$]+)\$', n)
        if cls:
            return cls.group(1) + '::' + base
        return base
    return n.rstrip('_') if n.endswith('_') and not n.startswith('_') else n


def main():
    db = Dbg()
    cmd = sys.argv[1] if len(sys.argv) > 1 else 'modules'
    if cmd == 'modules':
        print('ver', db.ver, 'langs', db.langs, 'segs', db.segs)
        for i, m in enumerate(db.mods):
            print(i, m['lang'], m['name'], 'loc', m['locals'], 'typ', m['types'], 'lin', m['lines'])
    elif cmd == 'globals':
        pat = re.compile(sys.argv[3]) if len(sys.argv) > 3 and sys.argv[2] == '-g' else None
        for g in sorted(db.gbls, key=lambda g: g['lin']):
            if pat and not pat.search(g['name']):
                continue
            print('0x%06X seg%d k%d %-14s %s' % (g['lin'], g['seg'], g['kind'], db.mods[g['mod']]['name'][-14:], g['name']))
    elif cmd == 'labels':
        # Canonical address notation: linear address as loaded by DOS4GW at LE object bases.
        with open(os.path.join(ROOT, 'labels.csv'), 'w') as f:
            f.write('addr,name,comment\n')
            for g in sorted(db.gbls, key=lambda g: g['lin']):
                mod = os.path.basename(db.mods[g['mod']]['name'].replace('\\', '/'))
                f.write('0x%06X,%s,%s|k%d|%s\n' % (g['lin'], demangle(g['name']), mod, g['kind'], g['name'].replace(',', ';')))
        print('wrote', len(db.gbls))
    elif cmd == 'code':
        # code symbols with size estimate (distance to next symbol)
        code = sorted([g for g in db.gbls if g['seg'] == 1], key=lambda g: g['lin'])
        end = 0x10000 + 0x79368
        for i, g in enumerate(code):
            nxt = code[i + 1]['lin'] if i + 1 < len(code) else end
            mod = db.mods[g['mod']]['name'].replace(chr(92), '/').split('/')[-1]
            print('%06X %6d %-14s %s' % (g['lin'], nxt - g['lin'], mod[:14], g['name']))
    elif cmd == 'raw':
        mi = next(i for i, m in enumerate(db.mods) if sys.argv[2].lower() in m['name'].lower())
        for a, b in db.demand_blocks(mi, sys.argv[3]):
            print('block', hex(a), b - a)
            blk = db.d[a:b]
            for i in range(0, min(len(blk), 512), 16):
                print('  %04x %-48s %s' % (i, blk[i:i+16].hex(' '), re.sub(rb'[^\x20-\x7e]', b'.', blk[i:i+16]).decode()))


if __name__ == '__main__':
    main()
