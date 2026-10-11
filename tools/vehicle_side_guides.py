"""Flat-colour layout guides for regenerating vehicle side art (image-model edit input).
Shapes follow the user's markup of the first versions: the tank's ammo bulkhead keeps its
straight top/bottom edges up to one continuous hull rib, whose foot passes behind the
console plate's slanted top edge; the aircar's dashboard lip and blue cowl continue
outward at the original height, with diagonal bevel creases instead of grey consoles.
  python tools/vehicle_side_guides.py [PREVIOUS_CAR_LEFT PREVIOUS_CAR_RIGHT]
Writes assets/vehicle-sides/{tank-left,car-left,car-right}-guide2.png (1536x1024).
"""
from pathlib import Path
import sys
from PIL import Image, ImageDraw
from prepare_vehicle_sides import ROOT, SIDE, CONTEXT, cockpit, registered

S = 4  # draw at 4x backscreen resolution for clean diagonals
OUT = ROOT / "assets/vehicle-sides"


def canvas_with_strip(vehicle, side, layout):
    image, _ = cockpit(vehicle)
    ref = Image.new("RGB", ((SIDE + CONTEXT) * S, 240 * S))
    if side == "left":
        ref.paste(layout, (0, 0))
        ref.paste(image.crop((0, 0, CONTEXT, 240)).resize((CONTEXT * S, 240 * S), Image.Resampling.NEAREST), (SIDE * S, 0))
    else:
        ref.paste(image.crop((320 - CONTEXT, 0, 320, 240)).resize((CONTEXT * S, 240 * S), Image.Resampling.NEAREST), (0, 0))
        ref.paste(layout, (CONTEXT * S, 0))
    return ref.resize((1536, 1024), Image.Resampling.LANCZOS)


def pts(*p):
    return [(x * S, y * S) for x, y in p]


def tank_left():
    # Side-canvas coordinates: x=267 touches the original's column 0.
    g = Image.new("RGB", (SIDE * S, 240 * S), (22, 24, 30))      # hull side wall
    d = ImageDraw.Draw(g)
    d.rectangle((0, 0, SIDE * S, 36 * S), fill=(12, 12, 16))      # ceiling
    # Ammo bulkhead: same rows as the original (38..147), up to the rib.
    d.rectangle((130 * S, 38 * S, SIDE * S, 147 * S), fill=(74, 76, 84))
    d.rectangle((130 * S, 92 * S, SIDE * S, 147 * S), fill=(62, 64, 72))   # lower plate, as original
    for y in (42, 88, 143):
        for x in range(150, SIDE, 20):
            d.ellipse(pts((x - 1, y - 1), (x + 1, y + 1)), fill=(150, 150, 160))  # rivet rows
    # Vent band under the bulkhead (original rows 148..175).
    d.rectangle((130 * S, 148 * S, SIDE * S, 176 * S), fill=(40, 42, 48))
    for y in range(156, 174, 4):
        d.rectangle((140 * S, y * S, SIDE * S, (y + 2) * S), fill=(96, 98, 104))
    # Continuous hull rib: right edge (151,0)->(134,43)->(134,115)->(197,238), 26 px wide.
    edge = [(151, 0), (134, 43), (134, 115), (197, 238)]
    rib = edge + [(x - 26, y) for x, y in reversed(edge)]
    d.polygon(pts(*rib), fill=(112, 116, 128))
    d.line(pts(*edge), fill=(170, 174, 186), width=2 * S)                   # lit edge
    for x, y in ((129, 30), (121, 60), (121, 100), (127, 135), (146, 175)):
        d.ellipse(pts((x - 1, y - 1), (x + 1, y + 1)), fill=(190, 190, 200))
    # Console plate in front of the rib foot: top edge continues the original
    # light plate (row ~176 at the strip) down-left to (143,240).
    d.polygon(pts((SIDE, 176), (143, 240), (SIDE, 240)), fill=(150, 150, 156))
    d.line(pts((SIDE, 176), (143, 240)), fill=(205, 205, 210), width=2 * S)
    d.polygon(pts((SIDE, 196), (190, 240), (SIDE, 240)), fill=(70, 72, 78))  # machinery below the plate
    return g


def car(side, previous):
    """Approved canopy from the previous version above the lip; new dashboard below."""
    top = registered("car", side, previous, over=S)
    x0 = 0 if side == "left" else CONTEXT * S
    g = top.crop((x0, 0, x0 + SIDE * S, 240 * S))
    d = ImageDraw.Draw(g)
    m = (lambda x: x) if side == "left" else (lambda x: SIDE - x)  # mirror for the right side
    P = lambda *p: [(m(x) * S, y * S) for x, y in p]
    # Lip: the original's outermost column (rows 144..167) carried outward, so it keeps
    # the exact height and colours where it meets the strip.
    image, _ = cockpit("car")
    edge = 0 if side == "left" else 319
    for y in range(144, 168):
        d.rectangle((0, y * S, SIDE * S, (y + 1) * S - 1), fill=image.getpixel((edge, y)))
    d.rectangle((0, 168 * S, SIDE * S, 240 * S), fill=(24, 36, 140))      # cowl top surface
    # Bevel facet between the two creases, then the dark cowl side at the junction
    # (the original's edge is dark navy below row 168).
    d.polygon(P((SIDE, 168), (247, 182), (180, 226), (150, 240), (200, 240), (SIDE, 198)), fill=(14, 22, 90))
    d.polygon(P((SIDE, 198), (200, 240), (SIDE, 240)), fill=(8, 12, 48))
    d.line(P((SIDE, 168), (247, 182), (180, 226), (150, 240)), fill=(70, 100, 220), width=S)
    d.line(P((SIDE, 198), (200, 240)), fill=(50, 80, 190), width=S)
    return g


if __name__ == "__main__":
    canvas_with_strip("tank", "left", tank_left()).save(OUT / "tank-left-guide2.png")
    if len(sys.argv) == 3:
        for side, prev in zip(("left", "right"), sys.argv[1:]):
            canvas_with_strip("car", side, car(side, Path(prev))).save(OUT / f"car-{side}-guide2.png")
    print("guides written to", OUT)
