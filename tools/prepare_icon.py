"""Rebuild the Windows icon from the existing commercial Darklight logo.
Run: python tools/prepare_icon.py (requires Pillow).
Artwork retains its original rights; see THIRD_PARTY_NOTICE.md.
"""
from pathlib import Path
from PIL import Image, ImageDraw

PROJECT = Path(__file__).resolve().parents[1]
SIZES = (16, 24, 32, 48, 64, 128, 256)


def main():
    shield = Image.open(PROJECT / "docs/img/title.png").convert("RGBA").crop((450, 67, 506, 158))
    mask = Image.new("L", shield.size)
    ImageDraw.Draw(mask).polygon(
        [(13, 0), (31, 18), (47, 3), (52, 25), (55, 42), (52, 67),
         (45, 72), (35, 79), (30, 90), (21, 85), (13, 76), (8, 58), (6, 34), (8, 16)], fill=255)
    shield.putalpha(mask)
    shield = shield.resize((144, 234), Image.Resampling.NEAREST)
    icon = Image.new("RGBA", (256, 256))
    icon.alpha_composite(shield, (56, 11))
    path = PROJECT / "assets/cybermage.ico"
    icon.save(path, sizes=[(n, n) for n in SIZES])
    with Image.open(path) as check:
        assert check.ico.sizes() == {(n, n) for n in SIZES}
        assert check.convert("RGBA").getextrema()[3] == (0, 255)
    print(f"Created {path} ({', '.join(map(str, SIZES))} px, transparent)")


if __name__ == "__main__":
    main()
