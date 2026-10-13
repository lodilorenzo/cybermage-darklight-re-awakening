//! CyberMage: Darklight Re-Awakening — Rust reimplementation host for
//! CyberMage: Darklight Awakening (Origin, 1995).
//!
//! The original CM_HI.EXE code runs on an x86 interpreter with all DOS,
//! DPMI, BIOS and hardware services emulated natively.  Game functions are
//! progressively replaced by native Rust implementations (see `native/`).

// Packaging opts into GUI startup; diagnostic builds retain their console.
#![cfg_attr(all(windows, feature = "windows-gui"), windows_subsystem = "windows")]

mod audio;
mod assets;
mod desktop;
mod display;
mod gamepad;
mod frontend;
mod tgv;
mod verify;
mod input;
mod le;
mod music;
mod native;
mod machine;
mod rebrand;
mod midi_out;
mod sc55;
mod vfs;
mod vga;

use machine::Machine;
use std::path::PathBuf;
use x86::{Cpu, Fault};

pub struct Config {
    pub root: PathBuf,
    pub headless: bool,
    pub seconds: f64,
    pub ips: f64,
    pub screenshot_every: f64,
    pub keys: Vec<(f64, u8)>,
    /// Scripted key holds: (press time, release time, scancode, extended)
    pub holds: Vec<(f64, f64, u8, bool)>,
    pub trace: bool,
    pub realtime: bool,
    pub resolution: Option<usize>,
    /// Synthetic controller holds for reproducible headless input checks.
    pub pads: Vec<(f64, f64, gamepad::Pad)>,
    pub desktop: Vec<(f64,f64,desktop::Snapshot)>,
}

pub struct Emu {
    pub cpu: Cpu,
    pub m: Machine,
    pub ips: f64,
    pub cfg_trace: bool,
    /// Debug write-watches: (runtime address, last byte value)
    pub watches: Vec<(u32, u8)>,
    /// Guest EIP sampling profile (link address -> samples)
    pub profile: Option<std::collections::HashMap<u32, u64>>,
    /// Debug entry counter (runtime address, count)
    pub counter: Option<(u32, u64)>,
    pub profile_start: f64,
    /// Debug: override the scene number of a new game (CM_SCENE=N)
    pub scene_override: Option<u32>,
    /// Debug: Use enters scene vehicle entry N from any distance/angle (CM_VEHICLE=N)
    pub vehicle_anywhere: Option<u32>,
    /// Includes time spent presenting/sleeping between realtime slices.
    pub realtime_last: Option<std::time::Instant>,
}

impl Emu {
    pub fn new(cfg: &Config) -> Emu {
        let game = cfg.root.join("iso").join("CYBERMAG");
        let overlay = cfg.root.join("run");
        let vfs = vfs::Vfs::new(game.clone(), overlay.clone());
        write_default_config(&overlay);
        let mut cpu = Cpu::new();
        if cfg.trace {
            cpu.enable_trace(64);
        }
        let mut m = Machine::new(vfs);
        if let Some(mode) = cfg.resolution { m.display.mode = mode; }
        if m.verifier.is_some() { m.display.mode = 0; }
        let exe = std::fs::read(game.join("CM_HI.EXE")).expect("CM_HI.EXE not found");
        m.load_program(&mut cpu, &exe, "").expect("load");
        native::install(&mut m, &mut cpu);
        let watches = std::env::var("CM_WATCH")
            .map(|v| {
                v.split(',')
                    .filter_map(|a| u32::from_str_radix(a.trim_start_matches("0x"), 16).ok())
                    .map(|a| (le::rt(a), 0u8))
                    .collect()
            })
            .unwrap_or_default();
        let profile = if std::env::var("CM_PROFILE").is_ok() { Some(Default::default()) } else { None };
        let counter = std::env::var("CM_COUNT").ok().and_then(|v| u32::from_str_radix(&v, 16).ok()).map(|a| (le::rt(a), 0));
        let profile_start = std::env::var("CM_PROFILE_START").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let scene_override = std::env::var("CM_SCENE").ok().and_then(|v| v.parse().ok());
        let vehicle_anywhere = std::env::var("CM_VEHICLE").ok().and_then(|v| v.parse().ok());
        Emu { cpu, m, ips: cfg.ips, cfg_trace: cfg.trace, watches, profile, counter, profile_start, scene_override, vehicle_anywhere, realtime_last: None }
    }

