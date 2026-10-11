"""Load a save slot (or -1: new game) from the title in an isolated copy of run/ and screenshot it.
Run after cargo build --release:
  [CM_VEHICLE=1] [CM_SCENE=N] python tools/check_vehicle.py SLOT|-1 [MODE ...] [--seconds N] [-- extra cybermage args]
Screenshots: out/vehicle-check/slotN-MODE.png. Never writes the user's run/ files.
"""
from pathlib import Path
import os
import shutil
import subprocess
import sys
import tempfile


def main():
    argv = sys.argv[1:]
    extra = argv[argv.index("--") + 1:] if "--" in argv else []
    argv = argv[:argv.index("--")] if "--" in argv else argv
    seconds = "16"
    if "--seconds" in argv:
        i = argv.index("--seconds"); seconds = argv[i + 1]; del argv[i:i + 2]
    slot, modes = int(argv[0]), argv[1:] or ["1280x800"]
    project = Path(__file__).resolve().parents[1]
    exe = project / "target/release" / ("cybermage.exe" if os.name == "nt" else "cybermage")
    output = project / "out/vehicle-check"
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="cybermage-vehicle-") as directory:
        root = Path(directory)
        if os.name == "nt":
            subprocess.run(["cmd", "/c", "mklink", "/J", str(root / "iso"), str(project / "iso")],
                           check=True, capture_output=True)
        else:
            (root / "iso").symlink_to(project / "iso", target_is_directory=True)
        shutil.copytree(project / "run", root / "run")
        def button(t, mask):
            return ["--pad", f"{t}:{t + 0.2}:0:0:0:0:{mask:x}:0:0"]
        # Title: Load (DPad right), open, move down to the slot, accept.
        keys = [*button(9, 8), *button(9.8, 0x1000)]
        t = 10.3
        for _ in range(slot):
            keys += button(t, 2); t += 0.4
        keys += button(t + 0.2, 0x1000)
        if slot < 0:
            keys = ["--key", "9:1c"]  # new game (scene from CM_SCENE)
        env = {k: v for k, v in os.environ.items() if not k.startswith("CM_") or k in ("CM_VEHICLE", "CM_COUNT", "CM_DUMP", "CM_SCENE")}
        for mode in modes:
            args = [str(exe), "--headless", "--root", str(root), "--resolution", mode,
                    "--seconds", seconds, *keys, *extra]
            result = subprocess.run(args, env=env, capture_output=True, timeout=300)
            log = result.stdout.decode(errors="replace") + result.stderr.decode(errors="replace")
            (output / f"slot{slot}{os.environ.get('CM_SCENE', '')}-{mode}.log").write_text(log)
            assert result.returncode == 0 and "FAULT" not in log and "panicked" not in log, log[-3000:]
            from PIL import Image
            Image.open(root / "out/final.bmp").save(output / f"slot{slot}{os.environ.get('CM_SCENE', '')}-{mode}.png")
            print(output / f"slot{slot}{os.environ.get('CM_SCENE', '')}-{mode}.png")


if __name__ == "__main__":
    main()
