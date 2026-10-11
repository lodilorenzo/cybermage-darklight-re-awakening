# CyberMage: Darklight Re-Awakening

**Game-mod version 1.0.0** · [Landing page](https://lodilorenzo.github.io/cybermage-darklight-re-awakening/)
· [Source release](https://github.com/lodilorenzo/cybermage-darklight-re-awakening/releases/tag/v1.0.0)

*CyberMage: Darklight Re-Awakening* is a Rust reimplementation host for
*CyberMage: Darklight Awakening* (Origin, 1995, v1.01F).  The original game data is read from the CD image; the game runs on
a Rust x86 interpreter with all DOS / DPMI / BIOS / VESA / sound services
implemented natively, and the original functions are being replaced one by
one with native Rust (OpenRCT2-style).  Every native replacement is verified
bit-for-bit against the original code with the built-in differential tester.

## Run

```
cargo build --release
target\release\cybermage.exe
```

The engine needs your own commercial **v1.01F** assets; they are not distributed.
First launch offers a local ISO-9660 `.iso` import or a folder import (select
`CYBERMAG/CM_HI.EXE`). CLI: `cybermage --import-assets PATH [--root DIRECTORY]`.
Imports are validated and atomic, never overwrite existing assets, and do not copy
source saves, save names or control/display settings. Original files go to `iso/CYBERMAG`; saves/configuration
go to `run`. Portable builds use the executable's directory, not the working folder.

**Portable Windows x64 release:** `python tools/package_release.py` creates
`dist/cybermage-windows-x64.zip` from an explicit allowlist, with a static C runtime,
MIT license, dependency notices, generated guides and SHA-256 manifests. It excludes
commercial game data, the ISO, personal saves/settings and screenshots. Packaging enables
`windows-gui` so the executable opens no console/log window; normal Cargo builds
retain the console for diagnostics. Player setup:
[release README](docs/RELEASE_README.md). Original code/guides are MIT; commercial
CyberMage content retains its original rights. See [third-party notice](THIRD_PARTY_NOTICE.md).

**GitHub source export:** `python tools/export_source.py` creates a fresh
`dist/cybermage-source/` from the current working tree, without Git history,
commercial game installation data, personal saves, extracted fonts or cockpit references.
Generated cockpit margins are included separately from the original center,
preserving the existing build/rendering. The static landing page in `docs/`
includes the edited cover and curated feature screenshots as explicit
commercial-art exceptions; these images retain their original rights.
Review [source export notes](docs/SOURCE_EXPORT.md) and artwork redistribution
rights before publishing. [Landing-page editing notes](docs/LANDING_PAGE.md)
explain how to add the YouTube trailers.

### Display and input options

**Options** from the title or pause menu keeps the original game settings and adds
**Game / Display / Actions** tabs in its instruction panel. **Q/E** or **LT/RT**
select the previous/next tab, wrapping through all three; pointer clicks select tabs.
**Page Up/Down** or **LB/RB** switch subpages within Actions only. This also works when **Back** opens Actions
from gameplay or the title screen: the Game tab opens the actual game Options,
not the screen behind the overlay. Use **A** for the highlighted setting/action,
or the right-stick pointer and **X** to select under it on native pages. The pages
reuse the original Options artwork, bitmap font, colours and selection highlight.
Input Device, DOS Joystick Calibration and Remap Keyboard Controls are removed; original
sound, speech/music, brightness/contrast, difficulty and text-delay settings remain.
**F4/Back → Display and Input Options** opens Display directly. Choose a rendering
resolution (640×480 through 5120×2160, including ultrawide), mouse/right-stick sensitivity,
radial deadzone, inverted look and **Fullscreen**. Fullscreen uses a borderless
Windows desktop window. Changes apply when leaving Display and are saved in
`run/NATIVE.CNF`. **B** returns to the original Options screen when opened there,
or to the game when opened through Actions.

The selected resolution is the output size, with additional horizontal
visibility rather than a stretched 4:3 image. To target smooth **60 FPS**, the 3D
world automatically adjusts its internal resolution; HUD and menus stay at the
selected output resolution. HUD, hands, vehicle camera insets, menus and
movies keep their original proportions. The game starts with its viewport maximized
across the entire game window, with the HUD overlaid rather than a decorative border.
**Back → Shrink/Enlarge Game View** adjusts the viewport.
Window resizing and fullscreen scale the selected output; they do not change rendering
resolution. Match the resolution's aspect ratio to your monitor to avoid letterboxing.
You can also launch with `cybermage --resolution 1920x1080`.
Ultrawide choices: **2560×1080, 3440×1440, 3840×1080, 3840×1600,
5120×1440 and 5120×2160**. Choosing Quit exits without a confirmation popup;
the original shutdown sequence still runs.

On Windows, Xbox/XInput controllers are detected automatically, including hotplug.
Other controllers can use Steam Input configured to output an Xbox controller.
**Keyboard/mouse and Xbox controls coexist; no controller is required.** WASD moves/
strafes, mouse looks, left/right clicks fire/aim the hand, Space jumps, Ctrl crouches,
Shift runs, and E interacts/enters vehicles. 1/2/3 select weapons/powers/inventory,
M/Tab toggles the map, Esc opens menus, F1 opens guides and F4 opens Actions.
Full commands, menus and illustrations: [CONTROLS](docs/CONTROLS.md).
Windows mouse look is relative/recentered with cursor confinement during gameplay;
menus use a proportion-correct pointer. Losing focus releases controls, frees the
cursor and pauses simulation; controller disconnection leaves desktop input usable.
Held desktop keys are suppressed on context changes; mouse buttons keep their
original game/menu ownership. Overlay dismissal suppresses both. Both input
methods use the existing DOS command transport; legacy formats/bindings are preserved.

| Controller | Gameplay |
|---|---|
| Left stick | Forward/backward + strafe; partial tilt walks (diagonals normalized) |
| Right stick | Turn + look up/down |
| RT | Fire |
| Hold LT | Use-hand/interaction aiming on foot; RT clicks, release LT to leave |
| X | Open door / press switch / enter vehicle |
| A / B | Dedicated jump (including from rest) / crouch |
| LB / RB | Weapons / Darklight powers |
| Y | Inventory/key selector |
| Hold left-stick click | Run |
| Right-stick click | Centre view |
| D-pad up/down | Fly up/down |
| D-pad left/right | VDU / land or exit vehicle |
| Back | Controller Actions page (automap is the first entry) |
| Start/Menu | Pause menu |

In DOS menus/selectors: left stick or D-pad navigates, **A** accepts, **B** goes back,
and **X** clears selections where supported. Right stick moves the pointer,
**RT** left-clicks and **LT** right-clicks (including secondary shop actions). **RB/LB** answer Yes/No
in confirmation dialogs. Inside Options, triggers switch main tabs instead of
clicking; **X** selects under the pointer on Display/Actions.
Held buttons/triggers retain their original context until release, so opening a
menu cannot turn a held gameplay action into an accidental accept/click.

**F4/Back** opens a paused **Actions** page. D-pad/left stick selects, LB/RB
changes pages, A executes, B/Back closes, Start opens the original game menu.
The selected action is remembered, making repeated map/aim adjustments quicker.
Actions are also accessible through the original Options tabs. Gameplay commands
selected there leave the pause menus before running;
they are unavailable from title-screen Options.

| Additional original function | Controller route |
|---|---|
| Automap on/off, map zoom, map size | Back → named map command |
| Aircar guns: narrow / widen / center aim | Back → named aircar aim command |
| Game view size, VDU monitors | Back → named display command (D-pad left also cycles VDUs) |
| Pause without menus, help, skip NPC speech | Back → named command |
| Display/input settings | Back → Display and Input Options |
| Save, load, restart, continue, difficulty/sound options, quit | Start → original menus; navigate with D-pad/left stick or the pointer, accept with A/RT |
| Any weapon/power/item, including entries 13–20 | Selector → navigate or use the pointer, then A/RT |

**Save/load needs no typing.** Select a save slot and accept with **A/RT**: its
name is generated automatically from your local date and time, for example
**2026-10-10 19:05:09**. Loading uses the same slot selection; **B** cancels.
Other slots and existing save formats are unchanged. The virtual **Keys** page
has been removed. **Esc/Enter/Space** or **A/B/Start** skip movies.
Xbox 360 controllers use the existing Windows XInput backend.

Layout checked against both pages of [Origin’s original CyberMage reference card](https://www.pixsoriginadventures.co.uk/downloads/)
(the “Cybermage – Reference Card” download). The card specifies **Forward + Ctrl**
for jumping: Ctrl alone is contextual use-hand input. Modern A uses the original
jump physics without that hand-screen fallback; Space shares this dedicated jump.

### In-game illustrated guides

**F1** opens the new keyboard/mouse guide; **F4/Back → Control Guides** opens the
last-used input guide. **Left/Right** or **LB/RB** switches between the keyboard/mouse
and Xbox 360 diagrams; **Esc/B** closes. Gameplay stays paused underneath.
Both are original image-generated, late-1990s-style pixel art, letterboxed in 4:3.
Artwork and briefs: `assets/{keyboard,controller}-help.{png,prompt.txt}`. RGB exports
are embedded with no runtime PNG decoder. Regenerate an export with
`python tools/prepare_controller_help.py assets/keyboard-help.png assets/keyboard-help.rgb`
(or the same command with `controller-help`).

Headless `--key`/`--hold` remain **diagnostic injections only**, not live game inputs.
Music is played through the Windows General MIDI synthesizer; the original music
volume setting uses its correct 0–127 scale.

### Title screen

The window and setup dialogs are titled *CyberMage: Darklight Re-Awakening*. The
title screen's "DARKLIGHT AWAKENING" line is re-set as "DARKLIGHT RE-AWAKENING"
when the art is read (`rebrand.rs`): the original glyphs are reused (R, E and E's
bar as the hyphen) and condensed to the original width. "DARKLIGHT RE-AWAKENING" is
also written in the game's small font 0 below the title menu and under the logo of
the menu screens (Options, including the new tabs, key commands, keyboard, personal
log; that screen's label moves down to make room). The patch is derived from your
imported files at runtime, so the release contains no commercial artwork; it is
skipped where the art does not match the English v1.01F layout (French/German menu
headers keep their original labels).

## What is native Rust so far

- Whole platform layer: LE loader, DOS (files, exec), DPMI, BIOS, VESA banked
  video + page flipping, PIC/PIT, keyboard, mouse (`crates/game/src/machine.rs`).
- Sound: the `Sounds` class (HMI SOS replacement): sample mixer, HMI MIDI
  sequencer (`audio.rs`, `music.rs`, `native/sound.rs`).
- Movies: EA TGV decoder/player replacing `PLAYTGV.EXE` (`tgv.rs`).
- Renderer (image1.cpp): ray casting (`RayCast`, `CheckTile`), floors and
  ceilings (`DrawFlat`, `DrawTopFlat`), walls (`DrawWall`, `DrawPackWall`),
  sky (`DrawBackdrop`), helpers; HUD drawing (`DrawMonitor`,
  `DrawXScaleGlow`).

## Developer tools

See `CLAUDE.md` and `REVERSE.md`.  Useful switches:

- `--headless --seconds N --shots S --key T:SC --hold T1:T2:SC` scripted runs
- `--resolution WIDTHxHEIGHT` select native output without changing saved settings
- `--pad START:END:LX:LY:RX:RY:BUTTONS_HEX:LT:RT` synthetic controller hold for headless checks
- `--kb START:END:KEY+KEY:DX/s:DY/s:BUTTONS:WHEEL[:X:Y]` synthetic desktop hold (`NONE` for no keys; mouse buttons 1=left, 2=right)
- `python tools/check_release.py` checks ZIP/manifest isolation, folder/raw-ISO parity, version rejection, modern movement/look/jump/guides and keyboard save/load; requires locally owned assets
- `cargo test -p cybermage`; `python tools/check_native.py` (isolated saves, screenshots in `out/native-check/`)
- `[CM_VEHICLE=N] [CM_SCENE=N] python tools/check_vehicle.py SLOT|-1 [MODE ...] [-- ARGS]` loads a save copy (or a new game) and screenshots it to `out/vehicle-check/`; `CM_VEHICLE=N` lets Use enter scene vehicle entry N from anywhere (debug; with `--key T:1c`)
- `python tools/prepare_vehicle_sides.py guides|build ...` rebuilds the widescreen tank/aircar cockpit side art (see `assets/vehicle-sides/README.txt`)
- `python tools/check_saved_door.py [save.dat]` verifies the reported `SAVE003.DAT` doorway using an isolated copy; screenshots in `out/saved-door-check/`
- `CM_VERIFY=Func,Func|all` differential verification of native ports (forces classic output; host UI/input hooks are disabled)
- `CM_NO_HOOK=Func` keep the original code for a function
- `CM_PROFILE=1` guest profile (`python tools/profile.py`)
- `python tools/check_performance.py --window --repeat 3` isolated uncapped frame-work benchmark with p50/p95/p99 timings and real Windows upload; `--turn` adds camera motion, `--scene N` changes level (omit `--window` for headless capacity)
- `CM_BENCH_SECONDS=N` benchmark after a headless scripted boot and two-second warmup; `CM_BENCH_WINDOW=1` includes window upload
- `CM_NATIVE_PIXELS=11059200` disable adaptive resolution reduction for native-quality diagnostic comparisons
- `--tgv-dump FILE N` decode movie frames to `out\`
