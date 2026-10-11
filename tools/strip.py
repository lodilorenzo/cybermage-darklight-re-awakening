"""Make a thumbnail strip: python tools/strip.py OUT.png shot_0001 shot_0002 ..."""
import sys, os
from PIL import Image
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
out = sys.argv[1]
ims = []
for n in sys.argv[2:]:
    p = os.path.join(ROOT, 'out', n + '.bmp')
    ims.append(Image.open(p).convert('RGB').resize((320, 240)))
cols = min(4, len(ims))
rows = (len(ims) + cols - 1) // cols
W = Image.new('RGB', (320 * cols, 240 * rows))
for k, im in enumerate(ims):
    W.paste(im, (320 * (k % cols), 240 * (k // cols)))
W.save(os.path.join(ROOT, 'out', out))