    fn debug_overrides(&mut self) {
        if let Some(sc) = self.scene_override {
            if self.cpu.eip == le::rt(0x11074) {
                self.cpu.r[x86::EDX] = sc;
                // Current/next scene, as the original stores just before.
                let this = self.cpu.r[x86::EBP];
                self.cpu.wr32(this + 0x130CEC, sc);
                self.cpu.wr32(this + 0x130CF0, sc);
            }
        }
        // Pass SwitchSearch's vehicle distance, height-overlap and heading checks.
        if let Some(entry) = self.vehicle_anywhere {
            let r = &mut self.cpu.r;
            match le::link(self.cpu.eip) {
                // EBP is the vehiclePERSON entry offset.
                0x1B3F6 => r[x86::EAX] = if r[x86::EBP] == entry * 4 { 0 } else { 0x7000 },
                0x1B476 => r[x86::EAX] = 0,
                0x1B420 => r[x86::EBX] = r[x86::ECX].wrapping_add(1),
                0x1B436 => r[x86::EDX] = r[x86::EBX].wrapping_add(1),
                _ => {}
            }
        }
    }

    /// Run emulation for `dt` seconds of emulated time.
    pub fn run_for(&mut self, dt: f64) -> Result<(), Fault> {
        self.realtime_last = None;
        let total = (dt * self.ips) as u64;
        let batch = 2000u64;
        let mut done = 0u64;
        while done < total {
            let n = batch.min(total - done);
            let start_ic = self.cpu.icount;
            let mut res = Ok(());
            for _ in 0..n {
                if let Err(e) = self.cpu.step(&mut self.m) {
                    res = Err(e);
                    break;
                }
                if !self.watches.is_empty() {
                    self.check_watches();
                }
                self.debug_overrides();
                if let Some(c) = &mut self.counter {
                    if self.cpu.eip == c.0 {
                        c.1 += 1;
                    }
                }
            }
            if let Some(p) = &mut self.profile {
                if self.m.time >= self.profile_start {
                    *p.entry(le::link(self.cpu.eip)).or_insert(0) += 1;
                }
            }
            let ran = (self.cpu.icount - start_ic).max(1);
            done += ran;
            let step_t = ran as f64 / self.ips;
            self.m.time += step_t;
            match res {
                Ok(()) => {}
                Err(Fault::Halt) => {
                    // idle until next interrupt: skip ahead in time
                    let hz = self.m.pit_hz();
                    let need = (1.0 - self.m.irq0_accum).max(0.0) / hz;
                    self.m.time += need;
                    self.m.irq0_accum = 1.0;
                    done += (need * self.ips) as u64;
                }
                Err(e) => return Err(e),
            }
            self.service_irqs(step_t);
            self.m.service_events(&mut self.cpu, step_t);
            self.m.audio.advance(step_t);
            if self.m.display.help_open { return Ok(()); }
        }
        Ok(())
    }

    /// Run in real time until the wall-clock deadline.
    pub fn run_realtime(&mut self, until: std::time::Instant) -> Result<(), Fault> {
        self.run_realtime_slice(until, false)
    }

    /// Stop on a completed page flip, not halfway through the next world frame.
    pub fn run_frame(&mut self, until: std::time::Instant) -> Result<(), Fault> {
        self.run_realtime_slice(until, true)
    }

    fn run_realtime_slice(&mut self, until: std::time::Instant, frame: bool) -> Result<(), Fault> {
        let serial = self.m.display.frame_serial;
        let mut last = self.realtime_last.unwrap_or_else(std::time::Instant::now);
        loop {
            let mut res = Ok(());
            for _ in 0..4000 {
                if let Err(e) = self.cpu.step(&mut self.m) {
                    res = Err(e);
                    break;
                }
                if self.m.display.help_open || (frame && self.m.display.frame_serial != serial) { break; }
                self.debug_overrides();
                if let Some(c) = &mut self.counter {
                    if self.cpu.eip == c.0 {
                        c.1 += 1;
                    }
                }
            }
            if let Some(p) = &mut self.profile {
                if self.m.time >= self.profile_start {
                    *p.entry(le::link(self.cpu.eip)).or_insert(0) += 1;
                }
            }
            let now = std::time::Instant::now();
            let dt = (now - last).as_secs_f64();
            last = now;
            self.realtime_last = Some(now);
            self.m.time += dt;
            match res {
                Ok(()) => {}
                Err(Fault::Halt) => std::thread::sleep(std::time::Duration::from_micros(500)),
                Err(e) => return Err(e),
            }
            self.service_irqs(dt);
            self.m.service_events(&mut self.cpu, dt);
            self.m.audio.advance(dt);
            if now >= until || self.m.display.options_open || self.m.display.help_open
                || (frame && self.m.display.frame_serial != serial) {
                return Ok(());
            }
        }
    }

