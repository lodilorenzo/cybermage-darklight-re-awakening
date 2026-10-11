"""Collect object field names from the native Rust ports into fields.csv.

Scans crates/game/src/native/**/*.rs for `const NAME: u32 = 0xOFFSET;` with
OFFSET < 0x100000 (offsets into the Image1/Xox object).
"""
import os, re, glob
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
fields = {}
for f in glob.glob(os.path.join(ROOT, 'crates', 'game', 'src', 'native', '**', '*.rs'), recursive=True):
    for m in re.finditer(r'const ([A-Z0-9_]+): u32 = 0x([0-9A-Fa-f_]+);', open(f).read()):
        off = int(m.group(2).replace('_', ''), 16)
        if 0x1000 <= off < 0x100000:
            fields.setdefault(off, m.group(1))
with open(os.path.join(ROOT, 'fields.csv'), 'w') as f:
    f.write('offset,name\n')
    for off in sorted(fields):
        f.write('0x%X,%s\n' % (off, fields[off]))
print(len(fields), 'fields')
