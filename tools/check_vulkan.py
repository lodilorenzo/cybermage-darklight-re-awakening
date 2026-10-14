"""ROM-free Windows GPU readback/resize test; optionally enable Khronos sync validation.
--register-validation MANIFEST temporarily registers an explicit layer in HKLM (admin,
explicit permission required), restores its prior value in finally, and records a backup.
No SDK/driver installation. Build/use Vulkan without this option or without the SDK.
"""
from pathlib import Path
import argparse
import json
import os
import subprocess


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--validation", action="store_true")
    parser.add_argument("--game", action="store_true", help="also check real-game fullscreen presentation in an isolated root")
    parser.add_argument("--register-validation", type=Path, help="explicitly permit temporary HKLM registration of this layer manifest")
    args = parser.parse_args()
    if os.name != "nt":
        parser.error("this initial Vulkan backend requires Windows")
    if args.register_validation and not args.validation:
        parser.error("register-validation requires --validation")
    output = project / "out/vulkan-check"
    output.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ)
    if args.validation:
        env["CM_VK_VALIDATION"] = "1"
    else:
        env.pop("CM_VK_VALIDATION", None)
    key = None
    created = False
    old = None
    manifest = None
    backup = {}
    try:
        if args.register_validation:
            import winreg
            manifest = str(args.register_validation.resolve(strict=True))
            description = json.loads(Path(manifest).read_text())
            assert description["layer"]["name"] == "VK_LAYER_KHRONOS_validation"
            location = r"SOFTWARE\Khronos\Vulkan\ExplicitLayers"
            try:
                key = winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, location, 0, winreg.KEY_QUERY_VALUE | winreg.KEY_SET_VALUE | winreg.KEY_WOW64_64KEY)
            except FileNotFoundError:
                key = winreg.CreateKeyEx(winreg.HKEY_LOCAL_MACHINE, location, 0, winreg.KEY_QUERY_VALUE | winreg.KEY_SET_VALUE | winreg.KEY_WOW64_64KEY)
                created = True
            try:
                old = winreg.QueryValueEx(key, manifest)
                assert old[1] == winreg.REG_DWORD, "refusing to alter an unexpected registry value type"
            except FileNotFoundError:
                pass
            backup = {"key": "HKLM\\" + location, "value": manifest, "original": old, "key_created": created, "restored": False}
            (output / "validation-registry-backup.json").write_text(json.dumps(backup, indent=2))
            winreg.SetValueEx(key, manifest, 0, winreg.REG_DWORD, 0)
        result = subprocess.run(["cargo", "test", "-p", "cybermage", "--release", "vulkan_present_readback_resize_and_teardown", "--", "--ignored", "--nocapture"],
                                cwd=project, env=env, capture_output=True, timeout=240)
        log = result.stdout.decode(errors="replace") + result.stderr.decode(errors="replace")
        (output / ("validation-test.log" if args.validation else "gpu-test.log")).write_text(log)
        assert result.returncode == 0 and "presentation backend: Vulkan" in log and "using software" not in log, log[-6000:]
        print("Vulkan RGB/orientation/scaling/resize/V-Sync/teardown passed" + (" with Khronos synchronization validation" if args.validation else ""), flush=True)
        if args.game:
            subprocess.run(["cargo", "build", "--release"], cwd=project, check=True)
            command = ["python", "tools/check_performance.py", "--window", "--fullscreen", "--vsync", "--renderer", "vulkan", "--modes", "2560x1080", "--native-pixels", "921600", "--repeat", "1", "--seconds", "3", "--turn", "--label", "vulkan/validation-game"]
            if args.validation:
                command.append("--vk-validation")
            subprocess.run(command, cwd=project, env=env, check=True, timeout=120)
    finally:
        if key is not None:
            # Restore only this value; do not disturb other applications' layer entries.
            if old is not None:
                winreg.SetValueEx(key, manifest, 0, old[1], old[0])
            else:
                try:
                    winreg.DeleteValue(key, manifest)
                except FileNotFoundError:
                    pass
            empty = winreg.QueryInfoKey(key)[:2] == (0, 0)
            winreg.CloseKey(key)
            if created and empty:
                winreg.DeleteKeyEx(winreg.HKEY_LOCAL_MACHINE, location, winreg.KEY_WOW64_64KEY)
            backup["restored"] = True
            (output / "validation-registry-backup.json").write_text(json.dumps(backup, indent=2))
            print("Validation registry entry restored")


if __name__ == "__main__":
    main()
