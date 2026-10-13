# CyberMage: Darklight Awakening — Reverse Engineering Notes

Target: `iso/CYBERMAG/CM_HI.EXE` (v1.01F, 11-29-95), Watcom C/C++32 + DOS/4GW, LE format.
Approach: OpenRCT2-style hybrid. The original x86 code runs on a Rust interpreter
(`crates/x86`) with native HLE of DOS/DPMI/BIOS/VGA/VESA/sound (`crates/game`).
Game functions are replaced one by one by native Rust (`crates/game/src/native`).

## Desktop controls and asset-free release
- The earlier gamepad-only input decision is superseded: `desktop.rs` maps modern
  keyboard/mouse commands onto the same controller/DOS transport, without rewriting
  legacy `inDevice` or key assignments. WASD uses normalized native movement;
  Space uses the existing dedicated jump. Windows relative mouse movement is
  recentered/confined only during gameplay. Heading subtracts 10 angle units/pixel
  (matching right-stick direction); pitch adds half the screen-Y delta, preserving fractional
  low-sensitivity motion. Each sampled delta is consumed once, independent of guest
  polling rate. Both use the existing Look Speed/Invert settings.
- Desktop context changes suppress held keyboard keys; mouse buttons retain the
  shared Keys game/menu ownership so held hand aiming survives state 3 without
  turning held fire into a menu click. Overlay dismissal suppresses both sources.
  DOS mouse coordinates are 640x240; shared UI pointers are 640x480 (Y doubled).
  An unmoved desktop pointer must not undo original keyboard cursor repositioning.
  Windows async state filters stale minifb keys after focus loss; mouse
  buttons use fresh async state. Input ownership is OR-combined so one source cannot
  release a DOS key held by the other. Controller disconnection no longer blocks
  gameplay, because desktop input remains available.
- F1 opens keyboard/mouse help; F4/Back opens Actions. Q/E or LT/RT cycles Options
  tabs; PgUp/PgDn or LB/RB changes Actions pages. Left/Right or LB/RB switches guides.
  `keyboard-help.png/.rgb/.prompt.txt` is generated artwork with an E/R-row correction;
  only the two generated RGB guides are embedded. Original UI art remains runtime data.
- `assets.rs` imports a selected folder/executable or raw 2048-byte-sector ISO-9660.
  Directory extents/records and file sizes are bounded; only known asset names are
  copied, excluding saves and personal configuration. v1.01F CM_HI.EXE must be
  726898 bytes with FNV-1a-64 0x0A39EBE9A94993AC (version fingerprint, not ownership
  proof). Complete staging is validated before rename; existing asset directories
  are never overwritten. User cancellation exits, and headless missing-data errors
  do not invoke UI. Portable roots are executable-relative; only actual Cargo target
  builds default to the development project. DOS `..` paths are clamped inside VFS roots.
- `tools/package_release.py` uses an explicit release-file allowlist, a static Windows
  C runtime, MIT project license, dependency/stdlib notices and SHA-256 manifests.
  `tools/check_release.py` tests ZIP exclusion, default-root isolation, transactional
  malformed/version rejection, folder/ISO byte parity, fresh default boot, modern
  movement/look/stationary jump, both embedded guides, held/released hand mode,
  power/inventory commands, Escape cancellation and timestamped keyboard save/load.
  Original-data campaign coverage, real controller/audio and mouse/focus UX still
  require physical validation. No commercial assets or personal run files ship.

## Address conventions
- **Link addresses** (labels.csv, debug info): code object @ `0x10000`, data object @ `0x90000`.
- **Runtime linear** = link + `0x400000` (`le::rt()` / `le::link()`); keeps the first MB
  free for conventional memory / VGA window like a real extender.
- Labels come from the Watcom debug info embedded in the EXE (`tools/wdbg.py labels`):
  1718 globals (function + variable names, no locals/types/lines).

## Ph1: Binary layout
| Item | Value |
|---|---|
| LE header | file 0x28B8 |
| Object 1 (code) | link 0x10000, vsize 0x79368, pages 1..122 |
| Object 2 (data+bss+stack) | link 0x90000, vsize 0x17AB80, pages 123..137 |
| Entry | 0x6C404 (Watcom `cstart`, jumps to 0x6C47E) |
| Stack top | 0x20AB80 |
| Fixups | 10440, all type 7 (32-bit offset) |
| Debug info | Watcom v3, last 0x10B97 bytes of file |

Modules (by code size): cm.cpp (main loop, input, HUD), image1.cpp (raycast renderer),
cmfire/cmevent/cmmenu/cmcoll/cmload/cmshops/cmmove/cmdraw/cmselect, aui.cpp, video.cpp
(VESA), keyboard/mouse/joystick.cpp, cm_asm.ASM (ISRs, text, random), scale*.ASM
(column scalers), cmhmi40.cpp (Sounds class over HMI SOS), HMI SOS DIGI/MIDI libs,
Watcom CRT.

## Boot sequence (verified in emulator)
1. cstart: DOS/4G detected via `int 21h AX=FF00 DX=78h`; PSP/env selectors.
2. `main` (0x1F6DC): setenv DOS4G=QUIET, `LoadSetup` reads CMPATH.CNF
   (lines: cdPath, gamePath, spkPath, corePath), CMLANG.CNF, CMDATA.CNF.
3. `signature` prints banner; `IntroEpilogue` spawns `.\playtgv.exe` (intro movie).
4. `Sounds::BootSOS` (reads SOUNDSYS.CNF) — **replaced natively**.
5. `Xox::Init`, `Xox::StartClock`, `Aui::Animate` → title menu (`Xox::TitleMenu`).

## Timing
- `CM_ISR8` (0x67C06) is *not* an ISR: it is an HMI timer *event* callback at 40 Hz
  (`sosTIMERRegisterEvent(0x28, cm_sys_handler)`), ends with RET. Increments
  `_CMTimerTicks` (0x207473), polls joystick, moves the software pointer.
