"""Widescreen side art for the tank and aircar cockpits.
Original backscreens are 320x240 palette images in HIPAK.RSC, doubled by
Xox::LoadBackScreen (0x01FBF0); index 0 is the 3D window. Side art covers the
widest margin (32:9: 534 UI columns = 267 backscreen pixels per side).

References for outpainting:  python tools/prepare_vehicle_sides.py refs [OUTDIR]
Layout guides (v2):          python tools/prepare_vehicle_sides.py guides [OUTDIR]
Seam-repair input:           python tools/prepare_vehicle_sides.py seam tank|car left|right GENERATED OUT
  (the generated image with the original strip pasted exactly where the game shows it, and a
  BRIDGE-pixel key-colour band before it, to be inpainted so both sides connect)
Embedded assets:             python tools/prepare_vehicle_sides.py build tank|car GENERATED_LEFT GENERATED_RIGHT
  -> assets/<vehicle>-sides.png (preview) and assets/<vehicle>-sides.rgba
     (left then right, 534x480 RGBA each; alpha 0 = aircar window).
"""
from pathlib import Path
import sys
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
RSC = ROOT / "iso/CYBERMAG/HIPAK.RSC"
BACKSCREENS = {"tank": 0x5DE08, "car": 0x70A08}  # LoadBackScreen 2 and 3
PALETTE = 0x169970  # resource entry 31, the gameplay/Options palette
SIDE = 267          # backscreen pixels per side; 534 UI columns
CONTEXT = 93        # cockpit columns shown next to the placeholder (3:2 reference)
BRIDGE = 32         # side-art columns re-inpainted next to the join
SEAM_MAX = 40       # largest per-row colour step treated as a tone difference
SEAM = 48           # side-art columns over which the join's colour step fades out
PLACEHOLDER = {"tank": (255, 0, 255), "car": (0, 255, 0)}
WINDOW_KEY = (255, 0, 255)  # aircar windshield in references and generated art
# Aircar windshield rows (backscreen pixels): CarWinInfo 640x276 at y=16.
CAR_WINDOW_ROWS = range(8, 146)


def load(vehicle):
    data = RSC.read_bytes()
    idx = data[BACKSCREENS[vehicle]:BACKSCREENS[vehicle] + 320 * 240]
    raw = data[PALETTE:PALETTE + 768]
    e = lambda v: (v << 2) | (v >> 4)
    palette = [tuple(e(v) for v in raw[i * 3:i * 3 + 3]) for i in range(256)]
    return idx, palette


def cockpit(vehicle):
    idx, palette = load(vehicle)
    window = WINDOW_KEY if vehicle == "car" else palette[0]
    image = Image.new("RGB", (320, 240))
    image.putdata([window if i == 0 else palette[i] for i in idx])
    return image, sorted({palette[i] for i in idx if i != 0})


def refs(out):
    out.mkdir(parents=True, exist_ok=True)
    for vehicle in BACKSCREENS:
        image, _ = cockpit(vehicle)
        for side in ("left", "right"):
            ref = Image.new("RGB", (SIDE + CONTEXT, 240), PLACEHOLDER[vehicle])
            if side == "left":
                ref.paste(image.crop((0, 0, CONTEXT, 240)), (SIDE, 0))
            else:
                ref.paste(image.crop((320 - CONTEXT, 0, 320, 240)), (0, 0))
            path = out / f"{vehicle}-{side}-ref.png"
            ref.resize((1536, 1024), Image.Resampling.NEAREST).save(path)
            print(path)
        image.resize((640, 480), Image.Resampling.NEAREST).save(out / f"{vehicle}-cockpit.png")


