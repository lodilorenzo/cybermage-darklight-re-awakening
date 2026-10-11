"""Host-side hotspot analysis from an xperf dumper output.

  python tools/hotspots.py scratch/dump.txt [N]

Aggregates SampledProfile PCs inside cybermage.exe by RVA and disassembles
the hottest region with llvm-objdump.
"""
import sys, os, re, subprocess, collections, glob

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
dump = sys.argv[1]
topn = int(sys.argv[2]) if len(sys.argv) > 2 else 40
base = None
hist = collections.Counter()
for line in open(dump, errors='replace'):
    if base is None and 'ImageId' in line and '"cybermage.exe"' in line:
        base = int(line.split(',')[3].strip(), 16)
    if line.lstrip().startswith('SampledProfile,') and 'cybermage.exe (' in line:
        parts = [p.strip() for p in line.split(',')]
        pc = int(parts[4], 16)
        if parts[7].startswith('cybermage.exe!'):
            hist[pc] += 1
total = sum(hist.values())
print('base %x samples %d' % (base, total))
rvas = collections.Counter()
for pc, n in hist.items():
    rvas[pc - base] += n
# cluster into 64-byte buckets
buckets = collections.Counter()
for r, n in rvas.items():
    buckets[r & ~63] += n
for b, n in buckets.most_common(topn):
    print('%08x %5.2f%%' % (b, 100.0 * n / total))
exe = os.path.join(ROOT, 'target', 'release', 'cybermage.exe')
objdump = glob.glob(os.path.expanduser('~/.rustup/toolchains/stable-*/lib/rustlib/*/bin/llvm-objdump.exe'))[0]
out = os.path.join(ROOT, 'scratch', 'hot_asm.txt')
with open(out, 'w') as f:
    for b, n in buckets.most_common(12):
        va = 0x140000000 + b
        f.write('==== bucket %08x %.2f%%\n' % (b, 100.0 * n / total))
        r = subprocess.run([objdump, '-d', '--no-show-raw-insn', '-M', 'intel',
                            '--start-address=0x%x' % (va - 32), '--stop-address=0x%x' % (va + 96), exe],
                           capture_output=True, text=True)
        for l in r.stdout.splitlines():
            m = re.match(r'\s*([0-9a-f]+):', l)
            if m:
                a = int(m.group(1), 16) - 0x140000000
                f.write('%5d %s\n' % (rvas.get(a, 0), l))
print('wrote', out)
