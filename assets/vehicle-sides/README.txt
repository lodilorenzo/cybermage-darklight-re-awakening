Widescreen cockpit side art (generated with the Codex image tool, outpainting).
Embedded runtime assets: assets/tank-sides.rgba, assets/car-sides.rgba (previews: *-sides.png).
The .rgba files contain ONLY the added margins: left then right, each 534x480
RGBA, with no header. The original 640x480 center is loaded from the user's
commercial game files at runtime; it is not part of these embedded files.
Public source exports keep the .rgba files and this README, but omit all
reference/generation images listed below and the combined *-sides.png previews,
which contain original artwork. Those inputs must be kept locally to rebuild
art; compiling the existing .rgba files does not require them.
This pixel separation is not legal clearance for generated derivatives;
see docs/SOURCE_EXPORT.md before redistributing.

Current sources:
  tank-left-v3.png, car-left-v3.png  <- *-left-seam-v3.png: the v2 image with the original strip
      pasted where the game shows it and a 32-pixel key-colour band before it, inpainted so the
      art bridges into the original (fixes the visible v2 joins)
  tank-right-v2.png, car-right-v2.png (v2 joins were already clean)
v2 (follows the reviewed direction sketches *-direction-sketch.png):
  tank-*-v2.png <- tank-*-guide-v2.png (flat layout + original strip)
  car-*-v2.png  <- car-*-guide-v2.png  (v1 canopy kept, lower sides = blue body panels)
v1 sources (tank-left.png ... car-right.png, car-*-guide.png) are kept for reference only.

Rebuild:
  python tools/prepare_vehicle_sides.py guides OUTDIR      # regenerate guides
  python tools/prepare_vehicle_sides.py seam tank left assets/vehicle-sides/tank-left-v2.png OUT.png  # inpainting input
  python tools/prepare_vehicle_sides.py build tank assets/vehicle-sides/tank-left-v3.png assets/vehicle-sides/tank-right-v2.png
  python tools/prepare_vehicle_sides.py build car  assets/vehicle-sides/car-left-v3.png  assets/vehicle-sides/car-right-v2.png
The build registers each image against the original cockpit columns, quantizes to the
original backscreen colours, keys the aircar windshield (magenta) to transparent, and
removes any remaining brightness step at the join (per row, fading over 48 columns;
rows where different shapes meet are left alone).

Tank v2 prompt (left; right mirrored with POWER frame, "no second gauge"):
  Render a flat-colour layout sketch into finished pixel art; keep the original strip unchanged.
  mid-grey rectangle = riveted bulkhead continuing the AMMO wall with the same top/bottom edges;
  light-grey bent band = one structural hull rib tracing the cabin cross-section (ceiling chamfer,
  vertical, floor chamfer); dark top = ceiling with conduits; dark band = vent strip and recess;
  light-grey wedge = foreground console whose top edge rises toward the original console, in
  front of the rib; far side = angled cabin side wall fading to shadow. 1995 VGA pixel art,
  doubled 320x240, cool greys; no text, numbers, logos, windows or people.
Aircar v2 prompt (left; right mirrored around the KPH/ALT screen):
  Keep everything above the flat blue rectangle; repaint it as glossy royal/navy blue body /
  dashboard side panels continuing the original cowl, its curved edge sweeping outward, a slim
  recess under the lip, smooth panels with seams, bevels, rivets, one grille; no grey consoles,
  screens or lights; darker toward the edge. Magenta glass stays flat magenta.

Seam inpainting prompt (v3, tank; aircar analogous with green key, A-pillar, gun barrel and cowl):
  Fill ONLY the flat magenta band so both sides connect seamlessly: bulkhead tone/texture,
  top edge and rivet rows line up; vent slats continue at the same heights; the console's
  top edge is one continuous diagonal into the original slanted console; nothing outside
  the band changes.
