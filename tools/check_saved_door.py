"""Doorway regression using a COPY of the reported SAVE003.DAT, never personal saves.
Run after cargo build --release: python tools/check_saved_door.py [path/to/save.dat]
The supplied save must be at the reported level-two doorway (x~10.7, y~25.3).
"""
from pathlib import Path
import hashlib
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile


def main():
    project = Path(__file__).resolve().parents[1]
    save = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else project / "run/SAVE003.DAT"
    slot = int(save.stem[4:])
    names = bytearray(save.with_name("GAMENAME.USR").read_bytes())
    names[:24] = names[slot*24:(slot+1)*24]
    personal = {p: hashlib.sha256(p.read_bytes()).digest() for p in (project / "run").rglob("*") if p.is_file()}
    output = project / "out/saved-door-check"
    output.mkdir(exist_ok=True)
    exe = project / "target/release" / ("cybermage.exe" if os.name == "nt" else "cybermage")
    assert exe.exists(), "Run cargo build --release first"
    env = {k: v for k, v in os.environ.items() if not k.startswith("CM_")}
    env.update(CM_DUMP="1819c4:24", CM_TRACE_SONG="1")
    with tempfile.TemporaryDirectory(prefix="cybermage-door-") as directory:
        root = Path(directory)
        if os.name == "nt":
            subprocess.run(["cmd", "/c", "mklink", "/J", str(root / "iso"), str(project / "iso")], check=True, capture_output=True)
        else:
            (root / "iso").symlink_to(project / "iso", target_is_directory=True)
        (root / "run").mkdir()
        for name in ("CMDATA.CNF", "CMPATH.CNF"):
            shutil.copyfile(project / "run" / name, root / "run" / name)
        (root / "run/GAMENAME.USR").write_bytes(names)
        shutil.copyfile(save, root / "run/SAVE000.DAT")

        def run(name, mode="640x480", pad=None, seconds=20, original_collision=False):
            args = [str(exe), "--headless", "--root", str(root), "--resolution", mode, "--seconds", str(seconds),
                    "--pad", "9:9.2:0:0:0:0:8:0:0", "--pad", "9.8:10:0:0:0:0:1000:0:0",
                    "--pad", "10.8:11:0:0:0:0:1000:0:0"]
            if pad:
                args += ["--pad", pad]
            check_env = dict(env)
            if original_collision:
                check_env["CM_NO_HOOK"] = "Xox::SmallStepOpening"
            result = subprocess.run(args, cwd=root, env=check_env, capture_output=True, timeout=240)
            log = result.stdout.decode(errors="replace") + result.stderr.decode(errors="replace")
            (output / f"{name}.log").write_text(log)
            assert result.returncode == 0 and "FAULT" not in log and "panicked" not in log, log[-4000:]
            data = bytearray()
            for line in log.splitlines():
                if re.match(r"1819(C4|D4):", line):
                    data.extend(int(b.strip(), 16) for b in line.split("[")[1].split("]")[0].split(","))
            assert len(data) == 24, "Missing camera dump"
            position = struct.unpack_from("<ii", data)
            assert "ok=true" in log and re.search(r"\[music\] master volume [1-9]\d*/127", log), "Music did not start at an audible volume"
            shutil.copyfile(root / "out/final.bmp", output / f"{name}.bmp")
            print(f"{name}: camera={position}")
            return position

        initial = run("saved-baseline")
        assert 10*65536 < initial[0] < 11*65536 and 25*65536 < initial[1] < 26*65536, "Supply the save at the reported doorway"
        blocked = run("original-small-step", pad="14:18:0:1:0:0:0:0:0", original_collision=True)
        assert blocked[1] < 26*65536, "The original doorway obstruction was not reproduced"
        for name, mode, pad, seconds in [
            ("fixed-forward", "640x480", "14:18:0:1:0:0:0:0:0", 20),
            ("fixed-slow-wide", "1920x1080", "14:26:0:0.2:0:0:0:0:0", 28),
            ("fixed-ultrawide", "5120x1440", "14:18:0:1:0:0:0:0:0", 20),
        ]:
            crossed = run(name, mode, pad, seconds)
            assert crossed[1] > 27*65536, f"{mode}: still blocked at the open doorway"
    assert all(hashlib.sha256(p.read_bytes()).digest() == digest for p, digest in personal.items()), "Personal runtime file changed"
    print("Open doorway crossed at full/partial stick; music enabled; personal files unchanged")


if __name__ == "__main__":
    main()
