# Vulkan presentation backend (experimental)

Enable **Enhancements → Renderer → Vulkan**, then leave Enhancements to apply.
Alternatively:

```sh
cargo build --release
target/release/cybermage.exe --renderer vulkan
```

`--renderer software` forces the existing minifb/GDI path. Saved choice:
`renderer=vulkan|software` in `run/NATIVE.CNF`. Old configs default to software;
no existing user settings or saves were changed by these tests.

## Scope and requirements

This is a **native Vulkan presentation/scaling backend**, not yet a GPU port of
CyberMage's world renderer. Guest emulation, native walls/floors/sprites, palette
expansion and HUD composition remain on the CPU. Their finished BGRA8
framebuffer is uploaded to an optimal-tiled image and nearest-blitted into the
swapchain, with centered aspect-preserving black margins. Gameplay, host menus,
guides and movie frames use the same window-owned presenter. CPU screenshots
and frozen death/quit snapshots remain unchanged.

The initial implementation requires **Windows**, a Vulkan loader/driver, Vulkan
1.1, BGRA8 UNORM transfer/blit presentation, and modern presentation extensions:
`VK_EXT_surface_maintenance1`, `VK_EXT_swapchain_maintenance1`,
`VK_KHR_present_id` and `VK_KHR_present_wait`, plus their dependencies. Tested on
an RTX 2060 with Vulkan 1.4.351. Unsupported devices/platforms or runtime errors
fall back to software with a diagnostic and menu warning. No Vulkan SDK, shader
compiler, new system DLL bundle, graphics framework or driver setting is required
to build/run. The only new direct dependency is `ash` (MIT OR Apache-2.0).

Implementation: `crates/game/src/{vulkan,presentation}.rs`.
- Mapped coherent upload memory and the source image are reused until size changes.
- One GPU submission is in flight; command/staging reuse waits for its fence.
- Present semaphores are indexed by swapchain image, not by frame number.
- maintenance1 fences retire presentation-engine references on resize/shutdown;
  queue-submit completion alone is insufficient.
- FIFO + present-ID wait replaces the software DWM wait for V-Sync. Off selects
  IMMEDIATE, then MAILBOX, then FIFO if those are unavailable. No second DWM wait.
- GPU objects/surface are destroyed before the owning HWND. Backend changes
  recreate the window, avoiding minifb retaining an obsolete GDI buffer pointer.
- Transfer/blit execution is measured using hardware timestamp queries; CPU
  upload time and resource/acquire/presentation waits are reported separately.
  Adaptive quality reserves estimated GPU service time as well as CPU work.
- Driver first-use work is prewarmed before resuming the guest/audio realtime
  clock. Host window setup is a pause, not hundreds of milliseconds of audio
  catch-up. The existing SC-55 **40 ms** scheduling lead is restored after that
  pause without clearing PCM/MIDI or enlarging the **150 ms** ceiling.

## Measured result

These prototype measurements predate removal of the scanline effect; current
builds and benchmark commands below no longer include it.

Windows i7-7700K / RTX 2060, 3440×1440/120 Hz desktop, borderless 2560×1080
logical output, fixed 1478×623 world. Continuous opening-scene turning, Light
scanlines, real 48 kHz SC-55 and V-Sync. Three alternating software/Vulkan pairs
in **one boot**, 10 seconds measured per state after 2 seconds warmup:

| Median | Software | Vulkan |
|---|---:|---:|
| Completed game frames/s | 56.89 | 67.82 (**+19%**) |
| CPU-side upload/present work | 1.382 ms | 0.870 ms |
| CPU frame-work p50 | 11.35 ms | 10.63 ms |
| GPU transfer/blit p50 | not measured | 1.197 ms |

These are short capacity/presentation comparisons, not full-campaign FPS or
perfect refresh pacing guarantees. CPU frame-work excludes waits; GPU service
can overlap CPU work. Do not interpret CPU upload timing as GPU execution time.
World quality, SC-55 emulation and scanline strength were held fixed; the synth
remained active in every final interval.

**Audio stability is not certified:** strict `--check-audio` fails. Software had
0/0/0 underrun callbacks; Vulkan had **7/1/4**, with zero stale PCM clipping for
both. Keep software if you hear glitches. The strict check is retained, not
relaxed, and the latency buffer was not increased to hide them. Earlier paired
logs and a first failed Vulkan benchmark are retained: first-use driver work was
incorrectly counted as guest/audio catch-up and disabled music. That FPS result
is invalid; prewarming and clock handling fixed the disable, not the remaining
underruns. Logs: `out/performance-check/vulkan/confirmed-light/` and
`out/vulkan-check/`.

## Validation and reproduction

- **60 ordinary release tests pass**, 10 ignored.
- ROM-free GPU regression passes: bit-exact RGB at 1:1, colors/orientation,
  nearest scaling, black margins, source replacement, resize, V-Sync switching,
  and destruction, with Khronos **synchronization validation** and zero API errors.
- Actual fullscreen game presentation with scanlines also passes Khronos
  synchronization validation, including teardown. Third-party Galaxy overlay
  layer naming warnings are present; there are no application validation errors.
- ROM-backed SC-55 PCM/effects/timed-MIDI/switching regression still passes,
  including idempotent scheduling-lead restoration preserving completed PCM.
- Other GPUs, HDR/rotated displays, long focus/minimize transitions, movie playback
  and busy combat need broader validation. This remains an opt-in prototype.

```sh
python tools/check_vulkan.py
# With an already available Khronos layer:
python tools/check_vulkan.py --validation --game
# Paired performance; --check-audio intentionally fails on the current underruns:
python tools/check_performance.py --window --fullscreen --vsync --renderer-pair --music-output sc55 --modes 2560x1080 --repeat 3 --seconds 10 --turn --check-audio --label vulkan/paired
```

This agent's elevated Windows process ignored `VK_LAYER_PATH`. With explicit
user permission, `check_vulkan.py --validation --game --register-validation
PATH/TO/VkLayer_khronos_validation.json` temporarily registered a locally extracted
official LunarG layer and restored the original registry value/key in `finally`.
No SDK/driver was installed. Restoration is recorded in
`out/vulkan-check/validation-registry-backup.json` and independently checked.
Normal builds do not register or enable validation. Never use the registration
option without permission; force-killing the test harness can prevent cleanup.

## Next optimization

Upload the smaller indexed world/UI layers and compose them in Vulkan shaders
instead of uploading an ~11 MB fully composed image each frame. That could
remove much of the CPU composition stage and cut transfer traffic. It is not implemented or benchmarked by this first backend.

References: [present-semaphore reuse](https://docs.vulkan.org/guide/latest/swapchain_semaphore_reuse.html)
and [presentation retirement fences](https://docs.vulkan.org/refpages/latest/refpages/source/VkSwapchainPresentFenceInfoKHR.html).
