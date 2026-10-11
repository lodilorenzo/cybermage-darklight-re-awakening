"""Convert controller-help artwork with Pillow; no runtime image dependency.
PNG references: python tools/prepare_controller_help.py input.bmp reference.png
Embedded RGB: python tools/prepare_controller_help.py assets/controller-help.png assets/controller-help.rgb
"""
from pathlib import Path
import sys
from PIL import Image

source, target = map(Path, sys.argv[1:3])
image = Image.open(source)
alpha = image.getchannel("A").getextrema() if "A" in image.getbands() else None
print(f"Source: {image.size}, {image.mode}, alpha={alpha}")
assert alpha is None or alpha == (255, 255), "Help artwork must be opaque"
image = image.convert("RGB")
if target.suffix == ".rgb":
    assert image.width * 3 == image.height * 4, "Help artwork must be 4:3"
    image = image.resize((1280, 960), Image.Resampling.NEAREST)
    target.write_bytes(image.tobytes())
    assert target.stat().st_size == 1280 * 960 * 3
else:
    image.save(target)
print(f"{target}: {image.width}x{image.height}, RGB")
