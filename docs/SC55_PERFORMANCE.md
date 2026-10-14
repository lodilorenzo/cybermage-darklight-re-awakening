# SC-55 worker and native optimization

Scanlines and their benchmark switches have since been removed; the comparison
commands below are historical and require an older build/tooling.

These are the original worker measurements. Subsequent [Vulkan work](VULKAN.md)
restores the existing 40 ms audio lead after window/device setup. Its paired
software intervals have zero underruns, while experimental Vulkan still records
7/1/4; consult that report for current presentation/audio limitations.

2026-10-14; Windows i7-7700K / RTX 2060, 3440×1440/120 Hz desktop, borderless
2560×1080 output, fixed 1478×623 world, continuous turn in the opening scene.
Real 48 kHz audio and V-Sync; three alternating scanline off/on pairs.

## Result

| Music/backend | Scanlines off | Light scanlines on |
|---|---:|---:|
| Previous synchronous SC-55 (6 s/state) | 31.7 FPS | 29.5 FPS |
| Worker + native LTCG (10 s/state) | 56.6 FPS | 57.1 FPS |
| Windows MIDI, same new build (10 s/state) | 58.6 FPS | 58.0 FPS |

Numbers are medians of three runs. The game-thread emulation/audio stage drops
from ~19–20 ms/present to ~5 ms. SC-55 still uses an additional CPU core: this
recovers game-thread time, not Windows MIDI's total CPU cost. Host/GDI timing
varied between runs; neither backend is certified at 60 FPS throughout gameplay.
A separate 20 s/state worker run measured medians 55.9 off / 56.8 on.

**Audio limitation remains visible:** strict `--check-audio` fails in the first
measured interval: 3 underrun callbacks for SC-55, 4 for Windows MIDI. The next
five intervals have zero underruns for each backend. The 20-second run likewise
had 3 in its first interval and zero in the other five. No completed PCM was
clipped, and SC-55 remained active in all final measurements. These are similar
initial-run faults, not proof of zero-glitch playback. The assertion is retained;
no bigger latency buffer or relaxed threshold was used to pass it.

## Implementation

- `sc55_worker.rs` owns the FFI synthesizer entirely on one dedicated thread;
  there is no unsafe `Send` implementation and no synthesis in CPAL's callback.
- The game still advances sample voices deterministically. A queued job carries
  mixed effects, sample rate, and ordered MIDI messages at their existing 64-host-
  frame offsets. The worker synthesizes/clamps PCM and publishes it to the same
  output ring. Music continues on host menus; effects remain paused there.
- Pending work plus device-ready PCM stays within the existing **150 ms** ceiling.
  Completed stale PCM is trimmed as before, never pending MIDI. A **40 ms initial
  silence lead** accommodates worker/device scheduling inside that ceiling.
- Windows MMCSS `Audio` scheduling is scoped to the worker and reverted on exit.
- The MSVC C++ backend uses `/GL` and `/LTCG`, allowing the firmware/PCM hot path
  to inline across translation units. No fast-math, ROM patch, reduced chip clock,
  lower sample rate, voice reduction, or backend-source modification.
- Opening a replacement succeeds before altering the old song/output. Switching
  or shutdown cancels pending work, wakes and joins the old worker before clearing
  the ring, so it cannot write stale samples after the switch.
- A pending synthesis backlog exceeding 150 ms explicitly disables music rather
  than blocking gameplay or silently losing MIDI. Timestamped resynchronization
  after a very long host stall is a follow-up, not implemented here.

The worker-only first attempts without LTCG were insufficient: queue-budget
failure disabled the synth. Those FPS numbers are invalid as SC-55 results and
retained in the `diagnostic`, `retimed`, and `mmcss` logs. The benchmark now
asserts that the requested synthesizer remains active, preventing that false pass.

## Validation

- Release build; **58 standard tests pass** (9 ignored), plus the two explicitly
  run ROM-backed checks: worker PCM/switching and audible-note rendering.
- ROM-backed worker regression passes: **bit-identical PCM** against synchronous
  synthesis at 44.1/48 kHz, with timed program changes, note-on/off, all-sound-off,
  and mixed effects. Also tests bounded overload, no writes after shutdown,
  failed replacements, paused effects, and SC-55 → Windows MIDI → SC-55 switching.
- Final 60-second and 120-second on/off measurement windows keep synthesis active,
  with zero stale-PCM clipping. Strict zero-underrun validation remains **failed**
  due to the initial-run callbacks described above, for both music backends.
- Only the Windows/MSVC build and supplied SC-55 ROM set were measured; other
  platforms, other ROM revisions, movie/focus transitions, and busy combat are
  not comprehensively validated. User saves, ROMs and settings were unchanged.

```sh
cargo build --release
CM_SC55_ROMS=path/to/your/roms cargo test -p cybermage --release worker_matches_synchronous -- --ignored --nocapture
python tools/check_performance.py --window --fullscreen --vsync --music-output sc55 --scanline-strength 0.25 --modes 2560x1080 --scanlines-pair --repeat 3 --seconds 10 --turn --check-audio --label sc55-worker/sc55
python tools/check_performance.py --window --fullscreen --vsync --audio --music-output windows --scanline-strength 0.25 --modes 2560x1080 --scanlines-pair --repeat 3 --seconds 10 --turn --check-audio --label sc55-worker/midi
```

The last two commands intentionally fail on any underrun or PCM clipping; omit
`--check-audio` only for measurement, not to claim audio stability. Detailed logs:
`out/performance-check/sc55-worker/{confirmed-sc55,confirmed-midi,long-sc55}/`,
`tests-final.log`, `pcm-final.log`, and `audible-final.log`. A parser bug that counted the `benchmark
audio:` metadata as an FPS result was fixed; numeric FPS lines are now matched
specifically.
