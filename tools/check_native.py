"""Native display/controller smoke check using an isolated writable game directory.
Run after cargo build --release: python tools/check_native.py [--menus-only | --saves-only | --viewport-only | --controls-only]
Requires the original game data; never writes the user's run/ saves/configuration.
"""
from pathlib import Path
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
from datetime import datetime


def main():
    project = Path(__file__).resolve().parents[1]
    exe = project / "target/release" / ("cybermage.exe" if os.name == "nt" else "cybermage")
    assert exe.exists(), "Run cargo build --release first"
    output = project / "out/native-check"
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="cybermage-native-") as directory:
        root = Path(directory)
        if os.name == "nt":
            subprocess.run(["cmd", "/c", "mklink", "/J", str(root / "iso"), str(project / "iso")],
                           check=True, capture_output=True)
        else:
            (root / "iso").symlink_to(project / "iso", target_is_directory=True)
        (root / "run").mkdir()
        for name in ("CMDATA.CNF", "CMPATH.CNF"):
            source = project / "run" / name
            if source.exists():
                shutil.copyfile(source, root / "run" / name)
        config = root / "run/CMDATA.CNF"
        if config.exists():
            data = bytearray(config.read_bytes())
            struct.pack_into("<i", data, 4, 0)  # second dword: saved keyboard-only inDevice
            config.write_bytes(data)
        env = {k: v for k, v in os.environ.items() if not k.startswith("CM_")}
        env["CM_DUMP"] = "1819c4:24"  # Xox camera x/y/z, heading, pitch, horizon
        env["CM_COUNT"] = "06BB24"  # Mouse::ReadStatus must run even in keyboard mode

        def run(name, mode, *extra, boot=True, expect_exit=False, viewport=False):
            args = [str(exe), "--headless", "--root", str(root), "--resolution", mode,
                    "--seconds", "18", *(["--key", "9:1c"] if boot else []), *extra]
            check_env = dict(env)
            if viewport:
                check_env["CM_DUMP"] = "0e972c:24"  # primary view: active, x, y, offset, width, height
            result = subprocess.run(args, env=check_env, capture_output=True, timeout=240)
            log = result.stdout.decode(errors="replace") + result.stderr.decode(errors="replace")
            (output / f"{name}.log").write_text(log)
            assert result.returncode == 0 and "FAULT" not in log and "panicked" not in log, log[-4000:]
            count = re.search(r"counter 06BB24: (\d+)", log)
            assert count and int(count[1]) > 0, "DOS mouse routing was disabled"
            if expect_exit:
                assert "program exited with code 0" in log, "Quit still waits for confirmation"
                print(f"{name}: normal shutdown, no confirmation")
                return
            image = (root / "out/final.bmp").read_bytes()
            assert struct.unpack_from("<ii", image, 18) == tuple(map(int, mode.split("x")))
            assert len(set(image[54:])) > 32, "Blank output"
            (output / f"{name}.bmp").write_bytes(image)
            data = bytearray()
            for line in log.splitlines():
                if re.match(r"0E97(2C|3C):" if viewport else r"1819(C4|D4):", line):
                    data.extend(int(b.strip(), 16) for b in line.split("[")[1].split("]")[0].split(","))
            assert len(data) == 24, "Missing guest view/camera dump"
            if viewport:
                assert struct.unpack("<6I", data) == (1, 0, 0, 0, 640, 480), "Startup viewport is not maximized"
                print(f"{name}: full-window viewport at {mode}")
                return
            xy = struct.unpack_from("<ii", data)
            angle = struct.unpack_from("<H", data, 12)[0]
            pitch = struct.unpack_from("<i", data, 16)[0]
            z = struct.unpack_from("<i", data, 8)[0]
            print(f"{name}: {mode}, camera={xy}, z={z}, angle={angle}, pitch={pitch}")
            return xy, angle, pitch, z

        if "--controls-only" in sys.argv:
            from PIL import Image
            start = ["--pad", "9:9.2:0:0:0:0:1000:0:0"]
            baseline = run("guide-baseline", "640x480", *start, boot=False)
            guided = run("controller-guide", "640x480", *start, "--key", "14:35",
                         "--pad", "14.3:17:0:1:0:0:0:0:0", boot=False)
            assert guided == baseline, "The game continued moving behind the guide"
            raw = (project / "assets/controller-help.rgb").read_bytes()
            expected = bytearray()
            for y in range(480):
                for x in range(640):
                    pos = (y*2*1280+x*2)*3
                    expected.extend(raw[pos:pos+3])
            image = Image.open(output / "controller-guide.bmp").convert("RGB")
            assert image.tobytes() == expected, "Original help art was not replaced by the embedded controller guide"
            closed = run("controller-guide-close", "640x480", *start, "--key", "14:35",
                         "--pad", "15:15.2:0:0:0:0:2000:0:0",
                         "--pad", "15.5:17:0:1:0:0:0:0:0", boot=False)
            assert closed[0] != baseline[0], "B did not close the guide and resume controller movement"
            assert struct.unpack_from("<i", config.read_bytes(),4)[0] == 0, "Legacy saved controls were rewritten"
            print("Controller guide matches embedded artwork, pauses, and closes with B.")
            return

        if "--saves-only" in sys.argv:
            names_file = root / "run/GAMENAME.USR"
            names = bytearray(288)
            names[9*24:9*24+13] = b"EXISTING SAVE"
            names_file.write_bytes(names)
            guard = root / "run/SAVE009.DAT"
            guard.write_bytes(b"UNRELATED EXISTING SAVE FILE")
            def button(t, mask):
                return ["--pad", f"{t}:{t+0.2}:0:0:0:0:{mask:x}:0:0"]
            start = button(9, 0x1000)
            save_menu = [*start, *button(14, 0x10), *button(14.7, 4), *button(15.5, 0x1000)]
            before = datetime.now().replace(microsecond=0)
            saved = run("controller-save", "1280x720", *save_menu, *button(16.5, 0x1000),
                        "--seconds", "18", boot=False)
            records = names_file.read_bytes()
            stamp = records[:24].split(b"\0")[0].decode("ascii")
            assert before <= datetime.strptime(stamp, "%Y-%m-%d %H:%M:%S") <= datetime.now(), "Save is not named with local date/time"
            assert records[24:240] == names[24:240], "Saving renamed other slots"
            data_file = root / "run/SAVE000.DAT"
            saved_data = data_file.read_bytes()
            assert len(saved_data) > 100_000, "SaveGame did not serialize the game"
            motion = ["--pad", "14:15:0:1:0:0:0:0:0"]
            moved = run("save-moved", "1280x720", *start, *motion, "--seconds", "16", boot=False)
            assert moved[0] != saved[0], "Load regression needs a changed position"
            loaded = run("controller-load", "1280x720", *start, *motion, *button(16, 0x10),
                         *button(16.5, 2), *button(17.1, 4), *button(17.8, 0x1000),
                         *button(18.8, 0x1000), "--seconds", "22", boot=False)
            assert loaded == saved, "Controller load did not restore the saved game"
            assert data_file.read_bytes() == saved_data, "Loading rewrote the save"
            run("controller-save-cancel", "1280x720", *save_menu, *button(16.5, 0x2000),
                "--seconds", "18", boot=False)
            assert data_file.read_bytes() == saved_data, "Cancelling overwrote the save"
            title_loaded = run("controller-title-load", "1280x720", *button(9, 8),
                               *button(9.8, 0x1000), *button(10.8, 0x1000), "--seconds", "18", boot=False)
            assert title_loaded == saved, "Title load required keyboard input or did not restore the save"
            assert data_file.read_bytes() == saved_data, "Title loading rewrote the save"
            assert names_file.read_bytes()[:240] == records[:240], "Loading/cancelling changed save names"
            assert guard.read_bytes() == b"UNRELATED EXISTING SAVE FILE", "Unrelated save was modified"
            print(f"Save/load checks passed without keyboard input. Save name: {stamp}")
            return

        if "--menus-only" not in sys.argv:
            run("maximized-viewport", "1920x1080", "--seconds", "14", viewport=True)
            if "--viewport-only" in sys.argv:
                return
            base = run("classic", "640x480")
            wide = run("wide", "1920x1080")
            assert base == wide, "Resolution selection changed gameplay camera state"
            run("ultrawide", "3440x1440", "--key", "12:1b", "--key", "12.5:1b", "--key", "13:1b", "--key", "13.5:1b")
            for mode in ("2560x1080", "3840x1080", "3840x1600", "5120x1440", "5120x2160"):
                assert run(f"wide-{mode}", mode) == base, "Ultrawide changed gameplay state"
            run("4k-fire", "3840x2160", "--pad", "14:15:0:0:0:0:0:0:1")
            move = run("left-stick", "1920x1080", "--pad", "14:16:0.6:0.8:0:0:0:0:0")
            look = run("right-stick", "1920x1080", "--pad", "14:16:0:0:0.65:0.5:0:0:0")
            assert all(a != b for a, b in zip(move[0], base[0])), "Left stick did not combine forward/strafe"
            assert move[1:3] == base[1:3], "Left stick changed camera heading/pitch"
            assert look[0] == base[0], "Right stick moved the player"
            assert look[1] != base[1] and look[2] != base[2], "Right stick did not control both camera axes"
            together = run("keyboard-and-pad", "1280x720", "--hold", "14:16:e48",
                "--pad", "14:16:0:0:0.65:0.5:0:0:0")
            assert together[0] != base[0], "Connected pad disabled keyboard movement"
            assert together[1] != base[1] and together[2] != base[2], "Keyboard disabled pad look"
            run("controller-menu", "1280x720", "--pad", "14:14.2:0:0:0:0:10:0:0")
            # Modern A jumps from rest without using Ctrl's stationary hand screen.
            idle = run("standing", "1280x720", "--seconds", "14.15")
            jump = run("controller-jump", "1280x720", "--seconds", "14.15", "--pad", "14:14.4:0:0:0:0:1000:0:0")
            assert jump[:3] == idle[:3], "Stationary jump changed position/heading/pitch"
            assert jump[3] > idle[3], "A did not raise the player from a stationary position"
            hand = run("use-hand", "1280x720", "--seconds", "14.15", "--pad", "14:14.4:0:1:0:0:40:1:0")
            assert hand == idle, "LT hand aiming moved/jumped the player while the run stick was held"
            run("weapons", "1280x720", "--seconds", "14.55", "--pad", "14:14.1:0:0:0:0:100:0:0",
                "--pad", "14.2:14.3:0:0:0:0:8:0:0", "--pad", "14.4:14.5:0:0:0:0:1000:0:0")
            run("powers", "1280x720", "--seconds", "14.55", "--pad", "14:14.1:0:0:0:0:200:0:0",
                "--pad", "14.2:14.3:0:0:0:0:8:0:0", "--pad", "14.4:14.5:0:0:0:0:1000:0:0")
        menu_keys = ("--key", "14:01", "--key", "14.6:18")
        paused = run("game-options", "1280x720", "--seconds", "16", *menu_keys)
        run("native-options", "1280x720", "--seconds", "16", *menu_keys, "--key", "15:0f")
        old = (output / "game-options.bmp").read_bytes()
        new = (output / "native-options.bmp").read_bytes()
        # Native and DOS pages use the very same logo, palette and square-pixel scaling.
        pitch = 1280 * 3
        for y in range(180):
            start = 54 + (719-y) * pitch + 160 * 3
            assert old[start:start+300*3] == new[start:start+300*3], "Original menu artwork changed"
        colours = set(zip(new[54::3], new[55::3], new[56::3]))  # BGR, no row padding at 1280
        assert any(g > 200 and r < 80 and b < 80 for b, g, r in colours), "VGA menu palette is too dark"
        assert any(min(rgb) > 240 for rgb in colours), "Missing original white selection highlight"
        resumed = run("options-return", "1280x720", "--seconds", "21", *menu_keys,
            "--key", "15:0f", "--key", "15.2:50", "--key", "15.4:4d",  # look speed
            "--key", "15.45:50", "--key", "15.5:50", "--key", "15.55:50",  # Fullscreen row
            "--key", "15.6:1c", "--key", "15.8:01", "--key", "16.2:01", "--key", "17:01",
            "--pad", "18:20:0:1:0:0:0:0:0")
        assert resumed[0] != paused[0], "Options did not return to playable gameplay"
        native_config = (root / "run/NATIVE.CNF").read_text()
        assert "sensitivity=1.25" in native_config, "Settings were not saved"
        assert "fullscreen=true" in native_config, "Fullscreen setting was not saved"
        run("title-options", "1280x720", "--seconds", "11", "--key", "9:18", boot=False)
        run("quit-no-confirmation", "1280x720", "--key", "14:01", "--key", "14.6:10", expect_exit=True)
        assert struct.unpack_from("<i", config.read_bytes(), 4)[0] == 0, "Saved input device was rewritten"
    print(f"Native smoke checks passed. Screenshots/logs: {output}")


if __name__ == "__main__":
    main()