def register(generated, vehicle, side):
    """The generator redraws the cockpit columns slightly moved/rescaled. Find the
    affine (sx, sy, tx, ty) taking reference-canvas pixels to generated pixels."""
    g = np.asarray(generated, dtype=np.int32)
    k = generated.width / (SIDE + CONTEXT)
    image, _ = cockpit(vehicle)
    idx, _ = load(vehicle)
    x0 = SIDE if side == "left" else 0
    cols = range(0, CONTEXT) if side == "left" else range(320 - CONTEXT, 320)
    pts = [(x0 + i, y, image.getpixel((c, y))) for i, c in enumerate(cols) for y in range(0, 240, 2)
           if idx[y * 320 + c] != 0]
    qx = np.array([p[0] + 0.5 for p in pts]); qy = np.array([p[1] + 0.5 for p in pts])
    ref = np.array([p[2] for p in pts], dtype=np.int32)
    def cost(sx, sy, tx, ty):
        gx = np.clip((sx * k * qx + tx).astype(int), 0, g.shape[1] - 1)
        gy = np.clip((sy * k * qy + ty).astype(int), 0, g.shape[0] - 1)
        return np.abs(g[gy, gx] - ref).mean()
    best = min((cost(s, s, tx, ty), s, s, tx, ty) for s in np.arange(0.90, 1.101, 0.01)
               for tx in range(-80, 81, 4) for ty in range(-60, 61, 4))
    for step in (2, 1):  # refine scales independently, then offsets
        _, sx, sy, tx, ty = best
        best = min(best, *((cost(a, b, x, y), a, b, x, y) for a in np.arange(sx - 0.01, sx + 0.0101, 0.005)
                           for b in np.arange(sy - 0.01, sy + 0.0101, 0.005)
                           for x in range(tx - 2 * step, tx + 2 * step + 1, step)
                           for y in range(ty - 2 * step, ty + 2 * step + 1, step)))
    print(f"{vehicle} {side}: residual {best[0]:.1f}, scale ({best[1]:.3f},{best[2]:.3f}), offset ({best[3]},{best[4]})")
    return best[1:], k


def registered(vehicle, side, path, over=1):
    """Generated image resampled onto the reference canvas ((SIDE+CONTEXT)*over x 240*over)."""
    generated = Image.open(path).convert("RGB")
    (sx, sy, tx, ty), k = register(generated, vehicle, side)
    pad = 256  # edge-replicate where the registered canvas leaves the image
    padded = Image.fromarray(np.pad(np.asarray(generated), ((pad, pad), (pad, pad), (0, 0)), mode="edge"))
    fine = 4
    canvas = padded.transform(((SIDE + CONTEXT) * fine, 240 * fine), Image.Transform.AFFINE,
        (sx * k / fine, 0, tx + pad, 0, sy * k / fine, ty + pad), Image.Resampling.BILINEAR)
    return canvas.resize(((SIDE + CONTEXT) * over, 240 * over), Image.Resampling.BOX)


def poly(draw, points, side, fill):
    """Points in side-art pixels (x measured from the art's outer edge)."""
    draw.polygon([(x if side == "left" else SIDE - 1 - x, y) for x, y in points], fill=fill)


def tank_layout(side):
    """Cabin cross-section from the reviewed direction sketch: the ammo/power
    bulkhead runs straight out to one hull rib (ceiling chamfer, vertical, floor
    chamfer), and the console's top edge rises toward the original console,
    in front of the rib."""
    from PIL import ImageDraw
    g = Image.new("RGB", (SIDE, 240))
    d = ImageDraw.Draw(g)
    for x in range(SIDE):  # far side wall, darkening outward
        t = x / SIDE if side == "left" else 1 - x / SIDE
        d.line((x, 0, x, 239), fill=tuple(round(c * (0.35 + 0.65 * t)) for c in (40, 40, 48)))
    poly(d, [(150, 0), (267, 0), (267, 38), (136, 38)], side, (22, 22, 26))      # ceiling
    poly(d, [(136, 38), (267, 38), (267, 146), (136, 146)], side, (74, 76, 86))  # riveted bulkhead
    poly(d, [(136, 146), (267, 146), (267, 166), (148, 166)], side, (46, 48, 54))  # vents
    poly(d, [(140, 166), (267, 166), (267, 240), (190, 240)], side, (14, 14, 18))  # recess
    poly(d, [(130, 0), (156, 0), (136, 40), (136, 112), (200, 240), (174, 240), (110, 112), (110, 40)],
         side, (104, 106, 116))                                                   # hull rib
    top = 186 if side == "left" else 166  # original console top at the join
    poly(d, [(143, 232), (267, top), (267, 240), (128, 240)], side, (150, 150, 158))  # console
    return g


