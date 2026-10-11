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
    parser.add_argument("--native-pixels", type=int, help="disable adaptive resolution at this 3D pixel budget")
    args = parser.parse_args()
    if args.repeat < 1 or not 0 < args.seconds <= 60 or args.verify_every < 1:
        parser.error("repeat/verify-every must be positive; seconds must be in (0, 60]")
    if args.native_pixels is not None and args.native_pixels < 1:
        parser.error("native-pixels must be positive")
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
    if args.scene is not None:
        env["CM_SCENE"] = str(args.scene)
    if args.native_pixels is not None:
        env["CM_NATIVE_PIXELS"] = str(args.native_pixels)
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
        for mode in args.modes:
            for repeat in range(args.repeat):
                controls = ["--pad", "13:120:0:0:0.25:0:0:0:0"] if args.turn else []
                result = subprocess.run([str(args.exe.resolve()), "--headless", "--root", str(root), "--seconds", "14", "--key", "9:1c", "--resolution", mode, *controls], env=env, capture_output=True, timeout=240)
                log = result.stdout.decode(errors="replace") + result.stderr.decode(errors="replace")
                (output / f"{mode}-{repeat}.log").write_text(log)
                assert result.returncode == 0 and "FAULT" not in log and "panicked" not in log, log[-4000:]
                if args.verify:
                    checked = re.search(r"verify: (\d+) checked, (\d+) mismatches", log)
                    assert checked and int(checked[1]) > 0 and int(checked[2]) == 0, log[-4000:]
                    print(checked[0], flush=True)
                    shutil.copyfile(root / "out/final.bmp", output / f"{mode}-{repeat}.bmp")
                    continue
                metrics = re.search(r"benchmark .*", log)
                assert metrics, log[-4000:]
                if "world=" in metrics[0] and mode != "640x480":
                    assert re.search(r"world=[1-9]\d*x[1-9]\d*", metrics[0]), "Benchmark did not reach a native game frame"
                print(metrics[0], flush=True)
                upload = re.search(r"window upload: .*", log)
                if upload:
                    print(upload[0], flush=True)
                shutil.copyfile(root / "out/final.bmp", output / f"{mode}-{repeat}.bmp")
                if args.profile:
                    profile = output / f"{mode}-{repeat}-profile.txt"
                    shutil.copyfile(root / "out/profile.txt", profile)
                    subprocess.run(["python", str(project / "tools/profile.py"), "15", str(profile)], check=True)


if __name__ == "__main__":
    main()
