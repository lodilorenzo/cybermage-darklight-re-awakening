# CyberMage: Darklight Re-Awakening (Rust rewrite) — working notes

- Original game: `iso/CYBERMAG/` (extracted from `CMV1.01F.iso`, read-only).
- Writable game dir (config, saves): `run/` (overlay over iso/CYBERMAG).
- Build: `cargo build --release`; run windowed: `target/release/cybermage.exe`.
- Headless test: `cybermage --headless --seconds N --shots S --key T:SC` (screenshots in `out/`).
  Debug env: `CM_WATCH=addr,..` (write watch), `CM_COUNT=addr` (entry counter),
  `CM_PROFILE=1` (guest EIP profile → `out/profile.txt`, `python tools/profile.py`),
  `CM_TRACE_IRQ=1`, `CM_TRACE_INTS=1`, `CM_SCENE=N` (new-game scene),
  `CM_VEHICLE=N` (Use enters vehicle entry N anywhere; `tools/check_vehicle.py`).
- Addresses: link addresses everywhere (labels.csv); runtime = link + 0x400000.
- Tools (python, `tools/`): `wdbg.py` (debug info), `le.py` (loader), `dasm.py ADDR|LABEL`
  (`-f` whole function), `xref.py`, `profile.py`, `hotspots.py`, `bmp2png.py`, `strip.py`.
- Native replacement: `machine.hook(cpu, link_addr, name, fn)`; Watcom regcall args
  EAX, EDX, EBX, ECX then stack; native must `cpu.ret_near(stack_bytes)`.
- See REVERSE.md for findings and the task list; dead_ends.md for failed approaches.
