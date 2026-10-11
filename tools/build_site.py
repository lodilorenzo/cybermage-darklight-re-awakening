"""Copy the static GitHub Pages landing page to a fresh preview directory.
Usage: python tools/build_site.py [DESTINATION] (default: dist/landing-page)
No game installation, extracted fonts, Pillow or fontTools are needed.
GitHub Pages serves the committed docs/ files directly; this is only for previews.
"""
import argparse
from pathlib import Path
import shutil
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SCREENSHOT_NAMES = ("classic", "wide", "ultrawide", "options", "menu", "guide", "lab", "gate", "forest")
PAGE_FILES = ("index.html", "img/title.png", ".nojekyll", *(
    f"img/screenshots/{name}.png" for name in SCREENSHOT_NAMES
))


def build(destination):
    destination = Path(destination).resolve()
    if destination.exists():
        raise FileExistsError(f"Refusing to overwrite {destination}; choose a fresh preview directory")
    sources = [ROOT / "docs" / name for name in PAGE_FILES]
    for source in sources:
        if not source.is_file() or source.is_symlink():
            raise ValueError(f"Missing or linked landing-page file: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="landing-page-", dir=destination.parent) as temp:
        stage = Path(temp) / "page"
        stage.mkdir()
        for name, source in zip(PAGE_FILES, sources):
            target = stage / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
        if destination.exists():
            raise FileExistsError(f"Refusing to overwrite {destination}")
        stage.rename(destination)
    return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", nargs="?", type=Path, default=ROOT / "dist/landing-page")
    args = parser.parse_args()
    try:
        destination = build(args.destination)
    except (OSError, ValueError) as exc:
        parser.exit(1, f"Preview failed: {exc}\n")
    print(f"Static landing page copied to {destination}")
    print(f'Preview: python -m http.server 8000 --directory "{destination}"')


if __name__ == "__main__":
    main()