def guides(out):
    """Layout guides for the second generation round, in reference-canvas layout."""
    out.mkdir(parents=True, exist_ok=True)
    image, _ = cockpit("tank")
    for side in ("left", "right"):
        ref = Image.new("RGB", (SIDE + CONTEXT, 240))
        if side == "left":
            ref.paste(tank_layout(side), (0, 0)); ref.paste(image.crop((0, 0, CONTEXT, 240)), (SIDE, 0))
        else:
            ref.paste(image.crop((320 - CONTEXT, 0, 320, 240)), (0, 0)); ref.paste(tank_layout(side), (CONTEXT, 0))
        ref.resize((1536, 1024), Image.Resampling.NEAREST).save(out / f"tank-{side}-guide-v2.png")
        # Aircar: keep the reviewed upper canopy; below the dashboard lip the sides
        # become blue body panels continuing the original cowl.
        gen = Image.open(ROOT / f"assets/vehicle-sides/car-{side}.png").convert("RGB")
        k = gen.width / (SIDE + CONTEXT)
        x0, x1 = (0, SIDE) if side == "left" else (CONTEXT, SIDE + CONTEXT)
        gen.paste((16, 24, 176), (round(x0 * k) + 8, round(158 * k), round(x1 * k) - 8, gen.height))
        gen.save(out / f"car-{side}-guide-v2.png")
        print(out / f"tank-{side}-guide-v2.png", out / f"car-{side}-guide-v2.png")


def seam_blend(vehicle, side, pixels, window):
    """Gradient-domain join: per row, shift the art's brightness so its edge columns
    match the original's edge columns, fading the shift out over SEAM columns. Keeps the
    generated detail; removes the brightness/tint step at the join."""
    idx, palette = load(vehicle)
    a = np.array(pixels, dtype=np.float64).reshape(240, SIDE, 3)
    win = np.array(window).reshape(240, SIDE)
    cols = range(0, 3) if side == "left" else range(317, 320)
    orig = np.array([[palette[idx[y * 320 + c]] for c in cols] for y in range(240)], dtype=np.float64)
    valid = np.array([all(idx[y * 320 + c] != 0 for c in cols) for y in range(240)])
    edge = a[:, SIDE - 3:, :] if side == "left" else a[:, :3, :]
    edge_win = win[:, SIDE - 3:] if side == "left" else win[:, :3]
    valid &= ~edge_win.any(axis=1)
    diff = orig.mean(axis=1) - edge.mean(axis=1)
    # Only tone steps: large differences are different shapes meeting, which a
    # colour shift would turn into a haze.
    valid &= np.abs(diff).max(axis=1) <= SEAM_MAX
    # Brightness only: a tint shift would drag edge colours (the red POWER frame) outward.
    offset = np.where(valid[:, None], diff.mean(axis=1, keepdims=True).repeat(3, 1), 0.0)
    # Smooth vertically (dithered texture makes single rows noisy), ignoring invalid rows.
    k = np.ones(5)
    w = np.convolve(valid.astype(float), k, "same")
    offset = np.stack([np.convolve(offset[:, ch], k, "same") for ch in range(3)], 1) / np.maximum(w, 1)[:, None]
    d = np.arange(SIDE)[::-1] if side == "left" else np.arange(SIDE)  # distance from the join
    fade = np.clip(1 - d / SEAM, 0, 1) ** 2
    a += offset[:, None, :] * fade[None, :, None] * (~win)[:, :, None]
    return [tuple(int(v) for v in p) for p in np.clip(np.rint(a), 0, 255).reshape(-1, 3)]


