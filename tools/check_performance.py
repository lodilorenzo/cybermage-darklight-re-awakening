"""Isolated frame benchmark. Build release first; never touches personal saves.
python tools/check_performance.py [--exe PATH] [--seconds 5] [--repeat 3] [--modes 1920x1080 3840x2160]
"""
from pathlib import Path
import argparse
import os
import re
import shutil
import subprocess
import tempfile


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, default=project / "target/release" / ("cybermage.exe" if os.name == "nt" else "cybermage"))
    parser.add_argument("--seconds", type=float, default=5)
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--modes", nargs="+", default=["1920x1080", "3840x2160"])
    parser.add_argument("--label", default="current")
    parser.add_argument("--turn", action="store_true", help="turn the camera continuously during measurement")
    parser.add_argument("--scene", type=int, help="benchmark a different new-game scene")
    parser.add_argument("--verify-every", type=int, default=1)
    parser.add_argument("--verify", help="differential-check named hooks (forces DOS resolution)")
    parser.add_argument("--profile", action="store_true", help="capture guest instruction samples")
    parser.add_argument("--window", action="store_true", help="include real window upload (requires a desktop)")
    parser.add_argument("--renderer", choices=["software", "vulkan"], default="software", help="window presentation backend (Vulkan currently Windows-only)")
    parser.add_argument("--renderer-pair", action="store_true", help="alternate software/Vulkan within one boot at a fixed world budget")
    parser.add_argument("--vk-validation", action="store_true", help="enable installed Khronos synchronization validation (not for speed measurements)")
    parser.add_argument("--native-pixels", type=int, help="disable adaptive resolution at this 3D pixel budget")
    parser.add_argument("--fullscreen", action="store_true", help="measure borderless monitor-sized upload")
    parser.add_argument("--vsync", action="store_true", help="include compositor pacing (requires --window)")
    parser.add_argument("--audio", action="store_true", help="include real-device audio output")
    parser.add_argument("--check-audio", action="store_true", help="require zero measured underruns/stale PCM clipping and a live synthesizer")
    parser.add_argument("--music-output", choices=["windows", "sc55"], default="windows", help="sc55 also enables audio and uses your run/sc55 ROMs")
    args = parser.parse_args()
    if args.repeat < 1 or not 0 < args.seconds <= 60 or args.verify_every < 1:
        parser.error("repeat/verify-every must be positive; seconds must be in (0, 60]")
    if args.native_pixels is not None and args.native_pixels < 1:
        parser.error("native-pixels must be positive")
    if (args.fullscreen or args.vsync) and not args.window:
        parser.error("fullscreen/vsync require --window")
    if args.vk_validation and not (args.renderer == "vulkan" or args.renderer_pair):
        parser.error("vk-validation requires a Vulkan renderer")
    if args.renderer_pair and args.repeat > 20:
        parser.error("renderer-pair requires repeat <= 20")
    if (args.renderer == "vulkan" or args.renderer_pair) and (not args.window or args.verify or os.name != "nt"):
        parser.error("Vulkan benchmarking requires --window on Windows and no --verify")
    if args.check_audio and (args.verify or not (args.audio or args.music_output == "sc55")):
        parser.error("check-audio requires real-device audio and no --verify")
    if args.music_output == "sc55" and not (project / "run/sc55").is_dir():
        parser.error("SC-55 requires your ROMs in run/sc55")
    output = project / "out/performance-check" / args.label
    output.mkdir(parents=True, exist_ok=True)
    env = {k: v for k, v in os.environ.items() if not k.startswith("CM_")}
    if not args.verify:
        env["CM_BENCH_SECONDS"] = str(args.seconds)
    if args.verify:
        env["CM_VERIFY"] = args.verify
        env["CM_VERIFY_EVERY"] = str(args.verify_every)
    if args.profile:
        env["CM_PROFILE"] = "1"
        env["CM_PROFILE_START"] = "14"
    if args.window:
        env["CM_BENCH_WINDOW"] = "1"
    if args.audio or args.music_output == "sc55":
        env["CM_BENCH_AUDIO"] = "1"
    if args.scene is not None:
        env["CM_SCENE"] = str(args.scene)
    if args.native_pixels is not None or args.renderer_pair:
        env["CM_NATIVE_PIXELS"] = str(args.native_pixels or 1280 * 720)
    if args.renderer_pair:
        env["CM_BENCH_RENDERERS"] = str(args.repeat)
    if args.vk_validation:
        env["CM_VK_VALIDATION"] = "1"
    with tempfile.TemporaryDirectory(prefix="cybermage-perf-") as directory:
        root = Path(directory)
        if os.name == "nt":
            subprocess.run(["cmd", "/c", "mklink", "/J", str(root / "iso"), str(project / "iso")], check=True, capture_output=True)
        else:
            (root / "iso").symlink_to(project / "iso", target_is_directory=True)
        (root / "run").mkdir()
        for name in ("CMDATA.CNF", "CMPATH.CNF"):
            if (project / "run" / name).exists():
                shutil.copyfile(project / "run" / name, root / "run" / name)
        (root / "run/NATIVE.CNF").write_text(
            f"fullscreen={str(args.fullscreen).lower()}\nvsync={str(args.vsync).lower()}\n"
            f"music_output={args.music_output}\nrenderer={args.renderer}\n")
        if args.music_output == "sc55":
            roms = project / "run/sc55"
            if os.name == "nt":
                subprocess.run(["cmd", "/c", "mklink", "/J", str(root / "run/sc55"), str(roms)], check=True, capture_output=True)
            else:
                (root / "run/sc55").symlink_to(roms, target_is_directory=True)
        for mode in args.modes:
            for repeat in range(1 if args.renderer_pair else args.repeat):
                controls = ["--pad", "13:120:0:0:0.25:0:0:0:0"] if args.turn else []
                result = subprocess.run([str(args.exe.resolve()), "--headless", "--root", str(root), "--seconds", "14", "--key", "9:1c", "--resolution", mode, "--renderer", args.renderer, *controls], env=env, capture_output=True, timeout=240)
                log = result.stdout.decode(errors="replace") + result.stderr.decode(errors="replace")
                (output / f"{mode}-{repeat}.log").write_text(log)
                assert result.returncode == 0 and "FAULT" not in log and "panicked" not in log, log[-4000:]
                if args.verify:
                    checked = re.search(r"verify: (\d+) checked, (\d+) mismatches", log)
                    assert checked and int(checked[1]) > 0 and int(checked[2]) == 0, log[-4000:]
                    print(checked[0], flush=True)
                    shutil.copyfile(root / "out/final.bmp", output / f"{mode}-{repeat}.bmp")
                    continue
                if args.window:
                    expected = ["Software", "Vulkan"] if args.renderer_pair else ["Vulkan" if args.renderer == "vulkan" else "Software"]
                    assert all(f"presentation backend: {name}" in log for name in expected) and "using software" not in log, "Requested renderer failed: " + log[-4000:]
                metrics = re.findall(r"benchmark \d+x\d+: .*", log)
                assert len(metrics) == (args.repeat * 2 if args.renderer_pair else 1), log[-4000:]
                for line in metrics:
                    if "world=" in line and mode != "640x480":
                        assert re.search(r"world=[1-9]\d*x[1-9]\d*", line), "Benchmark did not reach a native game frame"
                if args.check_audio:
                    underruns = re.findall(r"audio underrun callbacks: (\d+)",log)
                    assert len(underruns) == len(metrics) and all(int(n) == 0 for n in underruns), "Audio underruns: " + str(underruns)
                    if args.music_output == "sc55":
                        clipped = re.findall(r"SC-55 stale PCM frames discarded: (\d+)",log)
                        assert len(clipped) == len(metrics) and all(int(n) == 0 for n in clipped), "SC-55 PCM clipping: " + str(clipped)
                for line in log.splitlines():
                    if line.startswith(("presentation comparison", "presentation backend:", "Vulkan device:", "GPU transfer/blit:", "benchmark ", "stages:", "window upload:", "audio underrun callbacks:", "SC-55 stale PCM frames discarded:")):
                        print(line, flush=True)
                shutil.copyfile(root / "out/final.bmp", output / f"{mode}-{repeat}.bmp")
                if args.profile:
                    profile = output / f"{mode}-{repeat}-profile.txt"
                    shutil.copyfile(root / "out/profile.txt", profile)
                    subprocess.run(["python", str(project / "tools/profile.py"), "15", str(profile)], check=True)


if __name__ == "__main__":
    main()
