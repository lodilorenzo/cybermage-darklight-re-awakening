"""Look-down/movement regression in an isolated overlay; run after cargo build --release."""
from pathlib import Path
import os
import shutil
import struct
import subprocess
import sys
import tempfile


def main():
    project = Path(__file__).resolve().parents[1]
    exe = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else project / "target/release" / (
        "cybermage.exe" if os.name == "nt" else "cybermage")
    output = project / "out/floor-check"
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="cybermage-floor-") as directory:
        root = Path(directory)
        if os.name == "nt":
            subprocess.run(["cmd", "/c", "mklink", "/J", str(root / "iso"), str(project / "iso")],
                           check=True, capture_output=True)
        else:
            (root / "iso").symlink_to(project / "iso", target_is_directory=True)
        (root / "run").mkdir()
        for name in ("CMDATA.CNF", "CMPATH.CNF"):
            shutil.copyfile(project / "run" / name, root / "run" / name)
        env = {k: v for k, v in os.environ.items() if not k.startswith("CM_")}
        for mode in ("640x480", "1920x1080", "3840x2160", "5120x1440", "5120x2160"):
            args = [str(exe), "--headless", "--root", str(root), "--resolution", mode,
                    "--seconds", "17", "--key", "9:1c",
                    "--pad", "14:16:0.6:-0.6:0:-1:0:0:0"]
            result = subprocess.run(args, env=env, capture_output=True, timeout=240)
            log = result.stdout.decode(errors="replace") + result.stderr.decode(errors="replace")
            (output / f"{mode}.log").write_text(log)
            assert result.returncode == 0 and "FAULT" not in log and "panicked" not in log, log[-4000:]
            image = (root / "out/final.bmp").read_bytes()
            (output / f"{mode}.bmp").write_bytes(image)
            w, h = struct.unpack_from("<ii", image, 18)
            pitch = (w * 3 + 3) & ~3
            # Near floor between the hand and the right wall in the starting room.
            pixels = [image[54 + (h-1-y)*pitch + x*3:54 + (h-1-y)*pitch + x*3+3]
                      for y in range(h*70//100, h*75//100) for x in range(w*51//100, w*56//100)]
            black = sum(p == b"\0\0\0" for p in pixels) / len(pixels)
            print(f"{mode}: {black:.1%} black floor pixels", flush=True)
            assert black < 0.01, f"Missing floor polygons at {mode}: {black:.1%} black pixels"
    print(f"Floor checks passed. Screenshots/logs: {output}")


if __name__ == "__main__":
    main()