    fn service_irqs(&mut self, dt: f64) {
        // vertical retrace at 70 Hz, 10% duty
        let ph = (self.m.time * 70.0).fract();
        self.m.vga.retrace = ph < 0.1;
        self.m.irq0_accum += dt * self.m.pit_hz();
        if self.m.irq0_accum > 8.0 {
            self.m.irq0_accum = 8.0;
        }
        if self.m.irq0_accum >= 1.0 && self.m.raise_irq(&mut self.cpu, 0) {
            self.m.irq0_accum -= 1.0;
        }
        if !self.m.kbd_queue.is_empty() && self.m.pic_isr & 2 == 0 && self.cpu.iflag() {
            // latch next scancode into port 60h, then fire IRQ1
            let sc = self.m.kbd_queue.pop_front().unwrap();
            self.m.port60 = sc;
            self.m.raise_irq(&mut self.cpu, 1);
        }
    }

    fn check_watches(&mut self) {
        for w in self.watches.iter_mut() {
            let v = self.cpu.rd8(w.0);
            if v != w.1 {
                eprintln!(
                    "[watch] {:06X}: {:02X} -> {:02X} at {:06X} t={:.3}",
                    le::link(w.0),
                    w.1,
                    v,
                    le::link(self.cpu.cur_eip),
                    self.m.time
                );
                w.1 = v;
            }
        }
    }

    pub fn crash_report(&self, e: Fault) -> String {
        let mut s = format!("FAULT {:?} at link {:06X}\n{}\n", e, le::link(self.cpu.cur_eip), self.cpu.dump());
        if self.cpu.trace_on {
            s.push_str("recent EIPs (link):");
            for e in self.cpu.trace_history() {
                s.push_str(&format!(" {:06X}", le::link(e)));
            }
            s.push('\n');
        }
        s
    }

    pub fn screenshot(&mut self, path: &std::path::Path) {
        let mut buf = Vec::new();
        let (w, h) = display::render(&mut self.m, &self.cpu, &mut buf);
        write_bmp(path, w, h, &buf);
    }
}

fn write_default_config(overlay: &std::path::Path) {
    let p = overlay.join("CMPATH.CNF");
    if !p.exists() {
        // cdPath, gamePath, spkPath, corePath (see LoadSetup)
        std::fs::write(&p, "D:\\CYBERMAG\r\nC:\\CYBERMAG\r\nD:\\CYBERMAG\r\nD:\\CYBERMAG\r\n").ok();
    }
}

pub fn write_bmp(path: &std::path::Path, w: usize, h: usize, px: &[u32]) {
    let row = w * 3;
    let pad = (4 - row % 4) % 4;
    let size = 54 + (row + pad) * h;
    let mut b = Vec::with_capacity(size);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&(size as u32).to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&54u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(w as i32).to_le_bytes());
    b.extend_from_slice(&(h as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(&[0; 24]);
    for y in (0..h).rev() {
        for x in 0..w {
            let p = px[y * w + x];
            b.extend_from_slice(&[p as u8, (p >> 8) as u8, (p >> 16) as u8]);
        }
        b.extend(std::iter::repeat(0).take(pad));
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).ok();
    }
    std::fs::write(path, b).ok();
}

fn default_root() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    // Development builds use the project; a portable release uses its own directory.
    for ancestor in exe.ancestors() {
        if ancestor.join("Cargo.toml").is_file() && exe.starts_with(ancestor.join("target")) {
            return ancestor.to_path_buf();
        }
    }
    exe.parent().unwrap().to_path_buf()
}

