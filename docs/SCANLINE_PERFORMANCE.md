# Scanline performance investigation

**Archived: scanlines were removed from the build at the user's request.**
The filter, setting, F6 toggle and comparison tools no longer exist. Measurements
and commands below describe older builds, not the current executable.

**Historical measurements below use synchronous SC-55.** The subsequent
[SC-55 worker optimization](SC55_PERFORMANCE.md) brings its measured game FPS
close to Windows MIDI while preserving the emulator; it also records remaining
audio-validation limitations.

Measured on 2026-10-14: Windows, i7-7700K, RTX 2060, 3440×1440 desktop at 120 Hz.
Selected game output: **2560×1080**, borderless fullscreen (minifb/GDI scales to the
monitor). Opening scene, continuous camera turn, three alternating off/on pairs,
six seconds per state after two-second warmup. World budget fixed to 921600
pixels (actual raster **1478×623**) so adaptive quality cannot hide filter cost.
These are short diagnostic benchmarks, not a whole-campaign 60 FPS guarantee.
User saves, ROMs and `run/NATIVE.CNF` were not changed.

## Frame capacity: before and after optimization

No V-Sync or audio device; includes real fullscreen upload. Numbers are medians
of three runs, not the production frontend's capped FPS.

| Build / scanlines | Uncapped FPS | Composition + filter (ms) | Frame-work p95 (ms) | Frames with >16.67 ms work |
|---|---:|---:|---:|---:|
| Before / off | 98.0 | 3.13 | 12.96 | 0.7% |
| Before / on | 71.7 | 6.75 | 17.49 | 8.8% |
| Optimized / off | 103.7 | 2.95 | 11.87 | 0.2% |
| Optimized / on | 92.1 | 4.27 | 12.36 | 0.5% |

The original full-image scalar loop added about **3.6 ms** at this resolution.
The optimized loop adds about **1.3 ms** in the stable repeat: roughly 60% less
filter overhead. On/off runs happen within the same boot, with order reversed
on alternate pairs. They do not replay identical guest frames; small scene and
host scheduling differences remain.

A first optimized capacity run was unstable: off **53–78 FPS**, on **36–55 FPS**,
with increased time in *all* stages, not just scanlines. Those logs are retained
in `out/performance-check/scanlines/after-capacity/`; the stable repeat is in
`after-capacity-repeat/`. Do not interpret the disturbed run as scanlines alone,
or the repeat as proof that all intermittent stalls are fixed.

### Kernel-only check

Nine alternating batches, 64 frames each, using Windows thread CPU time to
exclude descheduling. Identical buffer-reset cost subtracted from both states.
Approximate per-frame CPU cost:

| Output | Original filter | Optimized filter |
|---|---:|---:|
| 2560×1080 | 3.66 ms | 1.71 ms |
| 3840×2160 | 10.01 ms | 4.64 ms |

Optimization: baseline x86-64 SSE2 shades four pixels together using widened
16-bit channels and an 8-bit gain. Fully bright rows are skipped. Other
architectures and partial vector tails retain a scalar path. No extra threads,
dependencies, graphics backend or change to the sine beam/scale gate. RGB differs
from the original shader by at most one step; alpha stays unchanged.

## Actual saved audio choice is the larger bottleneck

The user's saved settings were fullscreen, V-Sync, **SC-55**, Light scanlines.
Repeated the optimized on/off comparison with those settings and a **real
48 kHz audio device**. Then repeated with Windows MIDI, leaving everything
else the same. Medians of three paced runs:

| Music output | Scanlines off FPS | Scanlines on FPS | Off p95 work | On p95 work |
|---|---:|---:|---:|---:|
| Windows MIDI | 56.9 | 56.3 | 13.61 ms | 14.97 ms |
| SC-55 | 31.7 | 29.5 | 32.03 ms | 35.52 ms |

The emulation/audio stage averages about **19–20 ms/present with SC-55**, versus
**5–6 ms with Windows MIDI**. Every measured SC-55 frame exceeds the 16.67 ms
work budget, even with scanlines off. Thus **SC-55 is the main measured cause of
choppiness in this saved configuration**; optimizing the filter cannot fix that.
V-Sync/compositor waits add about 6–8 ms and can introduce refresh-slot misses.
The audio checks measure CPU/frame timing, not device-underrun counts or audio
quality. Busy combat, other scenes, and production adaptive-quality behavior
were not certified.

For the immediate performance workaround, select **Enhancements → Music Output
→ Windows MIDI**. The user's choice was deliberately not changed automatically.
Moving SC-55 synthesis off the emulation thread would be a separate timing/audio
change requiring synchronization and underrun validation; it was not implemented
by this scanline patch. It is now implemented by the subsequent worker patch
linked above; the historical table does not describe current SC-55 performance.

## Menu fix and validation

- Display tab/page/Actions entry renamed **Enhancements**.
- Enhancements and Actions use the same output beam as gameplay, exactly once.
  Guides remain unfiltered; stored death/quit snapshots stay clean.
- Release build and **58 Rust tests pass** (8 ignored).
- `python tools/check_native.py --scanlines-only` passes real-game beam-reference,
  held-F6 edge/persistence, unchanged guest-camera and exact off-restoration checks.
- `out/native-check/scanlines-enhancements.bmp` visually checked: renamed tabs fit
  and the effect covers the new menu, including its header.

## Reproduce

```sh
cargo build --release
python tools/bench_scanlines.py out/performance-check/scanlines/display-before.rs
python tools/check_performance.py --window --fullscreen --modes 2560x1080 --scanlines-pair --repeat 3 --seconds 6 --turn --label scanlines/capacity
python tools/check_performance.py --window --fullscreen --vsync --audio --music-output windows --scanline-strength 0.25 --modes 2560x1080 --scanlines-pair --repeat 3 --seconds 6 --turn --label scanlines/midi-vsync
python tools/check_performance.py --window --fullscreen --vsync --music-output sc55 --scanline-strength 0.25 --modes 2560x1080 --scanlines-pair --repeat 3 --seconds 6 --turn --label scanlines/sc55-vsync
```

The before-source snapshot is a local artifact, not required to benchmark the
current kernel (omit that positional argument). SC-55 uses the user's lawful
ROMs in `run/sc55/`, read through an isolated directory link; none are shipped.
Full measurements are in `out/performance-check/scanlines/`, particularly
`before-capacity`, `after-capacity-repeat`, `after-midi-vsync`, `after-sc55-vsync`,
`kernel-after.log`, and `smoke.log`.