- `CM_ISR9` (0x67B7B) is the real keyboard ISR (int 9 via `_dos_setvect`), maintains
  `_kbScanStatus[128]` (0x207C54): 3 = down, 2 = was pressed (latched), 0 = up.
  Uses latch byte 0x97462 initialised to 0 → first key is swallowed unless a key
  release arrives first (we inject 0x9C at install).
- Watcom `delay()` busy-polls `int 21h AH=2Ch`.

## Video
- VESA 0x101 (640x480x8) via `int386(0x10)`: 4F02 set mode, 4F05 bank switch,
  4F07 display start (page flipping). Emulated with a 64 KiB window at 0xA0000 swapped
  into 4 MiB VRAM on bank switch.

## Sound (native replacement of `Sounds` class, cmhmi40.cpp)
| Method | Link | Args (Watcom regs) | Native |
|---|---|---|---|
| BootSOS | 0x6654C | this, noSound, path | sets this->on, registers CM_ISR8 @40Hz |
| CMsosPlaySample | 0x66C3C | this, slot, len, data; stack: chans, bits, vol(0..7FFF), loops(0/-1), pan(0..FFFF) ret 0x14 | mixer voice |
| SampleDone | 0x66D20 | this, slot → 1 if free | |
| SetSamplePanVol | 0x66D8C | this, slot, pan, vol | |
| CMsosPlaySong | 0x66DEC | this, song ptr in EDX (copied to ECX inside the original) | HMI player → Windows General MIDI or embedded Nuked-SC55 |
| MidiMasterVolume | 0x66E3C | this, volume (low 7 bits, 0..127) | music volume /127, not sample volume /32767 |
- Samples: 8-bit unsigned mono 11025 Hz; `soundadrs[]` (0x1E3D60) holds loaded data,
  lengths at Xox+0xE4F48[n], file offsets at Xox+0xE4B48[n], file handle Xox+0xE352C.

### Embedded SC-55 output
- `sc55.rs` / `sc55_bridge.cpp` wrap the vendored GPL-2.0-or-later Nuked-SC55
  0.8.0 backend (revision in `third_party/nuked-sc55/NOTICE`); compiled by `build.rs`.
  Backend source is unmodified; no Roland ROMs are included. `run/sc55/` is
  SHA-256 checked against upstream definitions, preferring MK1 then MK2.
- Options → Display → Music Output selects Windows MIDI / SC-55; stored as
  `music_output=windows|sc55` in NATIVE.CNF. Failure retains the old output and
  shows a menu warning. Successful switches silence the old synth and restart
  the current song to restore programs/controllers.
- SC-55 PCM is linearly resampled to the CPAL host rate and mixed with samples;
  MIDI sequencing and synthesis advance in <=64-frame chunks. Host pages keep
  synthesizing music/tails without advancing sample voices. HMI SysEx is still
  skipped; initialization uses the emulator's GM reset.
- Missing/unknown ROM rejection and settings round-trip tested; 56 unit tests
  pass (8 ignored). Release and static-runtime Windows GUI builds pass. Native
  menu screenshot confirms the selector; the full menu smoke script exceeded
  its 300-second budget after options-return. Audible ROM-backed validation
  remains pending (ignored `sc55_renders` check).
  Combined executable distribution is GPL-2.0-or-later; portable packager bundles
  corresponding source including the core, bridge/build scripts and Cargo deps.

### Music volume regression (latest-save investigation)
- The game passes 95/127 for the saved 75% music setting. The native hook used
  the digital sample denominator 32767, rounding MIDI channel volume to zero.
  `native/sound.rs` now matches the original `AND 0x7F` and seven-bit scaling;
  Windows MIDI output and the existing HMI sequencer are retained.
  `CM_TRACE_SONG=1` now also reports the applied master volume.
- Unit regression checks mute/default/max/masked values, stack return, and an HMI
  note plus CC7=75 at 75/127. Real title and loaded-level songs parse successfully.
  Audible output on the user's physical sound device still needs confirmation.

### Open doorway / small-step collision regression
- Reproduced using an isolated copy of `run/SAVE003.DAT`: camera starts
  (703872,1660544), heading 0x4550. Forward motion stopped at y=1679232 (~25.623),
  despite raised panels. Keeping every original Image1 function crosses it;
  isolated rotation/angle functions do not fix it. This is step-size dependent,
  not a renderer memory corruption or a reason to disable collision.
- `Xox::FixNxNy` 0x044090 tests the active opening at proposed centre coordinates
  before testing a masked FX door frame. Frame 313 has flags 0x48080000, centre
  (0xA8000,0x19E000), angle 0x4000; active region 7 is x=0xA4000..0xAC000,
  y=0x19B000..0x1A5000. The frame clips the player at y=0x19A000, 0x1000 before
  the aperture. Small movement steps cannot reach the exempt area; the original
  renderer's slower/larger steps can.
- Native hook at 0x044122 (displaced `MOV EAX,[ESP+0x40]`) extends only the depth
  of active axis-aligned masked FX openings by 0x1000, preserving the jamb width.
  Angle 0/0x8000 is normal X; 0x4000/0xC000 is normal Y. Solid panels, closed
  apertures, special platform flags and diagonals retain the original code.
  Success uses original cleanup 0x044A7C; fallback resumes 0x044126. No guest
  stack unwinding, teleports, save edits or broad noclip. Verifier/write logging
  retains unmodified DOS behavior.
- Unit regression covers both sides, orientation, jambs, solid panels, closed
  regions, diagonals, write logging and the original argument/register cleanup.
  `python tools/check_saved_door.py` copies the reported save to an isolated root,
  first reproduces the obstruction with this hook disabled, then crosses at
  640x480, partial-stick 1920x1080 and 5120x1440. All finish beyond y=27 tiles;
  SHA-256 checks confirm personal runtime files are unchanged.

## Renderer (image1.cpp) — TODO
Raycaster: RayCast/CheckTile/CheckAbove/CheckBelow → wall/flat/prop lists →
DrawWall/DrawFlat/DrawTopFlat/DrawMaskWall/DrawPackWall/DrawPackProp + `_scale`
column scalers. Hot spots (guest profile, in-game): DrawFlat 21%, _scale 11%,
DrawTopFlat 6%, DrawWall 4%.