fn parse_args() -> Config {
    let mut cfg = Config {
        root: default_root(),
        headless: false,
        seconds: 10.0,
        ips: 60_000_000.0,
        screenshot_every: 0.0,
        keys: Vec::new(),
        holds: Vec::new(),
        trace: false,
        realtime: false,
        resolution: None,
        pads: Vec::new(),
        desktop: Vec::new(),
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--headless" => cfg.headless = true,
            "--seconds" => {
                i += 1;
                cfg.seconds = args[i].parse().unwrap()
            }
            "--ips" => {
                i += 1;
                cfg.ips = args[i].parse().unwrap()
            }
            "--shots" => {
                i += 1;
                cfg.screenshot_every = args[i].parse().unwrap()
            }
            "--root" => {
                i += 1;
                cfg.root = PathBuf::from(args.get(i).expect("--root requires a directory"));
            }
            "--import-assets" => { i += 1; }
            "--resolution" => {
                i += 1;
                cfg.resolution = Some(args.get(i).and_then(|v| display::parse_mode(v))
                    .expect("--resolution requires a supported WIDTHxHEIGHT (e.g. 1920x1080)"));
            }
            "--trace" => cfg.trace = true,
            "--realtime" => cfg.realtime = true,
            "--pad" => {
                // START:END:LX:LY:RX:RY:BUTTONS(hex):LT:RT, axes -1..1, triggers 0..1
                i += 1;
                let p: Vec<&str> = args.get(i).expect("--pad requires a controller hold").split(':').collect();
                assert!(p.len() == 9, "--pad expects START:END:LX:LY:RX:RY:BUTTONS:LT:RT");
                let number = |n: usize, lo: f32, hi: f32| {
                    let v: f32 = p[n].parse().expect("invalid --pad number");
                    assert!(v.is_finite() && (lo..=hi).contains(&v), "--pad number out of range");
                    v
                };
                let (a, b): (f64, f64) = (p[0].parse().unwrap(), p[1].parse().unwrap());
                assert!(a.is_finite() && b.is_finite() && a >= 0.0 && b > a, "invalid --pad time interval");
                cfg.pads.push((a, b, gamepad::Pad { connected: true,
                    left: gamepad::stick(number(2,-1.0,1.0),number(3,-1.0,1.0),0.0),
                    right: gamepad::stick(number(4,-1.0,1.0),number(5,-1.0,1.0),0.0),
                    buttons: u16::from_str_radix(p[6].trim_start_matches("0x"),16).unwrap(),
                    lt: number(7,0.0,1.0), rt: number(8,0.0,1.0), ..gamepad::Pad::default() }));
            }
            "--kb" => {
                i += 1;
                let hold = args.get(i).ok_or_else(|| "--kb requires a desktop hold".to_string()).and_then(|s| desktop::parse_script(s));
                match hold {Ok(h) => cfg.desktop.push(h), Err(e) => {eprintln!("{e}"); std::process::exit(2);}}
            }
            "--hold" => {
                // --hold START:END:SCANCODE(hex, prefix e for extended)  e.g. 10:12:e48 = Up arrow
                i += 1;
                let p: Vec<&str> = args[i].split(':').collect();
                let (sc, ext) = match p[2].strip_prefix('e') {
                    Some(h) => (u8::from_str_radix(h, 16).unwrap(), true),
                    None => (u8::from_str_radix(p[2], 16).unwrap(), false),
                };
                cfg.holds.push((p[0].parse().unwrap(), p[1].parse().unwrap(), sc, ext));
            }
            "--key" => {
                // --key TIME:SCANCODE(hex)
                i += 1;
                let (t, k) = args[i].split_once(':').unwrap();
                cfg.keys.push((t.parse().unwrap(), u8::from_str_radix(k, 16).unwrap()));
            }
            _ => {}
        }
        i += 1;
    }
    cfg
}

