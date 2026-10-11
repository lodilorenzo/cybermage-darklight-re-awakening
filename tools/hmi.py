"""HMI-MIDISONG061595 inspector.

  python tools/hmi.py FILE [--events N]

Song header: 0xD4 i16 tick rate (Hz), 0xE4 i16 track count, 0xE8 u32 track
offset table.  Track header ("HMI-MIDITRACK"): 0x57 u32 data offset,
0x7B i16 MIDI channel.  Note-on events carry a VLQ duration (no note-offs).
FE meta events (sub 0x10..0x16) implement text, branches and loops.
"""
import sys, struct
from collections import Counter

d = open(sys.argv[1], 'rb').read()
nev = int(sys.argv[3]) if len(sys.argv) > 3 else 0
u16 = lambda o: struct.unpack_from('<H', d, o)[0]
i16 = lambda o: struct.unpack_from('<h', d, o)[0]
u32 = lambda o: struct.unpack_from('<I', d, o)[0]
print('magic', d[:18], 'rate', i16(0xD4), 'chans?', i16(0xE2), 'tracks', i16(0xE4), 'table', hex(u32(0xE8)))
for k in (0x22, 0xD6, 0xDA, 0xF4):
    print('  ptr %02X = %X' % (k, u32(k)))
ntr = i16(0xE4)
tab = u32(0xE8)


def vlq(p):
    v = 0
    while True:
        b = d[p]; p += 1
        v = (v << 7) | (b & 0x7F)
        if not b & 0x80:
            return v, p


for t in range(ntr):
    off = u32(tab + 4 * t)
    hdr = d[off:off + 0x99]
    data = off + u32(off + 0x57)
    chan = i16(off + 0x7B)
    print('track %d @%X data %X chan %d id %d  f20=%02X f21=%02X 4B=%X 63=%X 7D=%X' % (
        t, off, data, chan, i16(off + 0x5F), hdr[0x20], hdr[0x21], u32(off + 0x4B), u32(off + 0x63), u32(off + 0x7D)))
    p = data
    delta, p = vlq(p)
    run = 0
    kinds = Counter()
    n = 0
    tick = delta
    while p < len(d):
        st = d[p]
        if st >= 0x80:
            p += 1
            run = st
        else:
            st = run
        hi = st & 0xF0
        desc = ''
        if hi == 0x90:
            key, vel = d[p], d[p + 1]; p += 2
            dur, p = vlq(p)
            desc = 'on %d %d dur %d' % (key, vel, dur)
        elif hi in (0x80, 0xA0, 0xB0, 0xE0):
            a, b = d[p], d[p + 1]; p += 2
            desc = '%02X %d %d' % (hi, a, b)
        elif hi in (0xC0, 0xD0):
            a = d[p]; p += 1
            desc = '%02X %d' % (hi, a)
        elif st == 0xFF:
            typ = d[p]; ln = d[p + 1]
            desc = 'meta %02X len %d %r' % (typ, ln, d[p + 2:p + 2 + ln])
            p += 2 + ln
            if typ == 0x2F:
                kinds['end'] += 1
                if n < nev:
                    print('    %6d %s' % (tick, desc))
                break
        elif st == 0xFE:
            sub = d[p]
            desc = 'FE %02X %s' % (sub, d[p:p + 12].hex(' '))
            if sub == 0x10:
                p += d[p + 3] + 8  # TODO verify
            elif sub in (0x12, 0x14):
                p += 3
            elif sub in (0x13, 0x15):
                p += 11
            else:
                p += 7
        else:
            desc = 'status %02X ?' % st
            kinds['?'] += 1
            break
        kinds[desc.split()[0] + ('' if not desc.startswith('B0') else ' cc%s' % desc.split()[1])] += 1
        if n < nev:
            print('    %6d %s' % (tick, desc))
        n += 1
        delta, p = vlq(p)
        tick += delta
    print('   events', n, 'ticks', tick, dict(kinds))
