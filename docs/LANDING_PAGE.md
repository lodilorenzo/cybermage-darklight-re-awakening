# Landing page (version 1.0.0)

Live URL: https://lodilorenzo.github.io/cybermage-darklight-re-awakening/

GitHub Pages serves `main` → `/docs` directly. `docs/index.html` is the editable
static page; `.nojekyll` disables Jekyll processing. No custom workflow, package
manager, game installation or generator is needed to update the page.

## YouTube trailer

The page embeds one trailer: https://youtu.be/Tg-I3jYtXbk. The two original
placeholder slots have been removed. A native, titled iframe uses
`https://www.youtube-nocookie.com/embed/Tg-I3jYtXbk`, lazy loading, fullscreen
support and no autoplay. A direct YouTube link remains below the player.
No custom page JavaScript or local MP4 downloads are needed.

To change the trailer, update the iframe's video ID, its title and the direct
YouTube link in `docs/index.html`, then update the expected ID/link in
`tools/test_landing_page.py`. Commit/push to `main`; Pages redeploys. Enable
embedding on the YouTube video; private/age-restricted videos may not play
inside an iframe.

## Screenshots and artwork

The page has a responsive, captioned gallery of nine existing captures in
`docs/img/screenshots/`: classic 4:3, 1080p widescreen, ultrawide, native display
settings, the pause menu, the keyboard/mouse guide, and three environment shots.
Images load lazily and link directly to full-size PNGs; no lightbox library or
additional JavaScript is needed. The gallery's PNGs are explicitly allowlisted
by `SCREENSHOT_NAMES` in `tools/build_site.py`; source exports reuse that list.
Update it, the gallery markup/dimensions and tests together when adding images.

The edited cover (`docs/img/title.png`) retains its original copyright notice.
The cover and gameplay screenshots contain commercial artwork whose original
rights remain in force; neither editing the subtitle nor taking screenshots
makes that commercial content MIT licensed. These are explicit retained
exceptions requested by the project owner. Extracted game fonts, background
tiles, heading images, badges and standalone decorative image assets remain
excluded. System fonts and CSS preserve the retro layout instead.

The old private `site/` output is not exported or served by GitHub Pages.
`tools/build_site.py` copies only the static page, cover and allowlisted gallery
to a fresh preview directory; it no longer extracts commercial resources.

## Local preview and checks

```sh
python -m http.server 8000 --directory docs
# Browse http://localhost:8000/
python tools/test_landing_page.py
```

The check uses only Python's standard library to test the image allowlist,
real PNG dimensions, lazy loading/full-size links, local anchors, preview
copying and the single configured YouTube embed/direct link.
Optional isolated preview: `python tools/build_site.py [NEW_DIRECTORY]`.

Version 1.0.0 is initially a **source release**. Download links point to the
GitHub tag/release and source ZIP, not a nonexistent prebuilt game package.
Users must supply their own CyberMage v1.01F files when running the compiled
mod. See `THIRD_PARTY_NOTICE.md` and the source-export notes before publishing
any additional artwork or release files.
