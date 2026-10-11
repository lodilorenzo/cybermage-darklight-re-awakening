"""Run with: python tools/test_export_source.py (stdlib only, no game assets)."""
from pathlib import Path
import tempfile

from export_source import ASSET_SIZES, GITIGNORE, PROJECT, ROOT_FILES, export


def main():
    attributes = (PROJECT / ".gitattributes").read_text().splitlines()
    assert "*.rgb binary" in attributes and "*.rgba binary" in attributes, "Git must preserve raw pixel buffers"
    with tempfile.TemporaryDirectory() as temp:
        root = Path(temp) / "project"
        expected = set(ROOT_FILES) | set(ASSET_SIZES) | {
            "crates/game/src/main.rs", "crates/game/Cargo.toml",
            "tools/export_source.py", "docs/SOURCE_EXPORT.md",
        }
        for name in expected:
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(bytes(ASSET_SIZES[name]) if name in ASSET_SIZES else b"working tree content\n")
        excluded = {
            ".git/config", ".env", "CMV1.01F.iso", "iso/CYBERMAG/CM_HI.EXE",
            "run/SAVE000.DAT", "target/release/cybermage.exe", "site/fonts/cm-hud.woff2",
            "assets/tank-sides.png", "assets/car-sides.png", "assets/vehicle-sides/tank-left-v3.png",
            "tools/__pycache__/export_source.pyc", "crates/.private/secret.rs",
            "tools/.env", "docs/screenshot.png", "tools/unreviewed.bin",
        }
        for name in excluded:
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"DO NOT EXPORT")
        destination = root / "dist/source"
        export(root, destination)
        actual = {p.relative_to(destination).as_posix() for p in destination.rglob("*") if p.is_file()}
        assert actual == expected | {".gitignore"}, actual ^ expected
        for name in expected:
            assert (destination / name).read_bytes() == (root / name).read_bytes(), name
        assert (destination / ".gitignore").read_text() == GITIGNORE
        try:
            export(root, destination)
        except FileExistsError:
            pass
        else:
            raise AssertionError("Existing exports must never be overwritten")
        try:
            export(root, root / "crates/new-export")
        except ValueError:
            pass
        else:
            raise AssertionError("Exports inside source directories must be rejected")
        (root / "assets/car-sides.rgba").write_bytes(b"invalid")
        try:
            export(root, root / "dist/broken")
        except ValueError:
            pass
        else:
            raise AssertionError("Malformed runtime art must be rejected")
        assert not (root / "dist/broken").exists()
    print("Source export checks passed: isolation, byte parity, no overwrite, destination/asset validation.")


if __name__ == "__main__":
    main()
