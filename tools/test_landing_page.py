"""Check static assets and trailer embeds: python tools/test_landing_page.py
Requires Node.js for the small DOM-free JavaScript check; no npm dependencies.
"""
from html.parser import HTMLParser
from pathlib import Path
import subprocess
import tempfile

from build_site import PAGE_FILES, build

ROOT = Path(__file__).resolve().parents[1]


class Page(HTMLParser):
    def __init__(self):
        super().__init__()
        self.images, self.ids, self.anchors, self.trailers = [], set(), [], []

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if "id" in attrs:
            assert attrs["id"] not in self.ids, attrs["id"]
            self.ids.add(attrs["id"])
        if tag == "img":
            assert attrs.get("alt"), "Images need alternative text"
            self.images.append(attrs["src"])
        if tag == "a" and attrs.get("href", "").startswith("#"):
            self.anchors.append(attrs["href"][1:])
        if "data-youtube-id" in attrs:
            self.trailers.append(attrs)
        assert not (tag in {"script", "link", "iframe"} and ("src" in attrs or "href" in attrs)), attrs


def main():
    html = (ROOT / "docs/index.html").read_text(encoding="utf-8")
    page = Page()
    page.feed(html)
    assert page.images == ["img/title.png"], page.images
    assert set(page.anchors) <= page.ids, set(page.anchors) - page.ids
    assert len(page.trailers) == 2 and all(p.get("data-title") for p in page.trailers)
    assert "@font-face" not in html and "url(" not in html, "No extracted fonts/background art"
    assert "../dist/" not in html and "../README" not in html, "Use GitHub links, not local paths"
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
    subprocess.run(["node", "-e", r'''
const fs = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const html = fs.readFileSync(process.argv[1], 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)[1];
const panels = ['', 'invalid', 'https://evil.example/video', 'abcdefghij-', 'ABCDEFGHI_1'].map(id => ({
  dataset: {youtubeId: id, title: 'Trailer test'},
  replaceChildren(frame) { this.frame = frame; }
}));
vm.runInNewContext(script, {document: {
  querySelectorAll(selector) { assert.equal(selector, '[data-youtube-id]'); return panels; },
  createElement(tag) { assert.equal(tag, 'iframe'); return {}; }
}});
for (let i = 0; i < 3; i++) assert.equal(panels[i].frame, undefined);
for (const panel of panels.slice(3)) {
  assert.equal(panel.frame.src, 'https://www.youtube-nocookie.com/embed/' + panel.dataset.youtubeId);
  assert.equal(panel.frame.title, panel.dataset.title);
  assert.equal(panel.frame.loading, 'lazy');
  assert.equal(panel.frame.allowFullscreen, true);
  assert.equal(panel.frame.referrerPolicy, 'strict-origin-when-cross-origin');
}
''', str(ROOT / "docs/index.html")], check=True)
    print("Landing page checks passed: cover-only assets, anchors, isolated preview and validated YouTube embeds.")


if __name__ == "__main__":
    main()
