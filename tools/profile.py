"""Aggregate out/profile.txt (EIP samples) by function using labels.csv.
  python tools/profile.py [top] [profile.txt]"""
import sys, os, bisect
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from dasm import load_labels
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
labels = load_labels()
# only code labels
addrs = sorted(a for a in labels if a < 0x90000)
agg = {}
tot = 0
for line in open(sys.argv[2] if len(sys.argv) > 2 else os.path.join(ROOT, 'out', 'profile.txt')):
    a, n = line.split()
    a = int(a, 16); n = int(n)
    tot += n
    i = bisect.bisect_right(addrs, a) - 1
    name = labels[addrs[i]] if i >= 0 and a < 0x90000 else 'outside(%X)' % (a & ~0xFFF)
    agg[name] = agg.get(name, 0) + n
top = int(sys.argv[1]) if len(sys.argv) > 1 else 40
for name, n in sorted(agg.items(), key=lambda x: -x[1])[:top]:
    print('%6.2f%%  %s' % (100.0 * n / tot, name))
