"""Export current working-tree source, not Git history or commercial game data.
Usage: python tools/export_source.py [DESTINATION]
The destination must not exist; defaults to dist/cybermage-source.
"""
import argparse
from pathlib import Path
import shutil
import tempfile

from build_site import PAGE_FILES

PROJECT = Path(__file__).resolve().parents[1]
ROOT_FILES = (
    ".gitattributes", "Cargo.toml", "Cargo.lock", "README.md", "LICENSE", "COPYING", "THIRD_PARTY_NOTICE.md",
    "CLAUDE.md", "REVERSE.md", "dead_ends.md", "labels.csv", "fields.csv",
    "assets/cybermage.ico",
    "assets/controller-help.png", "assets/controller-help.prompt.txt",
    "assets/keyboard-help.png", "assets/keyboard-help.prompt.txt",
    "assets/vehicle-sides/README.txt",
    "third_party/dasp_sample-0.11.0/LICENSE-MIT",
    "third_party/dasp_sample-0.11.0/NOTICE",
    "third_party/nuked-sc55/LICENSE", "third_party/nuked-sc55/NOTICE", "third_party/nuked-sc55/README.md",
    *(f"docs/{name}" for name in PAGE_FILES),
)
# Raw cockpit assets are already split: left margin then right, no original center.
ASSET_SIZES = {
    "assets/controller-help.rgb": 1280 * 960 * 3,
    "assets/keyboard-help.rgb": 1280 * 960 * 3,
    "assets/tank-sides.rgba": 2 * 534 * 480 * 4,
    "assets/car-sides.rgba": 2 * 534 * 480 * 4,
}
TREE_SUFFIXES = {"crates": {".rs", ".toml", ".cpp"}, "tools": {".py", ".sh"}, "docs": {".md"},
                 "third_party/nuked-sc55/src/backend": {".cpp", ".c", ".h", ".in"}}
GITIGNORE = """/target/
/dist/
/iso/
/run/
/out/
/gfx/
/scratch/
/site/
/.pi/
*.iso
__pycache__/
*.py[cod]
.env
.env.*
# Local cockpit references/previews may contain commercial artwork.
/assets/vehicle-sides/*
!/assets/vehicle-sides/README.txt
/assets/*-sides.png
"""


def export(root, destination, *, include_site=True):
    root = Path(root).resolve()
    destination = Path(destination).absolute()
    if destination.exists() or destination.is_symlink():
        raise FileExistsError(f"Refusing to overwrite {destination}")
    destination = destination.resolve()
    if (root.is_relative_to(destination) or
            (destination.is_relative_to(root) and destination.relative_to(root).parts[0] != "dist")):
        raise ValueError("Export outside the project or under dist/, never into source directories")

    files = set(ROOT_FILES) | set(ASSET_SIZES)
    if not include_site:
        files.difference_update(f"docs/{name}" for name in PAGE_FILES)
    # ponytail: explicit trees/extensions, not a secret scanner; review source before publishing.
    for tree, suffixes in TREE_SUFFIXES.items():
        base = root / tree
        if not base.is_dir() or base.is_symlink():
            raise ValueError(f"Expected a regular source directory: {base}")
        for path in base.rglob("*"):
            relative = path.relative_to(root)
            if any(part.startswith(".") or part in {"__pycache__", "target"} for part in relative.parts):
                continue
            if path.is_symlink():
                raise ValueError(f"Refusing source symlink: {relative}")
            if path.is_file() and path.suffix in suffixes:
                files.add(relative.as_posix())
    for name in sorted(files):
        path = root / name
        if any(p.is_symlink() for p in (path, *path.parents) if p != root and p.is_relative_to(root)):
            raise ValueError(f"Refusing source symlink: {name}")
        if not path.is_file():
            raise FileNotFoundError(f"Missing required source file: {name}")
        if name in ASSET_SIZES and path.stat().st_size != ASSET_SIZES[name]:
            raise ValueError(f"Unexpected embedded asset size: {name}")

    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="source-export-", dir=destination.parent) as temp:
        stage = Path(temp) / "source"
        stage.mkdir()
        for name in sorted(files):
            target = stage / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(root / name, target)
        (stage / ".gitignore").write_text(GITIGNORE, encoding="utf-8")
        if destination.exists() or destination.is_symlink():
            raise FileExistsError(f"Refusing to overwrite {destination}")
        stage.rename(destination)
    return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", nargs="?", type=Path, default=PROJECT / "dist/cybermage-source")
    args = parser.parse_args()
    try:
        destination = export(PROJECT, args.destination)
    except (OSError, ValueError) as exc:
        parser.exit(1, f"Export failed: {exc}\n")
    print(f"Created {destination}")
    print("Current source, cockpit margins and static landing page included; no Git history or game installation.")
    print("The landing-page cover and curated screenshots retain their original commercial-content rights.")
    print("Review docs/SOURCE_EXPORT.md and redistribution rights before publishing.")


if __name__ == "__main__":
    main()