## Death transition performance, nested Load, and controller wrapping
- Ported PCDeath's hot column-copy block (0x01551A..0x0154DA) to native Rust.
  Original RND calls, stack cursors, eight-row shifts, DOS pixels, song/Escape
  handling and page flips remain unchanged. Original-x86 comparison covers every
  cursor position and columns 0/317/639, including registers and framebuffer.
  Release column-copy benchmark: 10.05 ms interpreted vs 0.141 ms native (~71x).
- FileMenu's death-background call at 0x04FB13 was missing overlay recording.
  Share the existing MenuOptions recorder (0x04F042), returning past each call's
  JMP. The nested Load mask is now replaced every frame, including shade boxes,
  slot text and cursor; cancellation restores the death-menu mask.
- Controller arrows wrap in original title, pause/death, save/load, title-load
  and Options menus. Preserve interior grid steps; beyond the last choice goes
  to the first and before the first to the last. Death skips Resume/Save and
  Options skips the removed legacy input rows. Keyboard-only routing is unchanged.
- Checks: 52 regular Rust tests pass; isolated release death/melt/Load/cancel and
  original-x86 column equivalence tests pass (`cargo test --release -p cybermage
  death_ -- --ignored --nocapture --test-threads=1`). All five controller-only
  save/load/cancel smoke runs pass (`python tools/check_native.py --saves-only`),
  preserving unrelated saves/configuration. Screenshots: `out/death-check/`.

## Death sequence widescreen presentation
- Reproduced at 1080p: the first melt frame discarded the native world and
  switched to a pillarboxed 640x480 framebuffer. `Xox::PCDeath` (0x0153D0) has
  one caller, `HandleNextFrame` at 0x01BC4E, gated by `PCDeadMan` (0x20687C).
  It renders/flips the world for 60 timer ticks (flip return 0x01543F), then
  shifts random columns down 8 rows without any `RenderScene` calls (return
  0x01554A). Its 640 stack u16 cursors start at 7; total displacement is cursor-7.
- `display.rs` captures the final native world + square-pixel HUD at 0x015459,
  before the DOS shifts. Presentation mirrors the original cursors across the
  full output width; original guest randomness, song, timing and choices remain
  untouched. The tunnel underlay (Xox+0xE5920) uniformly covers the screen,
  cropping vertically rather than stretching or adding black side bars.
- Death menus use a virtual background draw at 0x04F042 instead of
  `Image1::Render`. Route its return through the existing overlay recorder:
  buttons, shade boxes and glove keep square-pixel proportions over the wide
  tunnel. Shade-only menus retain their writes; actual native-world menus still
  tint the world and omit those writes from the opaque overlay mask.
  Release the large frozen screen on entering the menu, and all death state on
  the first non-death presentation. Mode 0 keeps the original DOS effect.
- Checks: 34 unit tests pass, including column displacement/cover sampling at
  640x480, 1080p, 32:9 and 4K, plus snapshot cleanup. The isolated original-game
  regression reproduced the failure before the fix, then passed the original
  melt and death-menu transitions and verified all four buttons' overlay masks:
  `cargo test -p cybermage death_melt_keeps_widescreen_and_returns_to_native_menu -- --ignored --test-threads=1`.
  Screenshots: `out/death-check/`.

## Native floor / ceiling clipping
- Reproduced the reported moving/look-down hole in the starting room at 1080p
  and 4K; the equivalent 640×480 frame has a complete floor.
- `Image1::FindPoint` (0x064B4C) clips partly behind-camera floor/ceiling edges
  using `height / 479`. Its native height numerator scales with focal length,
  but this fixed DOS row limit can place the clipped edge inside the larger
  viewport. Native clipping now uses `viewHeight - 1 - horizon`; keep the
  original 479 divisor for DOS rendering/differential verification.
- Regression: `python tools/check_floor.py`, isolated configuration/saves and
  screenshots in `out/floor-check/`. The old executable fails with 100% black
  pixels in the near-floor test patch at 1080p; the fix has 0% at 640×480,
  1080p and 4K. All 13 Rust tests pass, including clipping at up/level/down
  pitches across five resolutions. 28,087 differential projection checks
  (1,290 FindPoint, 26,797 RWCPlot) have zero mismatches on the DOS path.

## Native display / controller support

- Gamepad-only revision: remove host minifb keyboard/mouse sampling, including
  movies and keyboard shortcuts. Keep internal DOS IRQ/pointer emulation as the
  transport for XInput; no physical device feeds it. Original Options rows 0–2
  are Input Device, DOS Joystick Calibration and Remap Keyboard Controls. Skip their
  drawing/selection with hooks at 0x053F08 and 0x0544EF; leave sound/speech/music,
  brightness/contrast, difficulty and text delay on the original settings paths.
  `HelpScreen` 0x051FE8 is called at 0x014051 in HandleKeyboardEvents. Replace it
  with a paused host guide. For its known return at 0x014056, resume at 0x0140D8:
  skip the old dismissal wait, retain keyboard cleanup and `bdec4=bdec0+1`
  redraw request. Other callers return normally; never unwind another frame.
  Embed the generated 4:3 Xbox 360 pixel-art bitmap; B dismisses the guide.
  Options browsing: LT/RT rising edges cycle Game/Display/Actions on every tab;
  LB/RB page through Actions only. Consume triggers without DOS mouse clicks
  while Options is visible; X selects under the native-page pointer instead.
  Held triggers do not repeat or advance when an Options page opens.
  Back-opened Actions has no suspended OptOptions call. Selecting Game there
  must request the real Options screen, not just clear host-overlay flags.
  Reuse the original menu flow: pending request sets Escape in HandleKeyboardEvents
  only outside a pause menu, then selects O at the pause/title event loops
  (0x04E82A / 0x054D97). Original Options entry consumes the request. No synthetic
  call into arbitrary interrupted guest code or stack unwinding is needed.
  Regression (isolated writable roots, shared read-only ISO):
  `cargo test -p cybermage back_actions_trigger_tabs_open_real_game_options_without_exiting -- --ignored --test-threads=1`.
  Covers Back-origin Actions -> Game via RT and Actions -> Display -> Game via LT,
  from both title and gameplay; verifies real Options entry, all three pages,
  remembered Actions subpage, unchanged camera and no quit. Screenshots:
  `out/native-check/back-trigger-game-options-{true,false}.bmp`. Passed together
  with all 20 regular Rust tests and five menu/quit smoke checks.

