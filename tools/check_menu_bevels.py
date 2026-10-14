"""Check new menu borders against original Options artwork, using isolated settings."""
from pathlib import Path
import math
import os
import shutil
import subprocess
import tempfile
from PIL import Image

project = Path(__file__).resolve().parents[1]
output = project / "out/native-check"
output.mkdir(parents=True, exist_ok=True)
env = {k: v for k, v in os.environ.items() if not k.startswith("CM_")}
with tempfile.TemporaryDirectory(prefix="cybermage-menu-bevel-") as directory:
    root = Path(directory)
    if os.name == "nt":
        subprocess.run(["cmd", "/c", "mklink", "/J", str(root / "iso"), str(project / "iso")], check=True, capture_output=True)
    else:
        (root / "iso").symlink_to(project / "iso", target_is_directory=True)
    (root / "run").mkdir()
    for name in ("CMDATA.CNF", "CMPATH.CNF"):
        if (project / "run" / name).exists():
            shutil.copyfile(project / "run" / name, root / "run" / name)
    args = [str(project / "target/release" / ("cybermage.exe" if os.name == "nt" else "cybermage")),
            "--headless", "--root", str(root), "--seconds", "11", "--resolution", "1280x720",
            "--key", "9:18", "--key", "10:0f"]
    result = subprocess.run(args, env=env, capture_output=True, timeout=240)
    log = result.stdout.decode(errors="replace") + result.stderr.decode(errors="replace")
    (output / "menu-bevel-game.log").write_text(log)
    assert result.returncode == 0 and "FAULT" not in log and "panicked" not in log, log[-4000:]
    image = output / "enhancements-beveled.bmp"
    shutil.copyfile(root / "out/final.bmp", image)

im = Image.open(image)
assert im.size == (1280,720)
pixel = lambda x, y: im.getpixel((160 + math.ceil(x * 1.5), math.ceil(y * 1.5)))
separators = [146+(i+1)*26-6 for i in range(9)] + [446]
for y in range(146,458):
    if any(s <= y < s+4 for s in separators):
        continue
    assert [pixel(x,y) for x in range(12,16)] == [(121,121,130)]*2 + [(60,60,60)]*2, ("left stub",y)
    assert [pixel(x,y) for x in range(624,628)] == [(60,60,60)]*2 + [(121,121,130)]*2, ("right stub",y)
for y in separators:
    for dy in range(4):
        expected = (174,170,174) if dy < 2 else (77,77,85)
        assert pixel(100,y+dy) == expected, ("separator",y+dy)
assert [pixel(x,150) for x in range(350,356)] == [(121,121,130)]*2 + [(174,170,174)]*2 + [(60,60,60)]*2
# Green navigation glyphs must not intrude into the rounded bottom frame.
assert all(not (g > 150 and r < 80 and b < 80) for y in range(464,480) for x in range(12,628) for r,g,b in [pixel(x,y)])
print(f"Original bevel colors, continuous edges, value divider and footer bounds passed: {image}")
