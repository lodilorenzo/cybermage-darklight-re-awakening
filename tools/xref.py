"""Find references to an address in the relocated image (link addresses).

  python tools/xref.py ADDR|LABEL [--data]

Finds: rel32 calls/jumps targeting ADDR, and absolute 32-bit occurrences
(fixup targets / immediates / pointers in data).  Reports containing function.
"""
import sys, os, struct, bisect
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from le import LE
from dasm import load_labels, resolve

CODE_LO, CODE_HI = 0x10000, 0x10000 + 0x79368


def main():
    target = resolve(sys.argv[1])
    le = LE()
    labels = load_labels()
    addrs = sorted(labels)

    def owner(a):
        i = bisect.bisect_right(addrs, a) - 1
        return '%s+%X' % (labels[addrs[i]], a - addrs[i]) if i >= 0 else '?'

    mem = le.mem
    lo = le.lo
    hits = []
    # absolute
    pat = struct.pack('<I', target)
    i = mem.find(pat)
    while i >= 0:
        hits.append((lo + i, 'abs'))
        i = mem.find(pat, i + 1)
    # rel32 call/jmp/jcc
    code = mem[CODE_LO - lo:CODE_HI - lo]
    for k in range(len(code) - 5):
        op = code[k]
        if op in (0xE8, 0xE9):
            rel = struct.unpack_from('<i', code, k + 1)[0]
            if CODE_LO + k + 5 + rel == target:
                hits.append((CODE_LO + k, 'call' if op == 0xE8 else 'jmp'))
        elif op == 0x0F and 0x80 <= code[k + 1] <= 0x8F and k + 6 <= len(code):
            rel = struct.unpack_from('<i', code, k + 2)[0]
            if CODE_LO + k + 6 + rel == target:
                hits.append((CODE_LO + k, 'jcc'))
    for a, kind in sorted(hits):
        print('%06X %-4s %s' % (a, kind, owner(a)))


if __name__ == '__main__':
    main()