- `display.rs`: supported 4:3/16:9/16:10/21:9/32:9 modes up to 5120×2160;
  ultrawide choices include 2560×1080, 3440×1440, 3840×1080, 3840×1600,
  5120×1440 and 5120×2160. Guest framebuffer/ray allocation derives from the
  mode list rather than fixed 3840-pixel limits.
  `QuitToDosYN` (0x04E390) returns Yes immediately, skipping only its popup;
  its caller at 0x04F2BF and the normal shutdown path remain intact.
  On accepted Quit, retain the final composed RGB frame until window closure.
  The guest teardown still runs, but its DOS-only Aui/Shutdown page flips
  (0x057F7C) no longer replace the full-width native menu with black side bars;
  video-mode/palette resets and freed UI/scene data cannot change the snapshot.
  Regression: `quit_keeps_last_widescreen_frame_through_guest_shutdown` failed
  before the fix and now passes; `python tools/check_native.py --quit-only`
  verifies normal exit and nonblack left/right edges in the final 1280×720 image.
  Validation: all 54 regular Rust tests, real quit smoke check and release build.
  The Xox constructor initializes `ConSize` (0x1E3CF4) from ESI=2 at 0x01025B;
  that ESI value has no other use before being overwritten. Change the immediate
  to 0: `ConWinInfo[0]` is the full 640×480 view at offset (0,0); indices 1–4
  shrink it (index 4 is 240×180 at (200,150), not the largest). LoadScene and
  LoadGame both use this shared ConSize, so new/loaded games start maximized.
  Do not force the view each frame: original [/] sizing and vehicle insets remain.
  Settings in `run/NATIVE.CNF`, with `--resolution` override. `Xox::OptOptions`
  (0x053DDC) now runs the original Options screen from both title and pause menus.
  Its instruction panel gains Game / Display / Actions tabs; seven original
  non-input settings remain. LT/RT select main pages, LB/RB select Actions
  subpages. Back → Display and Controller Options opens Display directly.
  A return trap tracks the original menu
  lifetime; closing a host page resumes the suspended guest without popping its
  interrupted stack. Actions chosen here exit DOS menus before issuing gameplay
  input (disabled at the title screen). Track pause versus title Options using
  the guest return address (0x04F1E2 / 0x055692): both initialize player state 5.
  Send one Escape per menu phase, not repeating timed Escapes while a slow world
  frame restores presentation.
  Validation: 18 Rust tests, the opt-in Windows fullscreen desktop test, five
  isolated menu/quit checks and five floor checks pass. Menu checks include title
  Options, original/native artwork pixel equality, saved look speed/fullscreen and
  resumed controller movement after returning through both menus. Use
  `python tools/check_native.py --viewport-only` for the full startup viewport;
  `python tools/check_native.py --menus-only` for menus. Screenshots/logs are in
  `out/native-check/`.
- Manual save naming: `FileMenu` (0x04F3F4) receives EDX=0 for save, 1 for load;
  after slot acceptance and its normal sound delay, only save mode enters the name
  loop at 0x04FBAC. Hook that point, write local `YYYY-MM-DD HH:MM:SS` into the
  selected 24-byte `gameName` record (0x1E3297, ten manual slots), and resume at
  0x04FE20 with the original accepted-name/exit state. ESP locals: +0x6C is the
  one-based slot, +0x4C its zero-based text index, +0x48 mode, +0x50 exit request,
  +0x68 text cursor. Keep normal FileMenu rendering/cleanup and SaveGame serialization
  (0x05007C), including GAMENAME.USR. Both title and pause-menu loading already
  bypass text entry. Existing mouse/XInput slot navigation stays.
  SaveGame's other caller is the hidden restart slot 10, not a manual naming path.
  The Keys tab, virtual keyboard, menu-Y shortcut and keyboard Actions entry are
  removed; physical keyboard input and the inventory Y binding are unchanged.
  `python tools/check_native.py --saves-only`: five isolated controller-only runs
  pass (save, move, pause load, cancel save, title load), with zero physical key
  injections. Check local timestamp bounds, restored camera state, unchanged
  unrelated slot names/data and no save rewrites during loading/cancellation.
- Fullscreen is a saved Display toggle (`fullscreen=true/false`, default windowed),
  also available through Alt+Enter. It uses a borderless Windows popup fitted to
  the nearest monitor's full bounds, without switching display mode or changing
  rendering resolution. Reuse the same window factory on startup, resolution
  changes and fullscreen changes; reset input ownership/mouse deltas and the
  realtime timestamp on recreation. minifb must update/register its HWND state
  before SetWindowPos, otherwise the synchronous WM_SIZE leaves stale 640×480
  size reporting. Menu clicks map through both output/window letterbox layers.
  Runnable Windows desktop check: `cargo test -p cybermage
  fullscreen_window_fills_monitor_without_changing_render_resolution -- --ignored`.
- V-Sync is a saved Display toggle (`vsync=true/false`, default off). Windows
  minifb uses GDI: `GdiFlush` submits its drawing, then `DwmFlush` waits for
  compositor presentation in windowed/borderless modes. This replaces, rather
  than supplements, the frontend's 60 Hz sleep; the CPU slice retains its work
  budget and synchronization waits are excluded from adaptive-resolution timing.
  Movies share the wait but retain 15 FPS playback. Unavailable synchronization
  falls back to 60 FPS; non-Windows hosts show UNAVAILABLE. Display rows use
  28-pixel spacing for the extra toggle, shared with pointer hit testing.
  Validation: saved on/off/navigation regression, 54 Rust tests, release build,
  and windowed/fullscreen `cargo test -p cybermage
  vsync_waits_for_compositor_presentation -- --ignored`. The menu smoke suite
  hit the 240-second harness limit after producing both Options screenshots;
  the new Display layout was visually checked, but the full suite did not finish.