fn headless(cfg: &Config) {
    let mut emu = Emu::new(cfg);
    let mut t = 0.0;
    let step = 0.05;
    let mut next_shot = cfg.screenshot_every;
    let mut keys = cfg.keys.clone();
    let mut holds: Vec<(f64, f64, u8, bool, u8)> = cfg.holds.iter().map(|&(a, b, s, e)| (a, b, s, e, 0)).collect();
    let mut shot_n = 0;
    let mut pad_keys = gamepad::Keys::default();
    let mut previous_pad = gamepad::Pad::default();
    while t < cfg.seconds {
        if !cfg.pads.is_empty() || !cfg.desktop.is_empty() {
            let mut p = cfg.pads.iter().find(|&&(a,b,_)| t >= a && t < b).map(|&(_,_,p)| p)
                .unwrap_or(gamepad::Pad { connected: true, ..gamepad::Pad::default() });
            let playing = gamepad::gameplay(&emu.m,&emu.cpu);
            let mut edges=Vec::new(); let mut extra=[false;256]; let mut wheel=0.0;
            if !cfg.desktop.is_empty() {
                let mut shot=cfg.desktop.iter().find(|(a,b,_)| t>=*a && t<*b).map(|(_,_,s)|s.clone()).unwrap_or_default();
                shot.delta.0 *= step as f32; shot.delta.1 *= step as f32;
                wheel=shot.wheel; edges=emu.m.desktop.edges(&shot);
                (p,extra)=emu.m.desktop.update(shot,p,playing,&emu.cpu);
                if !emu.m.display.help_open {emu.m.display.help_keyboard=!emu.m.desktop.controller_guide;}
            }
            emu.m.gamepad = gamepad::Pad { guest_this: emu.m.gamepad.guest_this, apply_stub: emu.m.gamepad.apply_stub,
                last_time: emu.m.gamepad.last_time, pitch_fraction: emu.m.gamepad.pitch_fraction,
                jump_held: emu.m.gamepad.jump_held && p.down(gamepad::A), ..p };
            if !frontend::guide_input(&mut emu,&mut pad_keys,p,previous_pad,&edges) {
                pad_keys.desktop(&mut emu.m.kbd_queue,extra);
                emu.m.gamepad.game_buttons = pad_keys.controller(&mut emu.m.kbd_queue,p,&emu.cpu,playing);
                let buttons=pad_keys.menu_mouse();
                gamepad::pointer(&mut emu.m,p,playing,buttons,step as f32);
                if !cfg.desktop.is_empty() {desktop::pointer(&mut emu.m,playing);}
                if !playing && wheel!=0.0 {gamepad::tap(&mut emu.m.kbd_queue,if wheel>0.0 {0x48} else {0x50},true);}
            }
            previous_pad = p;
        }
        gamepad::controller_input(&emu.m,&mut emu.cpu);
        keys.retain(|&(kt, sc)| {
            if kt <= t {
                let was_open = emu.m.display.options_open;
                if !display::settings_key(&mut emu.m.display,sc) {
                    emu.m.kbd_queue.push_back(sc);
                    emu.m.kbd_queue.push_back(sc | 0x80);
                }
                if was_open && !emu.m.display.options_open {
                    emu.m.display.save(&emu.m.vfs.overlay.join("NATIVE.CNF")).expect("save native settings");
                }
                false
            } else {
                true
            }
        });
        for h in holds.iter_mut() {
            if h.4 == 0 && t >= h.0 {
                if h.3 {
                    emu.m.kbd_queue.push_back(0xE0);
                }
                emu.m.kbd_queue.push_back(h.2);
                h.4 = 1;
            } else if h.4 == 1 && t >= h.1 {
                if h.3 {
                    emu.m.kbd_queue.push_back(0xE0);
                }
                emu.m.kbd_queue.push_back(h.2 | 0x80);
                h.4 = 2;
            }
        }
        let res = if emu.m.display.options_open || emu.m.display.help_open {
            emu.realtime_last = None;
            emu.m.audio.advance_music(step); // as the windowed frontend: no held notes
            Ok(())
        } else if cfg.realtime {
            emu.run_realtime(std::time::Instant::now() + std::time::Duration::from_secs_f64(step))
        } else {
            emu.run_for(step)
        };
        if let Err(e) = res {
            if e == Fault::Stop {
                if let Some(c) = emu.m.exit_code {
                    eprintln!("program exited with code {}", c);
                    break;
                }
                if let Some((p, a)) = emu.m.spawn_request.take() {
                    eprintln!("spawn {} {:?} (skipped in headless)", p, a);
                    // resume as if child returned success
                    emu.cpu.set_cf(false);
                    emu.cpu.r[x86::EAX] = 0;
                    continue;
                }
            }
            eprintln!("{}", emu.crash_report(e));
            break;
        }
        t += step;
        if cfg.screenshot_every > 0.0 && t >= next_shot {
            next_shot += cfg.screenshot_every;
            let p = cfg.root.join("out").join(format!("shot_{:04}.bmp", shot_n));
            shot_n += 1;
            emu.screenshot(&p);
        }
    }
    if let Ok(seconds) = std::env::var("CM_BENCH_SECONDS") {
        if emu.m.verifier.is_none() {
            benchmark(&mut emu, seconds.parse().expect("CM_BENCH_SECONDS must be a number"));
        } else { eprintln!("benchmark skipped: differential mode disables host presentation"); }
    }
    if let Some(p) = &emu.profile {
        std::fs::create_dir_all(cfg.root.join("out")).expect("profile output directory");
        let mut s = String::new();
        for (a, n) in p {
            s.push_str(&format!("{:06X} {}
", a, n));
        }
        std::fs::write(cfg.root.join("out").join("profile.txt"), s).ok();
    }
    let p = cfg.root.join("out").join("final.bmp");
    emu.screenshot(&p);
    if let Ok(spec) = std::env::var("CM_DUMP") {
        // CM_DUMP=linkaddr:len  -> hex dump of guest memory at exit
        if let Some((a, n)) = spec.split_once(':') {
            let a = u32::from_str_radix(a.trim_start_matches("0x"), 16).unwrap_or(0);
            let n: u32 = n.parse().unwrap_or(64);
            for row in (0..n).step_by(16) {
                let b = emu.cpu.read_bytes(le::rt(a + row), 16.min((n - row) as usize));
                eprintln!("{:06X}: {:02X?}", a + row, b);
            }
        }
    }
    if let Some(v) = &emu.m.verifier {
        eprintln!("verify: {} checked, {} mismatches", v.checked, v.mismatches);
        for (n, (k, bad)) in &v.per {
            eprintln!("  {:40} {:8} checked {:6} mismatched", n, k, bad);
        }
    }
    if let Some(c) = emu.counter {
        eprintln!("counter {:06X}: {}", le::link(c.0), c.1);
    }
    eprintln!(
        "CMTimerTicks={} events={:?}",
        emu.cpu.rd32(le::rt(0x207473)),
        emu.m.events
    );
    eprintln!(
        "ran {:.2}s, {} instructions, mode {:X}, eip link {:06X}",
        t,
        emu.cpu.icount,
        emu.m.vga.mode,
        le::link(emu.cpu.eip)
    );
}

/// Uncapped frame-work benchmark after scripted boot and two seconds of warmup.
/// Includes composition; CM_BENCH_WINDOW also measures real window upload.
fn benchmark(emu: &mut Emu, seconds: f64) {
    use std::time::{Duration, Instant};
    assert!(seconds.is_finite() && seconds > 0.0 && seconds <= 60.0);
    let mut first_frame = emu.m.display.frame_serial;
    let mut times = Vec::new();
    let mut buf = Vec::new();
    let mut window = std::env::var_os("CM_BENCH_WINDOW").map(|_| frontend::create_window(&emu.m.display,None).expect("benchmark window"));
    let mut presents = 0;
    let mut compose = Duration::ZERO;
    let mut upload = Duration::ZERO;
    let mut work = Duration::ZERO;
    let mut start = Instant::now();
    let mut warming = true;
    while warming || start.elapsed().as_secs_f64() < seconds {
        if warming && start.elapsed().as_secs_f64() >= 2.0 {
            warming = false; start = Instant::now();
            first_frame = emu.m.display.frame_serial;
            times.clear(); presents = 0; compose = Duration::ZERO; upload = Duration::ZERO; work = Duration::ZERO;
        }
        let frame_start = Instant::now();
        let serial = emu.m.display.frame_serial;
        emu.run_frame(frame_start + Duration::from_secs_f64(1.0 / 60.0)).expect("benchmark emulation");
        let before = Instant::now();
        display::render(&mut emu.m, &emu.cpu, &mut buf);
        compose += before.elapsed();
        if let Some(window) = &mut window {
            let before = Instant::now();
            let (w,h) = emu.m.display.size();
            window.update_with_buffer(&buf,w,h).expect("benchmark upload");
            upload += before.elapsed();
        }
        std::hint::black_box(&buf);
        work += frame_start.elapsed();
        if emu.m.display.frame_serial != serial {
            times.push(work.as_secs_f64() * 1000.0);
            emu.m.display.tune_resolution(work.as_secs_f64());
            work = Duration::ZERO;
        }
        presents += 1;
    }
    let elapsed = start.elapsed().as_secs_f64();
    let (w,h) = emu.m.display.size();
    times.sort_by(f64::total_cmp);
    assert!(!times.is_empty(), "benchmark must reach game presentation");
    eprintln!("benchmark {w}x{h}: {:.2} game fps, {:.2} presents/s, {:.3} ms composition/present; frame work p50={:.2} p95={:.2} p99={:.2} ms, world={}x{}",
        (emu.m.display.frame_serial - first_frame) as f64 / elapsed, presents as f64 / elapsed,
        compose.as_secs_f64() * 1000.0 / presents as f64,
        times[times.len()/2], times[(times.len()-1)*95/100], times[(times.len()-1)*99/100],
        emu.m.display.world_size.0, emu.m.display.world_size.1);
    if window.is_some() { eprintln!("window upload: {:.3} ms/present (included in frame work above)",upload.as_secs_f64()*1000.0/presents as f64); }
}

#[cfg(test)]
mod timing_tests {
    use super::*;
    #[test]
    fn frame_slice_stops_at_first_complete_flip() {
        let path = PathBuf::from("__frame_slice_no_files__");
        let mut emu = Emu {
            cpu: Cpu::new(), m: Machine::new(vfs::Vfs::new(path.clone(),path)),
            ips: 1_000_000.0, cfg_trace: false, watches: Vec::new(), profile: None,
            counter: None, profile_start: 0.0, scene_override: None, vehicle_anywhere: None, realtime_last: None,
        };
        emu.cpu.r[x86::ESP] = 0x30000;
        // call flip; jmp back to call. A frame slice must not start a second flip.
        emu.cpu.write_bytes(0,&[0xE8,0xFB,0xFF,0x40,0,0xEB,0xF9]);
        emu.m.hook(&mut emu.cpu,0x10000,"test flip",|m,c| {
            m.display.frame_serial += 1; c.ret_near(0); Ok(())
        });
        for serial in 1..=2 {
            emu.run_frame(std::time::Instant::now()+std::time::Duration::from_secs(1)).unwrap();
            assert_eq!(emu.m.display.frame_serial,serial);
            assert_eq!(emu.cpu.eip,5);
            assert_eq!(emu.cpu.r[x86::ESP],0x30000);
        }
    }
    #[test]
    fn realtime_counts_presentation_gaps_but_not_pauses_or_scripted_time() {
        use std::time::{Duration, Instant};
        let path = PathBuf::from("__timing_test_no_files__");
        let mut emu = Emu {
            cpu: Cpu::new(), m: Machine::new(vfs::Vfs::new(path.clone(),path)),
            ips: 1_000_000.0, cfg_trace: false, watches: Vec::new(), profile: None,
            counter: None, profile_start: 0.0, scene_override: None, vehicle_anywhere: None, realtime_last: None,
        };
        // A halted CPU is sufficient: the game clock must still advance.
        emu.cpu.wr8(0,0xF4);
        emu.realtime_last = Some(Instant::now() - Duration::from_millis(50));
        emu.run_realtime(Instant::now()).unwrap();
        assert!(emu.m.time >= 0.05);
        let before = emu.m.time;
        emu.run_for(0.0).unwrap();
        assert!(emu.realtime_last.is_none());
        assert_eq!(emu.m.time,before);
        // The frontend resets this marker when inactive/in native options/movies.
        emu.realtime_last = None;
        emu.run_realtime(Instant::now()).unwrap();
        assert!(emu.realtime_last.is_some());
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 2 && args[1] == "--tgv-dump" {
        let n = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(150);
        tgv::dump_frames(std::path::Path::new(&args[2]), n, std::path::Path::new("out"));
        return;
    }
    let cfg = parse_args();
    if let Some(index) = args.iter().position(|s| s == "--import-assets") {
        let Some(source) = args.get(index+1) else { eprintln!("--import-assets requires an ISO or folder path"); std::process::exit(2); };
        match assets::import(std::path::Path::new(source),&cfg.root) {
            Ok(()) => eprintln!("Imported game assets into {}",cfg.root.join("iso/CYBERMAG").display()),
            Err(e) => { eprintln!("Cannot import game assets: {e}"); std::process::exit(1); }
        }
        return;
    }
    match assets::ensure(&cfg.root,!cfg.headless) {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => { eprintln!("Cannot start CyberMage: {e}"); std::process::exit(1); }
    }
    if cfg.headless {
        headless(&cfg);
    } else {
        frontend::run(&cfg);
    }
}
