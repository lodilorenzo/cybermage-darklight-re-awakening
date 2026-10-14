# CyberMage: Darklight Re-Awakening 1.3.0 — Windows x64

CyberMage: Darklight Re-Awakening is an unofficial compatibility engine for
CyberMage: Darklight Awakening v1.01F.
Windows 10/11 x64. Keyboard/mouse or Xbox/XInput controller. No Rust, Python,
installer, online service, or separate VC++ Redistributable is required to play.
The packaged executable opens only the game/setup windows, with no console/log window.
A multi-size Darklight emblem is embedded for Explorer, the window and taskbar;
no separate icon file is needed.

## First launch

1. Extract the entire ZIP into a writable folder (not Program Files).
2. Run `cybermage.exe`. The first-run asset setup asks for your own commercial
   **CyberMage v1.01F** game files, which are NOT included.
3. Select either a raw ISO-9660 `.iso` CD image, or `CM_HI.EXE` inside an
   installed/extracted `CYBERMAG` folder. Selecting that file imports the folder.
   BIN/CUE and other executable versions are not supported.
4. Setup validates the supported executable and required files, copies only
   known game assets into `iso/CYBERMAG`, then starts the game. The source is
   unchanged; saves, save names and control/display settings are not imported.
   Required language/sound setup files are copied as game data; DOS paths are
   generated locally, not taken from the source installation. Cancel exits cleanly.

Asset import is offline: it does not download the game, bypass licensing,
verify ownership or provide commercial content. Supply lawfully obtained data.
Incomplete/unsupported imports are not published. Existing asset directories
are never overwritten; use a fresh extracted package if repairing an import.

Command-line folder/ISO setup is also available:

```
cybermage.exe --import-assets "D:\CYBERMAG"
cybermage.exe --import-assets "D:\CyberMage-v1.01F.iso"
```

The import command exits after setup; launch normally afterward. `--root PATH`
chooses a different writable installation root. Otherwise assets, saves and
settings live beside the executable, regardless of the shortcut working folder.

## Controls and settings

WASD move/strafe, mouse look, left click fire/cast, right click hand aim,
Space jump, Ctrl crouch, Shift run, E interact/enter vehicle. 1/2/3 select
weapons/powers/inventory. Esc opens the original save/load/settings/quit menu.
Save names are automatic local date/time stamps; you never need to type one.

F1 opens the illustrated keyboard/mouse guide. Left/Right or LB/RB switches
between keyboard/mouse and Xbox 360 guides; Esc/B closes. F4/Back opens Actions
for less-used commands. See `CONTROLS.md` and the two guide PNGs.

Options has Game/Enhancements/Actions tabs: Q/E or LT/RT cycles tabs; Page Up/Down
or LB/RB changes Actions subpages. Enhancements offers output resolutions up to
5120x2160, look speed, controller deadzone, inverted look, borderless
fullscreen and V-Sync. V-Sync synchronizes presentation to the Windows desktop
refresh in windowed/fullscreen mode, replacing the default 60 FPS cap. It is off
by default and saved in `run/NATIVE.CNF`; movies retain 15 FPS playback.
The game targets 60 FPS by adapting the 3D world's internal
resolution; HUD and menus stay at the selected output size. HUD, hands,
menus, movies and guides keep correct proportions.

**Enhancements → Renderer → Vulkan** enables experimental Windows GPU
presentation/scaling. Software remains the default and fallback; world rendering
is still CPU-side. Renderer/resolution/fullscreen changes apply when leaving
Enhancements. Vulkan needs a compatible driver and may still cause audio underruns.

Controller support is automatic/hotplug; no controller is required. Focus loss
releases inputs, frees the mouse and pauses the simulation. Disconnection leaves
keyboard/mouse available. Windows pointer speed/acceleration affects mouse look;
Look Speed provides additional tuning. Movies skip with Esc/Enter/Space or
A/B/Start. Close the window or use Quit; quit confirmation is skipped.

## Data and troubleshooting

- Original assets: `iso/CYBERMAG` (read-only to the engine).
- Your saves/configuration: `run` (created on first play). Back it up before
  upgrading or replacing an installation. Nothing in `run` ships in this ZIP.
- Native display settings: `run/NATIVE.CNF`.
- Music defaults to Windows MIDI. **Options → Enhancements → Music Output** selects
  Windows MIDI or the embedded Nuked-SC55. For SC-55, supply a complete lawfully
  obtained SC-55 or SC-55mkII firmware/wave ROM set in `run/sc55/` beside the
  executable (under your chosen `--root` if overridden). Known ROMs are identified
  by SHA-256, regardless of filenames. Original SC-55 is preferred if both models
  are present. No ROMs are bundled or downloaded. Firmware boot can take several
  seconds; switching restarts the song. Missing/invalid ROMs or unavailable audio
  leave the previous output selected; startup falls back to Windows MIDI.
  The choice is saved in `run/NATIVE.CNF`. A missing Windows MIDI device disables
  that backend's music, not sound effects. SC-55 synthesis now runs on a dedicated
  worker; initial-run audio underruns remain a known limitation.
- Only the fingerprinted v1.01F high-resolution executable is supported.
  A patched or different-language executable may need a separate port.
- This is a work-in-progress compatibility engine. Automated input/import,
  renderer, save/menu and collision regressions are not full-campaign or
  physical controller/audio validation. Report any reproducible problem.

## Redistribution

Combined executable: GPL-2.0-or-later (`COPYING`) because it embeds Nuked-SC55.
Original engine code and generated guides retain their MIT grant (`LICENSE`).
`SOURCE.zip` includes corresponding project/backend source, Cargo dependency
source and build scripts; extract it and run `cargo build --locked --release`
with Rust and a C++23 compiler (MSVC on Windows). Keep source/license material
with redistributed builds. Dependencies and copyright notices:
`THIRD_PARTY_NOTICE.md`, `DEPENDENCIES.txt`, `licenses/`.
Commercial CyberMage files and trademarks remain with their rights holders.
The embedded icon is cropped from the original Darklight logo, retains its
original commercial-artwork rights, and is not covered by the MIT license.
This independent project is not affiliated with Origin or Electronic Arts.

Redistribute ONLY the original clean ZIP. Do not redistribute your installation
once commercial assets or personal saves/configuration have been added.
`SHA256SUMS` checks the packaged files; the adjacent ZIP `.sha256` checks the ZIP.