- Host pages reuse the original Options bitmap at `Xox+0xE541C`, palette 31 from
  `Xox+0xE5348` (6-bit RGB), and loaded font 3. `_font_table` (0x20732F), 32 bytes
  per font: bitmap pointer +0, height +20, signed spacing +26, glyph stride +30.
  Each glyph starts at bitmap+14+character*stride, with a 16-bit width then
  width*height bytes; FE/FF are transparent. Reopen the resource file read-only
  through VFS so menu artwork reads never disturb its guest DOS seek position.
  Native pages compose into the selected resolution with uniform 4:3 UI scaling;
  the built-in substitute font and separate blue panels have been removed.
- Selected primary view is `Xox+0xBDEBC`, not necessarily view 0 (vehicles).
  `RenderInit` supplies wider ray angles at the legacy sample count so the original
  visibility lists include the extra world. `RenderScene` runs a native-sized pass
  with independent pitch/focal length and a separately allocated framebuffer,
  restores renderer scratch `[+0x4B000,+0xE34E4)`, then renders the legacy view for
  unchanged targeting and DOS HUD code. Projection centre and focal length are
  distinct; vertical FOV is fixed as horizontal FOV increases.
- Native world placement (`display::world_rect`): only the full 640×480 view
  extends to the output edges. All other views are insets in 4:3 cockpit/backdrop
  art and map through the UI scale: `TankWinInfo` (0x095AB8) 480×256 at (80,32),
  `CarWinInfo` (0x095AD8) 640×276 at (0,16) (the canopy frames even this full-width
  view), `ConWinInfo[1..4]` shrunk views. Focal length and RenderInit ray angles
  derive from that rect, so vehicles keep the original horizontal FOV.
  Vehicle entry is `Xox::SwitchSearch`: `vehiclePERSON[scene+0x130CEC][4]`
  (0x0962BC) objects, distance < 0x7000 (0x01B3F6), height overlap
  (0x01B420/0x01B436), heading < 0x1800 (0x01B476); type 5 = tank, 0x11 = aircar
  (`+0x130D14=1`). Scene 9/11 entry 0 is an aircar; scene 1 entry 0 and scene 11 entry 1 are tanks.
  Debug: `CM_VEHICLE=N` passes those checks for entry N; `CM_SCENE` also sets
  `+0x130CEC/+0x130CF0` so scene tables match. `tools/check_vehicle.py` validates.
- Vehicle widescreen side art: `Xox+0x1444F0` is the loaded backscreen (LoadBackScreen
  0x01FBF0: 2 = tank, 3 = aircar; 320x240 indices in HIPAK.RSC at 0x5DE08 / 0x70A08,
  doubled; palette = resource 31 at 0x169970; index 0 = view window). `display::compose`
  fills the UI margins with 534-column generated extensions (`assets/*-sides.rgba`) only
  while the native world is presented, so menus keep black margins. The aircar extension
  has windshield pixels (alpha 0): its full-width view widens to the output edges
  (`view_widens`) and shows through them; the tank view stays inside its frame.
  Assets/prompts: `assets/vehicle-sides/README.txt`, `tools/prepare_vehicle_sides.py`.
- Host pages (Display/Actions/help) and an unfocused window pause the guest. The HMI
  sequencer is advanced by the emulation loop, so it must keep running on wall-clock time
  while paused (`Audio::advance_music`, frame dt capped at 0.1 s); otherwise its sounding
  notes never receive note-offs and hang on the GM synth. Sample voices stay paused.
- Pause menu (`Xox::MenuOptions`) re-renders view 0 every frame (Image1::Render at
  0x04F04B), then TankMonitor/ACarMonitor, `ShadeMenuBox` boxes, text, the glove cursor
  (DrawXTankAim) and NextPage. It sets the primary view `Xox+0xBDEBC` to -1 on entry
  (0x04E783) and to 0 on exit (0x04F3C1); `display::primary_view` maps -1 to view 0, so
  the native world (and vehicle side art) stays full-width behind the menu. Writes after
  the world render up to NextPage form the overlay mask, like the gameplay HUD: while
  view -1 is active, `Image1::Render` (0x065D34) gets a return trap that starts recording.
  `FileMenu` (save/load, 0x04F3F4) has the same render/boxes/text/glove/NextPage loop. `ShadeMenuBox` (0x04E2BC;
  this, x1, y1, x2, stack y2, shade; inclusive; shade>0 SHADE_MAPS[shade], else
  FIXED_MAPS[|shade|]) is ported (CM_VERIFY: 5961 calls, 0 mismatches) and also shades
  the native world, so boxes stay translucent at high resolution. Nothing else marks
  the menu as active, so `gamepad::gameplay` checks `display::pause_menu` (view -1).
  The original also leaves the aircar minimap black behind the menu.
- Options header (host tabs) blanks exactly the instruction panel interior
  x 232..623, y 18..121 of the Options bitmap; its left bevel starts at x=216.
- Capture actual framebuffer writes during `Xox::RenderBellsAndWhistles` (0x01899C)
  with the existing CPU write log. No colour-key/difference mask: black and equal-to-
  background HUD pixels remain opaque. Uniformly scale the HUD, hands and secondary
  camera insets. `TranArea` (0x05F8E0) tints both world buffers, but its legacy world
  writes are excluded from the HUD mask. Publish world/HUD together only after
  `Video::NextPage` (0x06B3F0) returns; menu page flips deactivate the native world.
- `gamepad.rs`: Windows XInput polling, radial rescaled deadzone, normalized diagonal
  movement. The keyboard handler returns through a host trap (never synchronously
  runs a guest help/menu loop). Controller movement sets `+0x130D10` (relative
  direction) and `+0x144B08` (speed); right-stick yaw uses `+0x144B0C`, pitch uses
  `+0xE33F8`. Movement/camera are independent. Action keys use the loaded key bindings.
  Keyboard/controller key ownership is merged, including disconnect/focus release.
  Digital buttons and triggers retain their gameplay/menu binding until release;
  host overlays suppress their inputs until release. Both windowed and scripted
  frontends use `Keys::controller`. A now requests a grounded jump directly using
  the original `ProcessInputEvents` setup (0x014D5E..0x014E63), without Ctrl's
  stationary hand-screen fallback. LT retains that contextual hand control and
  stops controller running before the guest processes it; X is Enter/SwitchSearch.
