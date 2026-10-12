"""Check the clean ZIP, both commercial-data import paths, and modern desktop input.
Requires your original local v1.01F assets/ISO; all gameplay writes use temporary roots.
Run: python tools/check_release.py
"""
from pathlib import Path
from datetime import datetime
import hashlib
import os
import re
import shutil
import struct
import subprocess
import tempfile
import zipfile
from package_release import imported_dlls
from check_icon import check_icon

PROJECT = Path(__file__).resolve().parents[1]
ARCHIVE = PROJECT / "dist/cybermage-windows-x64.zip"
OUTPUT = PROJECT / "out/release-check"


def hashes(root):
    return {p.relative_to(root).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in root.rglob("*") if p.is_file()}


def main():
    personal = hashes(PROJECT / "run")
    OUTPUT.mkdir(parents=True, exist_ok=True)
    env = {k: v for k, v in os.environ.items() if not k.startswith("CM_")}
    env["CM_DUMP"] = "1819c4:24"
    try:
        with tempfile.TemporaryDirectory(prefix="cybermage-release-") as temp:
            root = Path(temp)
            with zipfile.ZipFile(ARCHIVE) as z:
                for name in z.namelist():
                    p = Path(name)
                    assert p.parts[0] == "cybermage-windows-x64" and ".." not in p.parts
                    assert p.parts[1] in {"cybermage.exe", "README.md", "CONTROLS.md", "LICENSE", "THIRD_PARTY_NOTICE.md",
                                          "DEPENDENCIES.txt", "SHA256SUMS", "licenses", "guides"}, name
                    assert not any(s.upper().startswith("SAVE") or s.lower() in {"iso", "run"} for s in p.parts), name
                    assert p.suffix.lower() not in {".iso", ".rsc", ".tgv", ".dat", ".cnf", ".usr"}, name
                z.extractall(root)
            package = root / "cybermage-windows-x64"
            exe = package / "cybermage.exe"
            imported_dlls(exe)  # verifies x64 GUI subsystem, including --no-build archives
            check_icon(exe)
            for line in (package / "SHA256SUMS").read_text().splitlines():
                digest, name = line.split(" *", 1)
                assert hashlib.sha256((package / name).read_bytes()).hexdigest() == digest
            expected = ARCHIVE.with_suffix(".zip.sha256").read_text().split()[0]
            assert hashlib.sha256(ARCHIVE.read_bytes()).hexdigest() == expected
            print("ZIP allowlist, asset exclusion and SHA-256 manifest passed.")

            def command(name, *args, success=True, dump="1819c4:24"):
                result = subprocess.run([str(exe), *map(str, args)], cwd=PROJECT, env={**env,"CM_DUMP":dump},
                                        capture_output=True, timeout=240)
                log = result.stdout.decode(errors="replace") + result.stderr.decode(errors="replace")
                (OUTPUT / f"{name}.log").write_text(log)
                assert (result.returncode == 0) == success, log[-4000:]
                assert "panicked" not in log and "FAULT" not in log, log[-4000:]
                return log

            log = command("missing-assets", "--headless", "--seconds", ".1", success=False)
            assert "Game assets unavailable" in log, "Portable default root leaked to the developer's assets"
            assert not (package / "run").exists()
            bad_iso = root / "bad.iso"
            bad_iso.write_bytes(bytes(20*2048))
            broken = root / "broken"
            command("bad-iso", "--import-assets", bad_iso, "--root", broken, success=False)
            assert not list(broken.iterdir()), "Failed import left partial assets"

            # Use copies, never modify the read-only original or personal files.
            source = root / "source"
            shutil.copytree(PROJECT / "iso/CYBERMAG", source)
            for name in ("SAVE000.DAT", "CMDATA.CNF", "GAMENAME.USR", "NATIVE.CNF", "CMPATH.CNF"):
                (source / name).write_bytes(b"MUST NOT BE IMPORTED")
            original_exe = (source / "CM_HI.EXE").read_bytes()
            changed = bytearray(original_exe)
            changed[-1] ^= 1
            (source / "CM_HI.EXE").write_bytes(changed)
            command("wrong-version", "--import-assets", source, "--root", broken, success=False)
            assert not list(broken.iterdir())
            (source / "CM_HI.EXE").write_bytes(original_exe)
            command("folder-import", "--import-assets", source, "--root", package)
            for name in ("SAVE000.DAT", "CMDATA.CNF", "GAMENAME.USR", "NATIVE.CNF", "CMPATH.CNF"):
                assert not (package / "iso/CYBERMAG" / name).exists()
            imported = hashes(package / "iso/CYBERMAG")
            command("no-overwrite", "--import-assets", source, "--root", package, success=False)
            assert hashes(package / "iso/CYBERMAG") == imported
            iso_root = root / "iso-import"
            command("iso-import", "--import-assets", PROJECT / "CMV1.01F.iso", "--root", iso_root)
            assert hashes(iso_root / "iso/CYBERMAG") == imported, "Folder and raw ISO extraction differ"
            print("Folder/ISO parity, wrong-version rejection, atomic failure and no-overwrite checks passed.")

            def kb(start, end, keys="NONE", dx=0, dy=0, buttons=0, wheel=0):
                return ["--kb", f"{start}:{end}:{keys}:{dx}:{dy}:{buttons}:{wheel}"]

            def play(name, *extra, seconds=18, boot=True, state=False):
                log = command(name, "--headless", "--root", package, "--resolution", "640x480",
                              "--seconds", seconds, *(kb(9, 9.2, "ENTER") if boot else []), *extra,
                              dump="1e2de8:4" if state else "1819c4:24")
                raw = bytearray()
                for line in log.splitlines():
                    if re.match(r"1E2DE8:" if state else r"1819(C4|D4):", line):
                        raw.extend(int(b.strip(), 16) for b in line.split("[")[1].split("]")[0].split(","))
                assert len(raw) == (4 if state else 24), log[-4000:]
                image = package / "out/final.bmp"
                shutil.copyfile(image, OUTPUT / f"{name}.bmp")
                assert len(set(image.read_bytes()[54:])) > 32
                camera = int.from_bytes(raw,"little") if state else (
                    *struct.unpack_from("<iii", raw), struct.unpack_from("<H", raw, 12)[0],
                    struct.unpack_from("<i", raw, 16)[0])
                print(f"{name}: {camera}")
                return camera

            base = play("fresh-default-boot")
            move = play("keyboard-wasd", *kb(14, 16, "W+D"))
            assert move[:2] != base[:2] and move[3:] == base[3:]
            look = play("mouse-look", *kb(14, 15, dx=100, dy=80))
            assert look[:3] == base[:3] and look[3] != base[3] and look[4] > base[4]
            jump = play("keyboard-jump", *kb(14, 14.4, "SPACE"), seconds=14.15)
            idle = play("keyboard-idle", seconds=14.15)
            assert jump[:2] == idle[:2] and jump[2] > idle[2]
            guided = play("keyboard-guide", *kb(14, 14.2, "F1"), *kb(14.3, 17, "W"))
            assert guided == base
            from PIL import Image
            def guide_pixels(name):
                raw=(PROJECT / "assets" / name).read_bytes()
                return b"".join(raw[(y*2*1280+x*2)*3:(y*2*1280+x*2)*3+3] for y in range(480) for x in range(640))
            assert Image.open(OUTPUT / "keyboard-guide.bmp").convert("RGB").tobytes() == guide_pixels("keyboard-help.rgb")
            switched = play("guide-switched", *kb(14, 14.2, "F1"), *kb(15, 15.2, "RIGHT"))
            assert switched == base
            assert Image.open(OUTPUT / "guide-switched.bmp").convert("RGB").tobytes() == guide_pixels("controller-help.rgb")
            closed = play("guide-closed", *kb(14, 14.2, "F1"), *kb(15, 15.2, "ESC"), *kb(15.5, 17, "W"))
            assert closed[:2] != base[:2]
            print("Modern movement, relative mouse axes, stationary jump and both guide pages passed.")
            assert play("mouse-hand-held", *kb(14, 18, buttons=2), state=True) == 3
            assert play("mouse-hand-released", *kb(14, 16, buttons=2), state=True) == 5
            assert play("keyboard-powers", *kb(14, 14.2, "2"), state=True) == 3
            assert play("keyboard-inventory", *kb(14, 14.2, "3"), state=True) == 3
            assert play("keyboard-exit-selector", *kb(14, 14.2, "2"), *kb(15, 15.2, "ESC"), state=True) == 5
            assert Image.open(OUTPUT / "keyboard-powers.bmp").crop((0,100,160,230)).tobytes() != Image.open(
                OUTPUT / "keyboard-inventory.bmp").crop((0,100,160,230)).tobytes()
            print("Held/released hand aiming, distinct power/inventory commands and Escape cancellation passed.")

            before = datetime.now().replace(microsecond=0)
            saved = play("keyboard-save", *kb(14, 14.2, "ESC"), *kb(14.7, 14.9, "LEFT"),
                         *kb(15.5, 15.7, "ENTER"), *kb(16.5, 16.7, "ENTER"))
            stamp = (package / "run/GAMENAME.USR").read_bytes()[:24].split(b"\0")[0].decode()
            assert before <= datetime.strptime(stamp, "%Y-%m-%d %H:%M:%S") <= datetime.now()
            assert (package / "run/SAVE000.DAT").stat().st_size > 100000
            loaded = play("keyboard-load", *kb(14, 15, "W"), *kb(16, 16.2, "ESC"),
                          *kb(16.5, 16.7, "DOWN"), *kb(17.1, 17.3, "LEFT"),
                          *kb(17.8, 18, "ENTER"), *kb(18.8, 19, "ENTER"), seconds=22)
            assert loaded == saved, "Keyboard load did not restore the saved camera"
            print(f"Keyboard save/load passed with automatic name {stamp}.")
    finally:
        assert hashes(PROJECT / "run") == personal, "Personal saves/configuration changed"
        print("Personal run/ file hashes unchanged.")


if __name__ == "__main__":
    main()
