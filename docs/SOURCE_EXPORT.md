# GitHub source export

From the project directory:

```sh
python tools/export_source.py
```

This creates `dist/cybermage-source/`, ready to review and initialize as a new
Git repository. An optional destination can be outside the project or under
`dist/`; it must not already exist:

```sh
python tools/export_source.py ../cybermage-public
```

The script uses its own location to find the project, so it also works from
another working directory. It copies current files, including uncommitted and
untracked source, not the Git index or commit history. It does not publish,
modify the original files, build binaries, or initialize Git.

## Included and excluded

Included: Rust workspace and lockfile, Python/shell tools, Markdown docs,
reverse-engineering notes/address tables, license/notices, original control
guides and the generated cockpit **margins only**, plus the static GitHub Pages
landing page (`docs/index.html`, `docs/.nojekyll`, `docs/img/title.png` and nine
curated PNGs in `docs/img/screenshots/`). The edited cover and feature screenshots
are explicit commercial-art exceptions requested for the landing page; their
commercial content is not MIT licensed. A fresh `.gitignore` excludes local
game installations, saves, build outputs and artwork references. The included
`.gitattributes` marks raw pixel buffers as binary so Git cannot corrupt their
bytes through newline conversion.

Excluded: ISO/game executables and resources, `run/`, saves/settings, `.git/`,
agent state, build outputs, non-allowlisted screenshots, the old extracted website/fonts,
cockpit reference/input images, and combined cockpit previews. Files outside the
explicit allowlist are not copied. Symlinks in selected source trees are
rejected, and embedded runtime asset sizes are validated before copying.

## Cockpit asset split: full functionality without the original center

`assets/{tank,car}-sides.rgba` already contain two 534×480 RGBA images, stored
**left then right**, with no header. They do not contain the original 640×480
cockpit center. `tools/prepare_vehicle_sides.py` crops the generated margins
before writing these files; its `*-sides.png` previews add the original center
and are intentionally **not exported**. Generation inputs also contain original
reference strips and are not exported.

The Rust renderer embeds only these `.rgba` margins. It obtains the original
center, palette, and other commercial assets from the user's own v1.01F files
at runtime. The exported source therefore preserves the existing rendering,
including transparent aircar windows, without embedding that original center.
No renderer change or placeholder is needed. Build as usual:

```sh
cargo build --locked --release
python tools/test_export_source.py
```

Users still need to import their own commercial game files. Rebuilding cockpit
art needs private generation inputs and locally owned assets; compiling does
not. The public export can be exported again using the same script.

## Review before publishing

The exporter is a file allowlist, **not a secret scanner or legal clearance**.
Review source/docs and licensing before pushing. Removing original/reference
pixels does not automatically remove possible derivative-work, copyright or
trademark restrictions on generated extensions. The existing MIT license does
not grant rights to commercial CyberMage content; retain the license and
third-party notice and obtain any necessary permission for the added artwork
and edited commercial cover/screenshots. See [landing-page notes](LANDING_PAGE.md)
for the curated gallery and trailer configuration.
Always publish this clean export, not the original working repository/history
or a directory after importing game assets.
