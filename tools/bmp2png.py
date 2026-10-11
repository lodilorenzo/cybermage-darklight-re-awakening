"""Convert out/*.bmp screenshots to PNG: python tools/bmp2png.py [names...]"""
import sys, os, glob
from PIL import Image
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
names = sys.argv[1:] or [os.path.basename(p)[:-4] for p in glob.glob(os.path.join(ROOT, 'out', '*.bmp'))]
for n in names:
    Image.open(os.path.join(ROOT, 'out', n + '.bmp')).save(os.path.join(ROOT, 'out', n + '.png'))
    print(n)
