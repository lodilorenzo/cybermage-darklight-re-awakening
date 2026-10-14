"""Check static assets and the trailer: python tools/test_landing_page.py (stdlib only)."""
from html.parser import HTMLParser
from pathlib import Path
import struct
import tempfile

from build_site import PAGE_FILES, SCREENSHOT_NAMES, build

ROOT = Path(__file__).resolve().parents[1]


class Page(HTMLParser):
    def __init__(self):
        super().__init__()
        self.images, self.ids, self.anchors, self.trailers = [], set(), [], []
        self.image_attrs, self.screenshot_links = [], []

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if "id" in attrs:
            assert attrs["id"] not in self.ids, attrs["id"]
            self.ids.add(attrs["id"])
        if tag == "img":
            assert attrs.get("alt"), "Images need alternative text"
            self.images.append(attrs["src"])
            self.image_attrs.append(attrs)
        if tag == "a" and attrs.get("href", "").startswith("#"):
            self.anchors.append(attrs["href"][1:])
        if tag == "a" and attrs.get("href", "").startswith("img/screenshots/"):
            self.screenshot_links.append(attrs["href"])
        if tag == "iframe":
            self.trailers.append(attrs)
        assert tag != "script", "The static page needs no custom JavaScript"
        assert not (tag == "link" and "href" in attrs), attrs


def main():
    html = (ROOT / "docs/index.html").read_text(encoding="utf-8")
    page = Page()
    page.feed(html)
    screenshots = [f"img/screenshots/{name}.png" for name in SCREENSHOT_NAMES]
    assert page.images == ["img/title.png", *screenshots], page.images
    assert page.screenshot_links == screenshots, "Every screenshot needs a full-size link"
    for attrs in page.image_attrs:
        data = (ROOT / "docs" / attrs["src"]).read_bytes()
        assert data[:8] == b"\x89PNG\r\n\x1a\n", attrs["src"]
        size = struct.unpack(">II", data[16:24])
        assert size == (int(attrs["width"]), int(attrs["height"])), attrs["src"]
        if attrs["src"] in screenshots:
            assert attrs.get("loading") == "lazy" and attrs.get("decoding") == "async"
    assert set(page.anchors) <= page.ids, set(page.anchors) - page.ids
    assert len(page.trailers) == 1, "Exactly one trailer, no placeholder slots"
    trailer = page.trailers[0]
    assert trailer["src"] == "https://www.youtube-nocookie.com/embed/Tg-I3jYtXbk"
    assert trailer.get("title") and trailer.get("loading") == "lazy"
    assert "allowfullscreen" in trailer
    assert trailer.get("referrerpolicy") == "strict-origin-when-cross-origin"
    assert 'href="https://youtu.be/Tg-I3jYtXbk"' in html
    assert 'href="https://lodilorenzo.dev/">lodilorenzo.dev</a>' in html
    assert "data-youtube-id" not in html and "upload coming soon" not in html
    assert "@font-face" not in html and "url(" not in html, "No extracted fonts/background art"
    assert "../dist/" not in html and "../README" not in html, "Use GitHub links, not local paths"
    release = "https://github.com/lodilorenzo/cybermage-darklight-re-awakening/releases/download/v1.3.0/"
    for name in ("cybermage-windows-x64.zip", "cybermage-windows-x64.zip.sha256"):
        assert f'href="{release}{name}"' in html, f"Missing Windows release link: {name}"
    assert "not a prebuilt Windows download" not in html, "Remove obsolete source-only instructions"
    with tempfile.TemporaryDirectory() as temp:
        preview = build(Path(temp) / "preview")
        files = {p.relative_to(preview).as_posix() for p in preview.rglob("*") if p.is_file()}
        assert files == set(PAGE_FILES), files
        for name in PAGE_FILES:
            assert (preview / name).read_bytes() == (ROOT / "docs" / name).read_bytes()
        try:
            build(preview)
        except FileExistsError:
            pass
        else:
            raise AssertionError("Preview must not overwrite existing files")
    print("Landing page checks passed: Windows ZIP/checksum links, allowlisted gallery, image dimensions/links, isolated preview and single YouTube trailer.")


if __name__ == "__main__":
    main()