- Xbox 360 layout researched against both pages of Origin's original reference
  card (https://www.pixsoriginadventures.co.uk/downloads/, reference-card download).
  Confirmed Forward+Ctrl jump, F1 weapons/F2 powers/F3 inventory, aircar A/D/S gun
  aim, map zoom/size, view size, VDU, P pause, slash help, backslash speech skip,
  and Backspace/Enter/Esc text commands. Back opens a paged native Actions panel;
  selectors use original navigation/pointer routes; no virtual keyboard page remains.
- `ShopInputEvents` (0x048658) has only mouse/joystick paths, no keyboard path.
  Keep the DOS mouse route enabled in gameplay and menus, with or without an
  XInput pad; keyboard polling is already unconditional. `Xox+0xE3510=1` and
  `+0xE3518=0` select the mouse rather than the exclusive DOS joystick path;
  the joypad is handled independently by XInput. Do not change saved `inDevice`
  (0x1E38CC). Both frontends refresh routing and `MenuOptions` (0x04E690)
  enables it before its prologue caches the flag. Removed temporary routing
  restoration, which disabled the physical mouse again after menus/disconnect.
  Menu-only device reads at 0x04EDE4, 0x04F9D6, 0x04FBF9, 0x0553B8,
  0x055E57 and 0x0567A1 now use `mov eax,1` rather than reading saved `inDevice`.
  These are the pointer branches after unconditional keyboard handling (pause,
  save/load, title and title load); config reads/writes remain unchanged.
  Hook TitleMenu (0x054C20) before its initial cursor placement: Startup clears
  routing, so enabling only on the next host sample leaves the cursor on Credits
  and keyboard Enter can start the wrong command. Refresh routing on Options
  return too, before its guest continuation repositions the cursor.
  RT/LT provide menu left/right clicks, including secondary shop actions.
  Native jump/look are disabled in shops. Utility A/D commands use a short held-key pulse because
  the aircar code tests the down bit, not just the keypress latch.
  Earlier controller validation: 10 Rust tests and 12 isolated smoke scenarios pass. Stationary A
  raises camera z from 166911 to 172031 without changing x/y/yaw/pitch; LT with
  full-forward/run held leaves all camera values unchanged. Physical Xbox 360
  and complete shop/vehicle/campaign validation still remain.
- `CM_VERIFY` forces 640×480 and disables asynchronous host hooks. Regression checks:
  `cargo test -p cybermage`; `python tools/check_native.py` uses an isolated overlay
  and produces screenshots/logs in `out/native-check/`. Headless classic/1080p/
  ultrawide/4K, RT fire, diagonal movement, yaw/pitch and pause-menu checks passed;
  68,790 differential leaf checks (flat/top-flat/projection) had zero mismatches.
- Latest validation: 15 Rust tests, 23 isolated native smoke scenarios and five
  floor checks pass, including all new ultrawide modes, concurrent keyboard/pad
  movement/look, mouse polling with saved keyboard-only input, unchanged saved
  input preference, title/pause Options return and normal quit without a popup.
  Floor patches contain 0% black at 640×480, 1080p, 4K, 5120×1440 and 5120×2160.
- Still needs physical-controller / full campaign verification, including vehicle
  transitions, shops, save-name entry and sky-level rendering at wide aspect ratios.

## High-resolution performance
- Composition scales source-pixel runs instead of dividing per output pixel, skips
  the low-res world beneath the native viewport, clears only uncovered margins,
  and reuses the legacy RGB allocation. No resolution reduction or HUD stretching.
- Native floor/ceiling spans and wall/prop columns write directly to the host-owned
  guest framebuffer, bypassing per-pixel code-cache invalidation. Legacy rendering
  and verification still use CPU writes; memory wrapping/masking is unchanged.
- Realtime slices retain their last wall-clock timestamp, so presentation/sleep
  time advances guest timers too. Native options, focus loss and movies reset it.
- Same scripted level-1 boot, window-free benchmark: 1080p presents 44→62/s;
  4K 19→47/s, composition 37.35→8.35 ms. Rates vary with viewport/scene/hardware;
  these exclude window upload, and do not imply sustained 60 FPS at 4K.
- Validation: 7 Rust tests, 7 native smoke scenarios, 10,814 differential
  flat/top-flat/wall checks with zero mismatches; before/after 4K BMPs byte-identical.
  Reproduce with `CM_BENCH_SECONDS=5 cybermage --headless --seconds 14 --key 9:1c
  --resolution 3840x2160` (use an isolated `--root` to preserve personal saves).

## 60 FPS performance pass
- User prioritizes 60 FPS over rendering resolution. Frontend now targets 60 Hz,
  stops guest slices at completed `Video::NativePublish` frames, accounts for
  composition/upload time, and avoids catch-up bursts after stalls/movies. Windows
  timer resolution is paired with shutdown cleanup; sleep plus a final <=1 ms
  deadline wait avoids the default coarse sleep quantum.
- 3D rendering has an adaptive pixel budget (initially 1280x720 pixels, minimum
  640x360 pixels), preserving aspect/FOV, native output dimensions, original HUD,
  hands, menus, vehicle insets and targeting. Every 60 completed frames it reduces
  the budget for >12 ms average work or >=3 missed 16.67 ms budgets, and increases
  it only below 9 ms without misses. Incomplete-frame work is accumulated. Source
  rows are expanded once; identical world/UI source rows reuse the composed row.
  Tint/menu shading uses raster dimensions rather than output dimensions.
- Realtime guest sampling found OMapInsert 14.17%, OMapDelete 13.17%, and __STOSD
  7.42% of guest samples (instruction samples, not inclusive native CPU time).
  Native `Image1::OMapInsert` 0x0584C0 / `OMapDelete` 0x0581DC preserve the original
  four-corner order, signed radius clamp (0x3800 or 0x7400 for flag byte 1 bit 7),
  16-bit next/id pairs at pointer +0xD7240, reserved entry-0 free list, duplicate
  suppression, head tombstones, recycled nodes and silent pool exhaustion.
  Native `__STOSD` 0x077D97 preserves EAX end pointer and its ECX=1 three-dword-tail
  quirk; regular CPU writes retain code invalidation and differential logging.
- Validation so far: 33 regular Rust tests pass; 297,813 OMap differential calls
  have zero mismatches. A second sampled pass checked 4,489 calls, including 307
  __STOSD calls, also zero mismatches. Five floor regressions, the isolated saved
  doorway, five menu checks and all three opt-in desktop/original-data tests pass.
- Initial window-free native-resolution baseline: 1080p 65.81, 1440p 45.23,
  4K 27.83 game FPS. New adaptive/window-upload runs show 1080p 82–88 and 4K
  63–71 FPS capacity (uncapped, not a sustained-60 guarantee; raster varies).
  Frame-work percentiles and real Windows upload are now reported separately.
  Full-campaign/controller validation and scheduling/driver spikes remain outside
  this benchmark; output upload/guest simulation can still be limiting at 4K.
  Continuous-turn window checks: level 1 ~95 FPS at 1080p / ~65 at 4K; scene 9
  ~89 / ~83 (p95 work 11.91 / 14.27 ms). Full-native window-free comparison:
  1080p ~73 / 4K ~29; lowering only the 3D raster is essential to this 4K target.
- Reproduce: `python tools/check_performance.py --window --repeat 3` (isolated
  roots; screenshots/logs in `out/performance-check/`). `--turn` tests camera
  motion; `--scene N` selects a level. `--native-pixels 11059200` disables adaptive
  reduction for native-quality comparison. `--verify Func,Func --verify-every N`
  checks ports against DOS code; `--profile` writes guest sample profiles.

## Overnight optimization pass (2026-10-11, target 09:00 local)
- Baseline retained in `out/performance-check/until-9/baseline.exe`; all 34 regular
  tests passed. Initial continuous-turn window medians: 1080p 92.05 FPS / p95
  12.57 ms; 4K 82.30 FPS / p95 14.18 ms. Other concurrent trailer-rendering
  processes later caused heavy contention, so later timings need paired repeats.
- Guest samples: UpdateObjects 18.83%, MakeMissileList 12.65%, CollSwimFlat 10.41%,
  MoveObject 9.96%, FindTarget 7.62%, CollFlat 5.80%. Avoid porting the 2,400-line
  UpdateObjects state machine wholesale; target its pure leaf scans first.
- `simulation.rs`: MakeMissileList 0x02DBA8 scans active objects 1..<signed
  `+0x130D48`; skip flags byte 3 bits 0/1 and byte 1 bit 2; writes count/list
  `+0x136224/+0x136228`. CollSwimFlat 0x041C4C uses floor panels `+0xC408C`,
  32x32 tiles, 16-bit next/id pairs in `+0xE33E4`, inclusive flat bounds and
  height `<object_z+0x4800`; nearest raised flat texture overrides tile water.
  211,217 real DOS/native differential calls passed with zero mismatches.
- CollFlat 0x041B30 and CollTopFlat 0x041978 share those chains/bounds. Floor
  collision writes its tile floor first, then strictly higher flats below head;
  ceiling collision retains the player-specific body height, minimum 0x5800,
  original one-unit clearance and signed tests; paired checks passed.
- Added MoveObject 0x033ED0 (animation, bob/floor following), Turn 0x02AD6C,
  FindTarget 0x02AB30, SightPerson 0x02A9B0, TrackTarget 0x0344E0, PathCheck
  0x044EF4, and UpdatePCObject 0x034488. Original movement/sound callees remain.
  UpdateObjects 0x0348C4 ports inactive scans/common preparation, retaining guest
  AI states and exceptional water/damage/buff/corpse paths. Rotation-count bytes
  are at +0x83412, NOT the nearby target-mode table +0x81234.
- MoveMonster 0x047B90 ports setup and ordinary floor/platform following; both
  original boundary solvers and bob/knockback/ceiling paths remain guest code.
  Preserve 16-bit actor lookup, signed quarter-cell division, speed ±256 clamp,
  rider heading, negative level-pointer wraps and integer fall overflow faults.
  Dedicated every-call checks across scenes 1–11: 748,458, zero mismatches.
- Native RANDOM 0x068AD0/RND 0x06731C retain three-word generator and signed wraps;
  zero-bound RND does not advance it. Image1::TileSight 0x05D5A8 retains the
  original downward starting-floor-pointer quirk. RaySight 0x05D6D4 shares exact
  quadrant/stepping rules with RayCast; Image1::Render 0x065D34 retains widget,
  redraw and skip-blit ordering. Its `[vtable+8]` call (0x0660CE) is
  `Xox::RenderBellsAndWhistles(this, fb, view)`: EBX=view is left over from the
  OMapInsert setup and must be passed explicitly. Otherwise the caller's EBX
  leaks in, and paths with EBX≠0 (F1/F2 selectors, Ctrl/right-click hand screen)
  draw no HUD. Initial dedicated checks: RNG 58,382; TileSight
  44,739; rays 51,203; frame orchestration 89; all zero mismatches.
- DrawXScaleAim 0x029260 reuses the packed glow scaler without colour remapping.
  DrawHGFXBitMap 0x02A624 retains MOVSD overlap behavior, DF, opaque black and
  16-bit row-counter stack writes. TEXT_STR 0x0675D4/TEXT_LEN 0x067589 port original
  bitmap fonts, including FF/FE transparency/remap modes, 16-bit cursor wrap and
  their different signed-vs-unsigned character/stride/width rules. Initial
  dedicated checks: aim/glow 1,090; opaque HUD bitmap 1,591; fonts 59,700; zero
  mismatches. HUD/menu write-log allocations are reused without losing opacity
  or a caller's existing log.
- Scene-5's sound-handle discrepancy was verifier replay consuming host voices
  twice. Verification now rewinds sample voices between passes, compares them,
  and retains only the original pass's audio state; a synthetic allocation/gain
  regression checks both replay isolation and detection of audio-only errors.
  All valid scenes 1–11 passed cross-scene checks. Original scene 12 exits with
  DIMENSION ERROR IN SCENE 12 before verification; it is not a port mismatch.
- Composition caches UI spans/rows, uses exact 3:2/integral expansion and computes
  generic output-to-source columns once. Fullscreen native VGA conversion reads
  only opaque DOS HUD pixels; other layouts retain full conversion. Masked flat
  spans normalize rotation before their loops (bitwise complement and wrapping
  negative steps); opaque spans retain their faster original loop. Paired CPU
  span checks show roughly 2× faster masked rotations with unchanged pixels.
- Final 07:19 snapshot (including bitmap/fonts), alternating continuous-turn
  real-window runs, fixed 1280x720 world raster, three repeats: median 1080p
  74.73→124.68 FPS (+67%); 4K 62.92→86.07 (+37%). Median p50 work:
  13.29→8.02 / 15.81→11.46 ms; p95: 14.69→9.37 / 17.06→13.13 ms.
  These are capacity measurements, not full-campaign or stall-free FPS
  guarantees. Evidence and final-metrics.json: out/performance-check/until-9/.
  A final single run with default adaptive quality (not equal-raster benchmarking)
  delivered 93.29→101.79 FPS at 1080p with world 914x515→1624x913; at 4K,
  82.40→88.49 FPS with world 640x360→1144x643. Adaptive mode spends most of the
  improvement on roughly 3.2× more world pixels, not just higher frame rates.
- Regression coverage: native display/controller/menu checks, save/load/cancel,
  controller-guide pause/close, saved doorway (music enabled, personal files
  unchanged), and all five floor resolutions (0% black floor pixels). Final
  tests: 50 ordinary + 5 opt-in release tests passed, including fullscreen/cursor
  and native death melt. Final font/aim/bitmap checks across scenes 1–11: 297,654,
  zero mismatches. Tank and aircar entry worked with the original vehicle HUDs;
  final controller saves, guide and saved doorway checks passed. Final smoke
  coverage was split after the full command's timeout: the first batch passed
  viewport/classic/wide/ultrawide/4K-fire/stick/keyboard/standing cases; a resumed
  tail passed jump/hand/weapons/powers and all menu/artwork/options/shutdown checks.
  The complete single-batch suite also passed before the final bitmap/fonts.

## Data files
| File | Format |
|---|---|
| *.RSC | u32 maxrecs, u32 count, then offset table (see tools TODO) |
| HIPAK.RSC/FSC/GSC | full-screen art, offsets at `this+0xE5348`; entry 0 = raw 640x480 title (read by `LoadTitleScreen`/`TitleMenu`), 31 = 6-bit palette, 53 = Options background. FSC/GSC differ only in "Presented by"; the wordmark is identical. `rebrand.rs` re-sets it as "DARKLIGHT RE-AWAKENING" on DOS read. Entries 37/52/53/54 are the logo-headed menu screens (key commands, keyboard, options, personal log; English label rows 102..111 in font 0, `_font_table` at 0x20732F, 32-byte descriptors). Title menu boxes (`titleBoxY`) end at row 444 |
| PANELnn.DAT | level panels (TODO) |
| TEXT.MSG / SPEECH.ENG | message index + text / speech audio |
| *.TGV | EA TGV video (intro ECM/FCM/GCM, ending CMEND*, credits CRED*) |
| *.HMI | HMI-MIDISONG0615 |
| DRUM.BNK / MELODIC.BNK | AdLib instrument banks |

## Next Tasks
### RE Investigation
- [ ] RSC container format (all RSC files)
- [ ] Map/scene format (SCENE.RSC, PANELnn.DAT)
- [ ] Renderer data structures (Image1 object layout, wall/flat lists, textures)
- [ ] Xox object layout (fields used by HUD/input)
### Native ports
- [x] Native resolutions / Hor+ widescreen world rendering, square-pixel HUD and native options page
- [x] Windows XInput twin-stick controls, menu navigation, movie skipping and controller text entry
- [x] Integrate Display / Actions into original Options; reuse original menu artwork and font
- [x] Automatic local date/time save names; remove the virtual Keys page
- [x] Fix near-floor black gaps when looking down/moving at native resolutions
- [ ] Renderer (image1.cpp + scale.ASM) — performance critical
- [x] Restore HMI music volume through the existing Windows General MIDI output
- [x] Fix open doorway small-step collision; isolated latest-save regression
- [ ] Validate ROM-backed SC-55 playback, host-menu audio, switching and CPU cost (SC-55 backend builds on Windows; other platforms still need build/runtime checks)
- [ ] Non-Windows default MIDI device backend (embedded SC-55 now available in code)
- [ ] TGV movie player (replaces PLAYTGV.EXE)
### Verification
- [x] Differential test harness (guest vs native function, write-log diff) � `CM_VERIFY`
- [ ] DrawPackInvis / DrawPackAura not yet exercised by tests (need cloaked / aura enemies)
- [ ] DrawMaskFlat, DrawBackdrop not yet exercised (need masked flats / sky levels)

## Verification checkpoints
- [x] Boots to title menu, starts new game, renders level 1 (headless screenshots)
- [x] Portable Windows release: folder/raw-ISO parity, exclusions, hashes, failed-import
  cleanup, no-overwrite, fresh boot, modern input/guides and keyboard save/load.
- [x] 25 standard Rust tests plus all 3 opt-in Windows/original-data tests passed.
- [x] Focused controller guide, save/load/cancel/title-load and menu/resume/quit checks.
- [x] Floor checks: 0% black at all 5 tested resolutions; saved doorway crosses at
  full/partial stick while the unhooked original reproduces the blockage.
- [ ] Complete native smoke sweep: timed out after viewport/widescreen/4K stages;
  focused controller/menu/save runs subsequently passed.
- [ ] Ph6: full game session played