def seam_input(vehicle, side, path, out):
    generated = Image.open(path).convert("RGB")
    (sx, sy, tx, ty), k = register(generated, vehicle, side)
    image, _ = cockpit(vehicle)
    strip = image.crop((0, 0, CONTEXT, 240) if side == "left" else (320 - CONTEXT, 0, 320, 240))
    strip = strip.resize((round(CONTEXT * sx * k), round(240 * sy * k)), Image.Resampling.NEAREST)
    x = round((SIDE if side == "left" else 0) * sx * k + tx)
    mask = None
    if vehicle == "car":  # keep the generated windshield key colour
        mask = Image.eval(strip.convert("L"), lambda v: 255).copy()
        mask.putdata([0 if p == WINDOW_KEY else 255 for p in strip.getdata()])
    generated.paste(strip, (x, round(ty)), mask)
    band = round(BRIDGE * sx * k)
    key = (255, 0, 255) if vehicle == "tank" else (0, 255, 0)
    generated.paste(key, (x - band, 0, x, generated.height) if side == "left" else (x + strip.width, 0, x + strip.width + band, generated.height))
    generated.save(out)
    print(out)


def side_art(vehicle, side, path):
    """Generated 3:2 image -> 267x240 palette pixels plus window mask."""
    image = registered(vehicle, side, path)
    box = (0, 0, SIDE, 240) if side == "left" else (CONTEXT, 0, SIDE + CONTEXT, 240)
    art = image.crop(box)
    _, colours = cockpit(vehicle)
    pal = Image.new("P", (1, 1))
    flat = [c for rgb in colours for c in rgb]
    pal.putpalette(flat + flat[:3] * (256 - len(colours)))
    key = lambda p: p[0] > 170 and p[2] > 170 and p[1] < 110
    pixels = list(art.getdata())
    window = [vehicle == "car" and key(p) and (i // SIDE) in CAR_WINDOW_ROWS for i, p in enumerate(pixels)]
    # Key fringes and stray key pixels outside the windshield become cockpit colours.
    solid = [(24, 24, 32) if key(p) else p for p in pixels]
    solid = seam_blend(vehicle, side, solid, window)
    art.putdata(solid)
    art = art.quantize(palette=pal, dither=Image.Dither.NONE).convert("RGB")
    rgba = Image.new("RGBA", art.size)
    rgba.putdata([(0, 0, 0, 0) if w else p + (255,) for p, w in zip(art.getdata(), window)])
    return rgba


def build(vehicle, left, right):
    sides = [side_art(vehicle, "left", left), side_art(vehicle, "right", right)]
    sides = [s.resize((SIDE * 2, 480), Image.Resampling.NEAREST) for s in sides]
    image, _ = cockpit(vehicle)
    preview = Image.new("RGBA", (SIDE * 4 + 640, 480))
    preview.paste(sides[0], (0, 0))
    preview.paste(image.resize((640, 480), Image.Resampling.NEAREST).convert("RGBA"), (SIDE * 2, 0))
    preview.paste(sides[1], (SIDE * 2 + 640, 0))
    preview.save(ROOT / f"assets/{vehicle}-sides.png")
    target = ROOT / f"assets/{vehicle}-sides.rgba"
    target.write_bytes(sides[0].tobytes() + sides[1].tobytes())
    assert target.stat().st_size == 2 * SIDE * 2 * 480 * 4
    print(f"{target}: 2x {SIDE * 2}x480 RGBA")


if __name__ == "__main__":
    if sys.argv[1] == "seam":
        seam_input(sys.argv[2], sys.argv[3], Path(sys.argv[4]), Path(sys.argv[5]))
    elif sys.argv[1] == "guides":
        guides(Path(sys.argv[2]) if len(sys.argv) > 2 else ROOT / "scratch/vehicle-sides")
    elif sys.argv[1] == "refs":
        refs(Path(sys.argv[2]) if len(sys.argv) > 2 else ROOT / "scratch/vehicle-sides")
    else:
        build(sys.argv[2], *map(Path, sys.argv[3:5]))
