"""Disassembler for CM_HI.EXE (link addresses, labels.csv annotated).

  python tools/dasm.py ADDR|LABEL [count]      # disassemble count instructions (default 60)
  python tools/dasm.py -f LABEL                # whole function (until next label)
  python tools/dasm.py --hist                  # opcode histogram over code object
"""
import sys, os, re, csv, struct
from capstone import Cs, CS_ARCH_X86, CS_MODE_32

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from le import LE

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CODE_LO, CODE_HI = 0x10000, 0x10000 + 0x79368


def load_labels():
    labels = {}
    p = os.path.join(ROOT, 'labels.csv')
    if os.path.exists(p):
        for row in csv.DictReader(open(p)):
            try:
                a = int(row['addr'], 16)
            except ValueError:
                continue
            labels.setdefault(a, row['name'])
    return labels


_le = None


def image():
    global _le
    if _le is None:
        _le = LE()
    return _le


_fields = None


def load_fields():
    global _fields
    if _fields is None:
        _fields = {}
        p = os.path.join(ROOT, 'fields.csv')
        if os.path.exists(p):
            for row in csv.DictReader(open(p)):
                _fields[int(row['offset'], 16)] = row['name']
    return _fields


def annotate(op_str, labels):
    fields = load_fields()

    def rep(m):
        v = int(m.group(0), 16)
        if v in labels:
            return '%s<%s>' % (m.group(0), labels[v])
        return m.group(0)
    op_str = re.sub(r'0x[0-9a-f]{5,8}', rep, op_str)

    def frep(m):
        v = int(m.group(1), 16)
        if v in fields:
            return '+%s]' % fields[v]
        return m.group(0)
    return re.sub(r'\+ 0x([0-9a-f]{4,5})\]', frep, op_str)


def disasm(addr, count=60, until_label=False):
    le = image()
    labels = load_labels()
    md = Cs(CS_ARCH_X86, CS_MODE_32)
    md.skipdata = True
    nxt = min([a for a in labels if a > addr] + [CODE_HI])
    size = (nxt - addr) if until_label else 16 * count
    code = le.read(addr, size)
    n = 0
    out = []
    for ins in md.disasm(code, addr):
        if ins.address in labels and ins.address != addr:
            if until_label:
                break
            out.append('%s:' % labels[ins.address])
        if COMPACT:
            out.append('%05X %s %s' % (ins.address, ins.mnemonic, annotate(ins.op_str, labels)))
        else:
            out.append('  %06X  %-20s %s %s' % (ins.address, ins.bytes.hex(), ins.mnemonic, annotate(ins.op_str, labels)))
        n += 1
        if not until_label and n >= count:
            break
    return '\n'.join(out)


def resolve(s):
    try:
        return int(s, 16)
    except ValueError:
        for a, n in load_labels().items():
            if n == s:
                return a
        raise SystemExit('unknown label ' + s)


COMPACT = False


def main():
    global COMPACT
    a = sys.argv[1:]
    if '-c' in a:
        a.remove('-c')
        COMPACT = True
    if a and a[0] == '--hist':
        le = image()
        md = Cs(CS_ARCH_X86, CS_MODE_32)
        md.skipdata = True
        from collections import Counter
        c = Counter()
        for ins in md.disasm(le.read(CODE_LO, CODE_HI - CODE_LO), CODE_LO):
            pfx = ''
            if ins.bytes[0] == 0x66 or (ins.bytes[0] in (0xF2, 0xF3) and len(ins.bytes) > 1 and ins.bytes[1] == 0x66):
                pfx = 'o16 '
            c[pfx + ins.mnemonic] += 1
        for k, v in sorted(c.items(), key=lambda x: -x[1]):
            print(v, k)
        return
    if a and a[0] == '-f':
        addr = resolve(a[1])
        print(load_labels().get(addr, '') + ':')
        print(disasm(addr, until_label=True))
        return
    addr = resolve(a[0])
    cnt = int(a[1]) if len(a) > 1 else 60
    print(disasm(addr, cnt))


if __name__ == '__main__':
    main()
