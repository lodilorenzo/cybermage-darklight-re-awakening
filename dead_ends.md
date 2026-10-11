# Dead ends

## Interpreter speed via decoded-instruction cache (session 1)
- Tried: per-byte-indexed `Vec<Op>` cache (24-byte ops over the 496 KiB code object).
- Result: no measurable gain in-game (~50 MIPS before and after); xperf shows the
  cache loads miss (12 MiB footprint) and the remaining cost is spread over dispatch.
- Also tried: unchecked memory accessors (no gain), inlining `step` into `run` (+40%
  in micro-benchmarks only).
- Better approach: port hot guest code (renderer) to native Rust; a JIT would be the
  only big interpreter win and is out of scope.

## Overnight renderer micro-optimizations (2026-10-11)
- Explicitly unrolling native wall columns four pixels at a time gave no reliable
  paired gain (1.4–1.7 ms per 1280x720 wall); reverted. Compiler unrolling is enough.
- Expanding world rows directly into output instead of caching the RGB row passed
  pixel equality, but repeated UI/world source combinations increased expansion
  work: 4K/1280x720 composition regressed 4.53→5.31 ms CPU; reverted.
- Copying only world gaps not covered by opaque UI passed equality but regressed
  generic adaptive scaling by 12–25%, even with a separate non-inlined copy helper.
  Fixed-ratio improvements were marginal; retained contiguous row copies instead.
- Evidence and paired benchmarks: `out/performance-check/until-9/`; use
  `tools/bench_compose.py` and `tools/bench_columns.py` before retrying these.
