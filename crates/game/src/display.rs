//! Native output modes and the host options page. DOS UI pixels keep square proportions.
use crate::{le::rt, machine::Machine};
use std::{io::{Read, Seek, SeekFrom}, path::Path};
use x86::{Cpu, Fault, EAX, EDX, EBX};

pub const MODES: &[(usize, usize)] = &[
    (640, 480), (800, 600), (1024, 768), (1280, 720), (1280, 800),
    (1280, 960), (1600, 900), (1920, 1080), (1920, 1200),
    (2560, 1080), (2560, 1440), (3440, 1440), (3840, 1080),
    (3840, 1600), (3840, 2160), (5120, 1440), (5120, 2160),
];

pub const MENU_TABS: [&str; 3] = ["GAME", "DISPLAY", "ACTIONS"];
pub const TAB_WIDTH: usize = 384 / MENU_TABS.len();
pub const SETTINGS_BACK: usize = 5;

pub struct Display {
    pub mode: usize,
    pub fullscreen: bool,
    pub sensitivity: f32,
    pub deadzone: f32,
    pub invert_y: bool,
    pub options_open: bool,
    pub help_open: bool,
    pub help_keyboard: bool,
    pub ui_pointer: (i32,i32),
    pub options_original: bool,
    pub pending_game_options: bool,
    pub options_in_game: bool,
    pub options_return: u32,
    pub tab: usize,
    pub art: MenuArt,
    pub row: usize,
    pub recording_hud: bool,
    pub current_view: u32,
    pub pending: bool,
    pub presented: bool,
    pub world: Vec<u8>,
    pub world_size: (usize, usize),
    pub pending_world_size: (usize, usize),
    pub frame_serial: u64,
    /// 3D pixel budget; UI/output resolution is independent.
    world_pixels: usize,
    fixed_pixels: bool,
    frame_work: f64,
    sample_frames: usize,
    slow_frames: usize,
    pub hud: Vec<bool>,
    pub rect: (usize, usize, usize, usize),
    pub pending_world: Vec<u8>,
    pub pending_hud: Vec<bool>,
    pub pending_rect: (usize, usize, usize, usize),
    pub flip_stub: u32,
    pub guest_fb: u32,
    pub guest_angles: u32,
    pub original_options: u32,
    pub original_hud: u32,
    pub original_flip: u32,
    pub original_input: u32,
    pub original_menu: u32,
    pub original_title: u32,
    pub original_title_events: u32,
    pub original_navigation: [u32; 5],
    pub legacy_rgb: Vec<u32>,
    pub original_render: u32,
    pub render_return: u32,
    /// Recording the pause menu's overlay writes (monitors, text, glove cursor).
    pub recording_menu: bool,
    pub menu_log: Vec<(u32, u8)>,
    death: Option<DeathMelt>,
}

/// PCDeath melts a frozen screen down in random columns, revealing menu artwork.
struct DeathMelt {
    frame: Vec<u32>,
    size: (usize, usize),
    backdrop: Vec<u8>,
    columns: [u16; 640],
    melting: bool,
}

impl DeathMelt {
    fn render(&self, pal: &[u32; 256], out: &mut Vec<u32>) -> (usize, usize) {
        let (w,h) = self.size;
        let edges: Vec<usize> = (0..=640).map(|x| (x*w).div_ceil(640)).collect();
        let shifts: [usize;640] = std::array::from_fn(|x| self.columns[x].saturating_sub(7).min(480) as usize*h/480);
        // All output modes are at least 4:3. Uniformly cover the screen with the
        // tunnel artwork (crop vertically), not stretched or letterboxed.
        let crop = (480-h*640/w)/2;
        out.resize(w*h,0);
        for y in 0..h {
            let background = &self.backdrop[(crop+y*640/w)*640..][..640];
            let row = &mut out[y*w..(y+1)*w];
            for x in 0..640 {
                let (a,b) = (edges[x],edges[x+1]);
                if y < shifts[x] { row[a..b].fill(pal[background[x] as usize]); }
                else { row[a..b].copy_from_slice(&self.frame[(y-shifts[x])*w+a..(y-shifts[x])*w+b]); }
            }
        }
        self.size
    }
}

impl Display {
    pub fn new(path: &Path) -> Self {
        let mut s = Self {
            mode: 3, fullscreen: false, sensitivity: 1.0, deadzone: 0.18, invert_y: false,
            options_open: false, help_open: false, help_keyboard: false, ui_pointer: (320,240), options_original: false, pending_game_options: false, options_in_game: false, options_return: 0,
            tab: 0, art: MenuArt::default(), row: 0,
            current_view: 0, pending: false, presented: false, recording_hud: false,
            world: Vec::new(), world_size: (0,0), pending_world_size: (0,0), frame_serial: 0,
            world_pixels: 1280 * 720, fixed_pixels: false, frame_work: 0.0, sample_frames: 0, slow_frames: 0,
            hud: vec![false; 640 * 480], rect: (0, 0, 0, 0),
            pending_world: Vec::new(), pending_hud: vec![false; 640 * 480], pending_rect: (0, 0, 0, 0), flip_stub: 0,
            guest_fb: 0, guest_angles: 0, original_options: 0, original_hud: 0,
            original_flip: 0, original_input: 0, original_menu: 0, original_title: 0, original_title_events: 0, original_navigation: [0; 5], legacy_rgb: Vec::new(),
            original_render: 0, render_return: 0, recording_menu: false, menu_log: Vec::new(), death: None,
        };
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines() {
                let Some((key, value)) = line.split_once('=') else { continue };
                match key {
                    "resolution" => { if let Some(i) = parse_mode(value) { s.mode = i; } }
                    "sensitivity" => { if let Ok(v) = value.parse::<f32>() { if v.is_finite() { s.sensitivity = v.clamp(0.25, 3.0); } } }
                    "deadzone" => { if let Ok(v) = value.parse::<f32>() { if v.is_finite() { s.deadzone = v.clamp(0.05, 0.4); } } }
                    "invert_y" => s.invert_y = value == "true",
                    "fullscreen" => s.fullscreen = value == "true",
                    _ => {}
                }
            }
        }
        // Diagnostic override for native-quality comparisons, not a saved setting.
        if let Some(pixels) = std::env::var("CM_NATIVE_PIXELS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&v| v > 0) {
            s.world_pixels = pixels.min(5120 * 2160);
            s.fixed_pixels = true;
        }
        s
    }
    pub fn raster_size(&self, w: usize, h: usize) -> (usize, usize) {
        let scale = (self.world_pixels as f64 / (w * h) as f64).sqrt().min(1.0);
        (((w as f64 * scale) as usize & !1).max(2), ((h as f64 * scale) as usize).max(2))
    }
    /// Adjust only after sustained work; leave headroom for upload and scheduling.
    pub fn tune_resolution(&mut self, seconds: f64) {
        if self.fixed_pixels || !self.presented || self.mode == 0 { return; }
        self.frame_work += seconds;
        self.slow_frames += usize::from(seconds > 1.0 / 60.0);
        self.sample_frames += 1;
        if self.sample_frames < 60 { return; }
        let average = self.frame_work / self.sample_frames as f64;
        let (w,h) = self.size();
        if average > 0.012 || self.slow_frames >= 3 { self.world_pixels = (self.world_pixels * 4 / 5).max(640 * 360); }
        else if average < 0.009 && self.slow_frames == 0 { self.world_pixels = (self.world_pixels * 11 / 10).min(w * h); }
        self.frame_work = 0.0;
        self.sample_frames = 0;
        self.slow_frames = 0;
    }
    pub fn size(&self) -> (usize, usize) { MODES[self.mode] }
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let (w, h) = self.size();
        std::fs::write(path, format!("resolution={w}x{h}\nsensitivity={}\ndeadzone={}\ninvert_y={}\nfullscreen={}\n", self.sensitivity, self.deadzone, self.invert_y, self.fullscreen))
    }
    pub fn change(&mut self, delta: i32) {
        match self.row {
            0 => { self.mode = (self.mode as i32 + delta).rem_euclid(MODES.len() as i32) as usize; self.presented = false; }
            1 => self.sensitivity = (self.sensitivity + delta as f32 * 0.25).clamp(0.25, 3.0),
            2 => self.deadzone = (self.deadzone + delta as f32 * 0.025).clamp(0.05, 0.4),
            3 => self.invert_y = !self.invert_y,
            4 => self.fullscreen = !self.fullscreen,
            _ => {}
        }
    }
}

pub fn parse_mode(text: &str) -> Option<usize> {
    let (w, h) = text.split_once('x')?;
    let size = (w.parse().ok()?, h.parse().ok()?);
    MODES.iter().position(|&m| m == size)
}

/// Copy complete initial instructions, then jump back past the trap. These
/// entry points have no relative instructions in the copied prologue.
fn original(m: &mut Machine, c: &mut Cpu, link: u32, bytes: usize) -> u32 {
    let p = m.heap_alloc((bytes + 5) as u32).expect("native trampoline memory");
    let code = c.read_bytes(rt(link), bytes).to_vec();
    c.write_bytes(p, &code);
    c.wr8(p + bytes as u32, 0xE9);
    c.wr32(p + bytes as u32 + 1, (rt(link) + bytes as u32).wrapping_sub(p + bytes as u32 + 5));
    p
}

/// The constructor's ESI=2 is used only to initialize ConSize, before ESI is
/// overwritten. Entry 0 is 640x480; larger ConSize indices shrink the view.
fn install_default_viewport(c: &mut Cpu) {
    assert_eq!(c.read_bytes(rt(0x01025B),5),[0xBE,2,0,0,0]);
    c.wr32(rt(0x01025C),0);
}

pub fn install(m: &mut Machine, c: &mut Cpu) {
    install_default_viewport(c);
    crate::gamepad::install_menu_routes(c);
    for (i, &(addr, ..)) in crate::gamepad::MENU_NAVIGATION.iter().enumerate() {
        m.display.original_navigation[i] = original(m,c,addr,7);
        m.hook_sig(c,addr,"Xox::ControllerMenuWrap",0,false,crate::gamepad::menu_navigation);
    }
    m.display.original_options = original(m, c, 0x053DDC, 4);
    m.display.original_hud = original(m, c, 0x01899C, 4);
    m.display.original_flip = original(m, c, 0x06B3F0, 5);
    m.display.original_input = original(m, c, 0x012DD4, 4);
    m.display.original_menu = original(m, c, 0x04E690, 4);
    m.display.original_title = original(m, c, 0x054C20, 4);
    m.display.original_title_events = original(m, c, 0x054D97, 5);
    m.display.original_render = original(m, c, 0x065D34, 5);
    m.hook_sig(c, 0x054C20, "Xox::ControllerTitlePointer", 3, false, title_pointer);
    m.hook_sig(c, 0x04E690, "Xox::ControllerMenuPointer", 2, false, menu_pointer);
    m.hook_sig(c, 0x04E82A, "Xox::RequestedPauseOptions", 0, false, pause_options_request);
    m.hook_sig(c, 0x054D97, "Xox::RequestedTitleOptions", 0, false, title_options_request);
    m.hook_sig(c, 0x04E390, "Xox::QuitToDosYN", 1, true, quit_confirm);
    m.hook_sig(c, 0x04FBAC, "Xox::AutomaticSaveName", 0, false, automatic_save_name);
    m.hook_sig(c, 0x053DDC, "Xox::NativeOptions", 1, false, options);
    m.hook_sig(c, 0x053F08, "Xox::ControllerOptionsRows", 0, false, controller_options_rows);
    m.hook_sig(c, 0x0544EF, "Xox::ControllerOptionsSelect", 0, false, controller_options_select);
    m.hook_sig(c, 0x051FE8, "Xox::ControllerHelp", 1, false, controller_help);
    let stub = m.heap_alloc(4).expect("options return trap");
    m.hook_sig(c, crate::le::link(stub), "Xox::OptionsReturn", 0, false, options_return);
    m.display.options_return = stub;
    m.hook_sig(c, 0x01899C, "Xox::NativeHud", 3, false, hud);
    m.hook_sig(c, 0x065D34, "Image1::MenuRender", 2, false, menu_render);
    let stub = m.heap_alloc(4).expect("menu render return trap");
    m.hook_sig(c, crate::le::link(stub), "Image1::MenuOverlay", 0, false, menu_overlay);
    m.display.render_return = stub;
    m.hook_sig(c, 0x05F8E0, "Image1::NativeTint", 4, false, tint);
    m.hook_sig(c, 0x06B3F0, "Video::NativePresent", 2, false, flip);
    let stub = m.heap_alloc(4).expect("presentation return trap");
    m.hook_sig(c, crate::le::link(stub), "Video::NativePublish", 0, false, publish);
    m.display.flip_stub = stub;
    assert_eq!(c.read_bytes(rt(0x015459),6),[0x8D,0x85,0x20,0x59,0x0E,0x00]);
    m.hook_sig(c, 0x015459, "Xox::NativeDeathMelt", 0, false, death_melt);
    assert_eq!(c.read_bytes(rt(0x01551A),7),[0x8B,0x84,0x24,0x08,0x05,0x00,0x00]);
    m.hook_sig(c, 0x01551A, "Xox::NativeDeathColumn", 0, false, death_column);
    for addr in [0x04F042,0x04FB13] {
        assert_eq!(c.read_bytes(rt(addr),5),[0xFF,0x53,0x04,0xEB,0x09]);
        m.hook_sig(c,addr,"Xox::DeathMenuOverlay",0,false,death_menu_overlay);
    }
    m.hook_sig(c, 0x012DD4, "Xox::GamepadInput", 1, false, crate::gamepad::input_hook);
}

fn menu_pointer(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    // MenuOptions caches the controller's virtual pointer route on entry.
    crate::gamepad::controller_input(m,c);
    c.eip = m.display.original_menu;
    Ok(())
}

fn title_pointer(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    // Startup clears routing. Enable before TitleMenu positions its initial
    // cursor, or the default cursor can select Credits instead of Start.
    crate::gamepad::controller_input(m,c);
    c.eip = m.display.original_title;
    Ok(())
}

fn requested_options_key(m: &Machine, c: &mut Cpu) {
    if m.display.pending_game_options {
        c.wr8(rt(0x207C55),0); // don't dismiss the menu with the opening Escape
        c.wr8(rt(0x207C6C),1); // original Options shortcut, after menu initialization
    }
}

fn pause_options_request(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    requested_options_key(m,c);
    c.r[EDX] = c.rd32(c.r[x86::ESP]+0x3C);
    c.eip = rt(0x04E82E);
    Ok(())
}

fn title_options_request(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    requested_options_key(m,c);
    c.eip = m.display.original_title_events; // replay the original CMP and flags
    Ok(())
}

fn controller_options_rows(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let sp = c.r[x86::ESP];
    // Keep original positions and setting handlers for rows 3–9. Rows 0–2
    // (device, DOS joystick calibration, keyboard remapping) are removed.
    c.r[x86::ESI] = 3;
    c.r[x86::EDI] = 234;
    c.r[EBX] = 46;
    c.wr32(sp+0x24,12);
    c.wr32(sp+0x20,c.rd32(sp+0x20).max(3));
    c.eip = rt(0x053F11);
    Ok(())
}

fn controller_options_select(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let sp = c.r[x86::ESP];
    let row = c.rd32(sp+0x20).max(3);
    c.wr32(sp+0x20,row);
    c.r[x86::ECX] = row;
    c.eip = rt(0x0544F3);
    Ok(())
}

fn controller_help(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let this = c.r[EAX];
    let keyboard_caller = c.rd32(c.r[x86::ESP]) == rt(0x014056);
    m.display.help_open = true;
    c.ret_near(0);
    if keyboard_caller {
        // The host owns dismissal. Skip the caller's DOS key/mouse wait, but
        // keep its normal input cleanup and the original frame-redraw request.
        c.wr32(this+0xBDEC4,c.rd32(this+0xBDEC0).wrapping_add(1));
        c.r[EAX] = 0x80; c.r[EBX] = 0; c.r[x86::ECX] = 0; c.r[x86::EDI] = 1;
        c.eip = rt(0x0140D8);
    }
    Ok(())
}

pub fn help_image() -> &'static [u32] {
    static PIXELS: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();
    PIXELS.get_or_init(|| include_bytes!("../../../assets/controller-help.rgb")
        .chunks_exact(3).map(|p| ((p[0] as u32)<<16) | ((p[1] as u32)<<8) | p[2] as u32).collect())
}

pub fn present_help(d: &Display, out: &mut Vec<u32>) -> (usize,usize) {
    let size = d.size();
    static KEYBOARD: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();
    let image = if d.help_keyboard {
        KEYBOARD.get_or_init(|| include_bytes!("../../../assets/keyboard-help.rgb")
            .chunks_exact(3).map(|p| ((p[0] as u32)<<16) | ((p[1] as u32)<<8) | p[2] as u32).collect()).as_slice()
    } else {help_image()};
    compose(image,(1280,960),size,&[],(0,0),&[],(0,0,0,0),false,&[0;256],None,out);
    size
}

fn quit_confirm(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    // Return Yes; the original caller still performs normal game shutdown.
    c.r[EAX] = 1;
    c.ret_near(0);
    Ok(())
}

fn format_save_name([year,month,day,hour,minute,second]: [i32;6]) -> String {
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

fn current_save_name() -> std::io::Result<String> {
    #[cfg(windows)]
    let fields = {
        #[link(name = "kernel32")]
        extern "system" { fn GetLocalTime(time: *mut u16); }
        // SYSTEMTIME: year, month, weekday, day, hour, minute, second, millis.
        let mut t = [0u16;8];
        unsafe { GetLocalTime(t.as_mut_ptr()); }
        [t[0] as i32,t[1] as i32,t[3] as i32,t[4] as i32,t[5] as i32,t[6] as i32]
    };
    #[cfg(unix)]
    let fields = unsafe {
        let time = libc::time(std::ptr::null_mut());
        let mut t: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&time,&mut t).is_null() { return Err(std::io::Error::last_os_error()); }
        [t.tm_year+1900,t.tm_mon+1,t.tm_mday,t.tm_hour,t.tm_min,t.tm_sec]
    };
    #[cfg(any(windows,unix))] {
        let name = format_save_name(fields);
        if name.len() > 20 { return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,"date exceeds save-name limit")); }
        Ok(name)
    }
    #[cfg(not(any(windows,unix)))]
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported,"local clock unavailable"))
}

/// FileMenu reaches this point only after accepting a save slot (not loading).
/// Bypass the text loop, but keep its normal rendering, cleanup and SaveGame caller.
fn automatic_save_name(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let sp = c.r[x86::ESP];
    let slot = c.rd32(sp+0x6C).wrapping_sub(1);
    let name = current_save_name();
    match name {
        Ok(name) if slot < 10 => {
            let addr = rt(0x1E3297)+slot*24;
            c.write_bytes(addr,&[0;24]);
            c.write_bytes(addr,name.as_bytes());
            c.wr32(sp+0x4C,slot);
        }
        _ => {
            // Cancel safely rather than corrupt names or save to an invalid slot.
            eprintln!("cannot name save: invalid slot or local clock unavailable");
            c.wr32(sp+0x6C,0);
        }
    }
    c.wr32(sp+0x48,1); // original accepted-name state
    c.wr32(sp+0x50,1); // finish after the current menu frame
    c.wr32(sp+0x68,0); // no text cursor
    c.wr8(rt(0x207CD4),1); // original keyboard-buffer cleanup
    c.eip = rt(0x04FE20);
    Ok(())
}

fn options(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    // Let the DOS options run normally. Host pages suspend/resume this exact
    // guest continuation; they must never pop an arbitrary interrupted stack.
    m.display.options_original = true;
    m.display.pending_game_options = false;
    // The title initializes the player state to 5 too; that field cannot tell
    // title Options from pause Options. Use their actual guest call sites.
    m.display.options_in_game = c.rd32(c.r[x86::ESP]) == rt(0x04F1E2);
    m.display.tab = 0;
    c.push32(m.display.options_return);
    c.eip = m.display.original_options;
    Ok(())
}

fn options_return(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    crate::gamepad::controller_input(m,c);
    m.display.options_original = false;
    m.display.options_in_game = false;
    m.display.tab = 0;
    c.ret_near(0);
    Ok(())
}

/// Controller and diagnostic script navigation; these are virtual DOS codes.
/// Returns true only for commands consumed by the host.
pub fn settings_key(d: &mut Display, sc: u8) -> bool {
    if d.help_open {
        if matches!(sc,0x01 | 0x1C) { d.help_open = false; }
        return true;
    }
    if d.options_original && sc == 0x0F {
        d.options_open = !d.options_open;
        d.tab = usize::from(d.options_open);
        d.row = 0;
        return true;
    }
    if !d.options_open { return false; }
    match sc {
        0x48 => d.row = (d.row + SETTINGS_BACK) % (SETTINGS_BACK + 1),
        0x50 => d.row = (d.row + 1) % (SETTINGS_BACK + 1),
        0x4B => d.change(-1),
        0x4D => d.change(1),
        0x1C if d.row < SETTINGS_BACK => d.change(1),
        0x1C | 0x01 => { d.options_open = false; d.tab = 0; }
        _ => {}
    }
    true
}

/// `MenuOptions` sets the primary view to -1 while it renders view 0 behind the
/// pause menu (0x04E783), and back to 0 on exit (0x04F3C1).
pub const MENU_VIEW: u32 = u32::MAX;
pub fn primary_view(c: &Cpu, this: u32) -> u32 {
    let v = c.rd32(this + 0xBDEBC);
    if v == MENU_VIEW { 0 } else { v }
}
pub fn pause_menu(c: &Cpu) -> bool { c.rd32(rt(0x09E5DC) + 0xBDEBC) == MENU_VIEW }

/// Mark the legacy pixels written since recording started as opaque UI.
fn finish_overlay(d: &mut Display, c: &mut Cpu, this: u32, old_log: Vec<(u32, u8)>) {
    c.wlog_on = false;
    d.recording_hud = false;
    d.pending_hud.fill(false);
    for &(addr, _) in &c.wlog {
        let i = addr.wrapping_sub(this) as usize;
        if i < d.pending_hud.len() { d.pending_hud[i] = true; }
    }
    // Empty callers need no backup: retain the recording allocation next frame.
    if old_log.is_empty() { c.wlog.clear(); } else { c.wlog = old_log; }
}

/// Menus with view -1 (pause, save/load) re-render the world, then draw on top of it.
fn menu_render(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    if pause_menu(c) && m.display.mode != 0 && !c.wlog_on { c.push32(m.display.render_return); }
    c.eip = m.display.original_render;
    Ok(())
}

/// Death menus call the virtual background drawer instead of Image1::Render.
/// Follow that call with the same overlay recorder used by ordinary pause menus.
fn death_menu_overlay(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    // Both MenuOptions and FileMenu use CALL [ebx+4]; JMP past Render.
    c.push32(c.cur_eip+14);
    c.push32(m.display.render_return);
    c.eip = c.rd32(c.r[EBX]+4);
    Ok(())
}

/// After a menu's world render: record what it draws on top, until NextPage.
fn menu_overlay(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let d = &mut m.display;
    if (d.pending || d.death.is_some()) && !c.wlog_on {
        d.menu_log = if c.wlog.is_empty() { Vec::new() } else { std::mem::take(&mut c.wlog) };
        c.wlog_on = true;
        d.recording_hud = true;
        d.recording_menu = true;
    }
    c.ret_near(0);
    Ok(())
}

/// Port of `Xox::ShadeMenuBox` (inclusive box, do-while loops as the original).
/// While a pause menu frame is recorded the shade also applies to the native
/// world, so boxes are tinted high-resolution panels rather than opaque UI.
pub fn shade_menu_box(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, x1, y1, x2) = (c.r[EAX], c.r[EDX] as i32, c.r[EBX] as i32, c.r[x86::ECX] as i32);
    let (y2, shade) = (c.stack_arg(0) as i32, c.stack_arg(1) as i32);
    let map = if shade > 0 { c.rd32(this.wrapping_add(0xBDED8).wrapping_add(shade as u32 * 4)) }
        else { c.rd32(this.wrapping_add(0xBDFD8).wrapping_add(shade.unsigned_abs() * 4)) };
    let palette: [u8; 256] = std::array::from_fn(|i| c.rd8(map.wrapping_add(i as u32)));
    let log_start = c.wlog.len();
    let mut y = y1;
    loop {
        let row = this.wrapping_add((y as u32).wrapping_mul(640));
        let mut x = x1;
        loop {
            let p = row.wrapping_add(x as u32);
            c.wr8(p, palette[c.rd8(p) as usize]);
            x += 1;
            if x > x2 { break; }
        }
        y += 1;
        if y > y2 { break; }
    }
    let d = &mut m.display;
    if d.recording_menu && d.pending {
        c.wlog.truncate(log_start);
        let (w, h) = d.size();
        let (uw, uh, ox, oy) = ui_rect(w, h, 640, 480);
        let (vx, vy, vw, vh) = d.pending_rect;
        // Same column/row boundaries as compose's UI scaling, then world-relative.
        let edge = |a: i32, full: usize, o: usize, u: usize| o + (a.clamp(0, full as i32) as usize * u).div_ceil(full);
        let (cx0, cx1) = (edge(x1, 640, ox, uw).max(vx) - vx, (edge(x2 + 1, 640, ox, uw).max(vx) - vx).min(vw));
        let (cy0, cy1) = (edge(y1, 480, oy, uh).max(vy) - vy, (edge(y2 + 1, 480, oy, uh).max(vy) - vy).min(vh));
        let (rw,rh) = d.pending_world_size;
        if rw > 0 && rh > 0 && d.pending_world.len() == rw * rh {
            for yy in cy0 * rh / vh..(cy1 * rh).div_ceil(vh).min(rh) { for xx in cx0 * rw / vw..(cx1 * rw).div_ceil(vw).min(rw) {
                let p = &mut d.pending_world[yy * rw + xx];
                *p = palette[*p as usize];
            } }
        }
    }
    c.ret_near(8);
    Ok(())
}

fn hud(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let this = c.r[EAX];
    if m.display.current_view != primary_view(c, this) || m.display.mode == 0 || c.wlog_on {
        c.eip = m.display.original_hud;
        return Ok(());
    }
    // Use the existing CPU write log, not colour-keying: black HUD pixels and
    // pixels equal to the world colour are still opaque.
    let old_log = if c.wlog.is_empty() { Vec::new() } else { std::mem::take(&mut c.wlog) };
    c.wlog_on = true;
    m.display.recording_hud = true;
    let result = m.call_guest(c, m.display.original_hud, &[this, c.r[EDX], c.r[EBX]]);
    finish_overlay(&mut m.display, c, this, old_log);
    result?;
    c.ret_near(0);
    Ok(())
}

/// Enter after PCDeath's final world frame, before any DOS-only column shifts.
fn death_melt(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    if m.display.mode != 0 && m.display.presented && !c.wlog_on {
        let mut frame = Vec::new();
        let size = render(m,c,&mut frame);
        m.display.death = Some(DeathMelt { frame, size,
            backdrop: c.read_bytes(c.r[x86::EBP]+0xE5920,640*480).to_vec(), columns: [7;640], melting: true });
    }
    // Replay the displaced LEA; no flags or original death/game logic change.
    c.r[EAX] = c.r[x86::EBP]+0xE5920;
    c.eip = rt(0x01545F);
    Ok(())
}

/// PCDeath's hot column loop (0x01551A..0x0154DA). Keep its random picks,
/// eight-row displacement, DOS framebuffer and stack cursors exactly intact.
fn death_column(_m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let sp = c.r[x86::ESP];
    let x = c.r[x86::ECX];
    let cursor = c.r[EAX]; // caller already checked cursor < 480
    let bottom = c.rd32(sp+0x508)+x;
    let backdrop = c.rd32(sp+0x500)+x;
    for y in (cursor+1..480).rev() {
        let dst = bottom-(479-y)*640;
        c.wr8(dst,c.rd8(dst-8*640));
    }
    for y in (cursor-7..=cursor).rev() {
        c.wr8(bottom-(479-y)*640,c.rd8(backdrop+y*640));
    }
    c.wr16(sp+x*2,(cursor+8) as u16);
    c.r[EAX] = bottom.wrapping_sub((487-cursor)*640);
    c.r[EDX] = backdrop.wrapping_add(cursor.wrapping_sub(8).wrapping_mul(640));
    c.r[EBX] = cursor.wrapping_sub(8);
    c.r[x86::ECX] = cursor.wrapping_sub(8);
    c.r[x86::ESI] = x*2;
    c.eip = rt(0x0154F5); // original iteration count, page flip, song and Escape
    Ok(())
}

fn flip(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    if m.display.recording_menu {
        m.display.recording_menu = false;
        let old_log = std::mem::take(&mut m.display.menu_log);
        finish_overlay(&mut m.display, c, rt(0x09E5DC), old_log);
    }
    if c.rd32(c.r[x86::ESP]) == rt(0x01554A) {
        if let Some(death) = &mut m.display.death {
            let cursors = c.r[x86::ESP]+4; // PCDeath's 640 stack u16 column cursors
            death.columns = std::array::from_fn(|x| c.rd16(cursors+x as u32*2));
        }
    }
    c.push32(m.display.flip_stub);
    c.eip = m.display.original_flip;
    Ok(())
}
fn publish(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let d = &mut m.display;
    d.frame_serial += 1;
    let melt = c.rd32(c.r[x86::ESP]) == rt(0x01554A) && d.death.is_some();
    if !melt {
        if d.mode != 0 && pause_menu(c) && c.rd32(rt(0x20687C)) != 0 {
            if let Some(death) = &mut d.death { death.melting = false; death.frame = Vec::new(); }
        } else { d.death = None; }
    }
    d.presented = d.pending || melt;
    if d.pending {
        // Preserve a vehicle's secondary camera as a square-pixel UI inset.
        let this = rt(0x09E5DC);
        let primary = primary_view(c, this);
        for view in 0..2 {
            let p = this + view * 36;
            if view == primary || c.rd32(p + 0x4B150) == 0 { continue; }
            let (x,y,w,h) = (c.rd32(p + 0x4B154) as usize, c.rd32(p + 0x4B158) as usize,
                c.rd32(p + 0x4B160) as usize, c.rd32(p + 0x4B164) as usize);
            for yy in y.min(480)..y.saturating_add(h).min(480) {
                for xx in x.min(640)..x.saturating_add(w).min(640) { d.pending_hud[yy * 640 + xx] = true; }
            }
        }
        std::mem::swap(&mut d.world, &mut d.pending_world);
        d.rect = d.pending_rect;
        d.world_size = d.pending_world_size;
    }
    if d.pending || (d.death.is_some() && !melt) { std::mem::swap(&mut d.hud, &mut d.pending_hud); }
    d.pending = false;
    c.ret_near(0);
    Ok(())
}

/// Palette effects act on the native world too, without turning the low-res
/// world into an opaque HUD layer during damage flashes / night vision.
fn tint(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let (this, effect, x, y) = (c.r[EAX], c.r[EDX], c.r[EBX], c.r[x86::ECX]);
    let (x2,y2) = (c.stack_arg(0), c.stack_arg(1));
    let map = c.rd32(this.wrapping_add(0xBDFD8).wrapping_add(effect.wrapping_mul(4)));
    let palette: [u8;256] = std::array::from_fn(|i| c.rd8(map + i as u32));
    let fb = this.wrapping_add(c.rd32(this + 0xE34A8));
    let log_start = c.wlog.len();
    for row in 0..y2.saturating_sub(y) {
        for col in x..x2 {
            let p = fb + row * 640 + col;
            c.wr8(p,palette[c.rd8(p) as usize]);
        }
    }
    if m.display.recording_hud && m.display.pending {
        let (vw,vh) = m.display.pending_world_size;
        let (w,h) = (c.rd32(this + 0xE34B4) as usize,c.rd32(this + 0xE34AC) as usize);
        if w > 0 && h > 0 {
            for yy in 0..((y2.saturating_sub(y) as usize * vh / h).min(vh)) {
                for xx in (x as usize * vw / w).min(vw)..(x2 as usize * vw / w).min(vw) {
                    if let Some(p) = m.display.pending_world.get_mut(yy * vw + xx) { *p = palette[*p as usize]; }
                }
            }
        }
        c.wlog.truncate(log_start);
    }
    c.ret_near(8);
    Ok(())
}

/// Uniformly scaled 4:3 UI, with native world pixels replacing the view only.
pub fn render(m: &mut Machine, c: &Cpu, out: &mut Vec<u32>) -> (usize, usize) {
    prepare_menu(m, c);
    if m.display.help_open { return present_help(&m.display,out); }
    if m.display.options_open {
        let mut page = Vec::new();
        menu(&m.display, &mut page);
        if m.desktop.controller_guide {m.display.art.cursor(&mut page,m.display.ui_pointer.0,m.display.ui_pointer.1);}
        return present_menu(&m.display, &page, out);
    }
    let backscreen = c.rd32(rt(0x09E5DC) + BACKSCREEN);
    let d = &mut m.display;
    let size = d.size();
    let native = d.mode != 0 && d.presented && m.vga.mode == 0x101
        && d.world_size.0 > 0 && d.world_size.1 > 0
        && d.world.len() == d.world_size.0 * d.world_size.1;
    let mask = if native && d.rect == (0,0,size.0,size.1) { d.hud.as_slice() } else { &[] };
    let source = m.vga.render_masked(c, &mut d.legacy_rgb, mask);
    if d.options_original && source == (640, 480) && !d.art.background.is_empty() {
        d.art.header(&mut d.legacy_rgb, 0, "GAME OPTIONS", "ARROWS / DPAD SELECT   ENTER / A CHANGE   ESC / B BACK");
        d.art.text(&mut d.legacy_rgb,46,146,"KEYBOARD / MOUSE + XBOX 360",125);
        d.art.text(&mut d.legacy_rgb,46,168,"F1 GUIDES  F4 / BACK ACTIONS",125);
        if m.desktop.controller_guide { d.art.cursor(&mut d.legacy_rgb,d.ui_pointer.0,d.ui_pointer.1); }
    }
    let pal: [u32; 256] = std::array::from_fn(|i| {
        let c = m.vga.dac[i];
        let e = |v: u8| ((v as u32) << 2) | ((v as u32) >> 4);
        (e(c[0]) << 16) | (e(c[1]) << 8) | e(c[2])
    });
    if let Some(death) = &d.death {
        if death.melting { return death.render(&pal,out); }
        // Death menus have no 3D view. Keep the tunnel full-width while the
        // recorded buttons, shade boxes and glove retain square-pixel scaling.
        let (w,h) = size;
        let rows = h*640/w;
        let crop = (480-rows)/2;
        compose(&d.legacy_rgb, source, size, &death.backdrop[crop*640..(crop+rows)*640],
            (640,rows), &d.hud, (0,0,w,h), true, &pal, None, out);
        return size;
    }
    // Menus keep the field but never present the native world, so they get no side art.
    let sides = if native { vehicle_sides(backscreen) } else { None };
    compose(&d.legacy_rgb, source, size, &d.world, d.world_size, &d.hud, d.rect, native, &pal, sides, out);
    size
}

/// `Xox+0x1444F0`: backscreen loaded for gameplay (Xox::LoadBackScreen argument).
const BACKSCREEN: u32 = 0x1444F0;
const TANK_BACKSCREEN: u32 = 2;
const CAR_BACKSCREEN: u32 = 3;
/// Side art width in UI columns: the widest (32:9) margin.
pub const SIDE_W: usize = 534;
/// Side-art pixel showing the native world (aircar side windows).
const CLEAR: u32 = 0xFF00_0000;

/// Generated cockpit extensions for widescreen margins (tools/prepare_vehicle_sides.py).
pub struct SideArt { left: Vec<u32>, right: Vec<u32> }

fn side_art(rgba: &[u8]) -> SideArt {
    let pixels: Vec<u32> = rgba.chunks_exact(4).map(|p| if p[3] == 0 { CLEAR }
        else { ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32 }).collect();
    let (left, right) = pixels.split_at(SIDE_W * 480);
    SideArt { left: left.to_vec(), right: right.to_vec() }
}

pub fn vehicle_sides(backscreen: u32) -> Option<&'static SideArt> {
    static TANK: std::sync::OnceLock<SideArt> = std::sync::OnceLock::new();
    static CAR: std::sync::OnceLock<SideArt> = std::sync::OnceLock::new();
    match backscreen {
        TANK_BACKSCREEN => Some(TANK.get_or_init(|| side_art(include_bytes!("../../../assets/tank-sides.rgba")))),
        CAR_BACKSCREEN => Some(CAR.get_or_init(|| side_art(include_bytes!("../../../assets/car-sides.rgba")))),
        _ => None,
    }
}

/// The aircar view spans the full legacy width; its side art has windows, so the
/// world continues behind them. Other partial views stay inside their 4:3 frames.
pub fn view_widens((x, _, w, h): (usize, usize, usize, usize), backscreen: u32) -> bool {
    x == 0 && w == 640 && (h == 480 || backscreen == CAR_BACKSCREEN)
}

fn compose(legacy: &[u32], (sw, sh): (usize, usize), (w, h): (usize, usize),
    world: &[u8], (rw,rh): (usize,usize), hud: &[bool], (vx, vy, vw, vh): (usize, usize, usize, usize),
    native: bool, pal: &[u32;256], sides: Option<&SideArt>, out: &mut Vec<u32>) {
    let (uw, uh, ox, oy) = ui_rect(w, h, sw, sh);
    // Divide per source column/row, not per output pixel. Do not scale the
    // low-resolution world only to overwrite it with the native world again.
    let ui_edges: Vec<usize> = (0..=sw).map(|x| ox + (x * uw).div_ceil(sw)).collect();
    // Side-art column of each margin pixel, counted outward from the UI edges.
    let sides = sides.filter(|_| (sw, sh) == (640, 480));
    let side_cols: Vec<usize> = if sides.is_none() { Vec::new() } else {
        (0..w).map(|x| if x < ox { SIDE_W.wrapping_sub(((ox - x) * sw).div_ceil(uw)) }
            else if x >= ox + uw { (x - ox - uw) * sw / uw } else { usize::MAX }).collect()
    };
    // Expand each source scanline once, then copy it for repeated output rows.
    let generic_scale = native && rw != vw && !(rw%2 == 0 && vw == rw*3/2) && vw%rw != 0;
    let world_cols: Vec<usize> = if generic_scale { (0..vw).map(|x| x*rw/vw).collect() } else { Vec::new() };
    let mut expanded = vec![0; if native {vw} else {0}];
    let mut last_world_row = usize::MAX;
    let mut last_sources = None;
    let mut ui_expanded = vec![0; uw];
    let mut ui_spans: Vec<(usize,usize)> = Vec::new();
    let mut last_ui = None;
    out.resize(w * h, 0);
    for y in 0..h {
        let world_row = native && (vy..vy.saturating_add(vh)).contains(&y) && vx < w;
        let ui_row = (oy..oy + uh).contains(&y);
        let sy = if world_row { (y - vy) * rh / vh } else { usize::MAX };
        let base = if ui_row { (y - oy) * sh / uh * sw } else { usize::MAX };
        let sources = (sy,base);
        if last_sources == Some(sources) {
            let (previous,next) = out.split_at_mut(y * w);
            next[..w].copy_from_slice(&previous[(y-1) * w..y * w]);
            continue;
        }
        last_sources = Some(sources);
        let row = &mut out[y * w..(y+1) * w];
        let right = vx.saturating_add(vw).min(w);
        // Only clear uncovered margins. Every world/UI pixel is overwritten below.
        if world_row { row[..vx].fill(0); row[right..].fill(0); }
        else if ui_row { row[..ox].fill(0); row[ox+uw..].fill(0); }
        else { row.fill(0); }
        if world_row {
            if sy != last_world_row {
                let src = &world[sy * rw..(sy + 1) * rw];
                if rw == vw {
                    for (dst,&p) in expanded.iter_mut().zip(src) { *dst = pal[p as usize]; }
                } else if rw%2 == 0 && vw == rw*3/2 {
                    for (dst,p) in expanded.chunks_exact_mut(3).zip(src.chunks_exact(2)) {
                        let a = pal[p[0] as usize];
                        dst[0]=a; dst[1]=a; dst[2]=pal[p[1] as usize];
                    }
                } else if vw%rw == 0 {
                    for (dst,&p) in expanded.chunks_exact_mut(vw/rw).zip(src) { dst.fill(pal[p as usize]); }
                } else {
                    for (dst,&x) in expanded.iter_mut().zip(&world_cols) { *dst = pal[src[x] as usize]; }
                }
                last_world_row = sy;
            }
            row[vx..right].copy_from_slice(&expanded[..right-vx]);
        }
        if ui_row {
            if last_ui != Some((base,world_row)) {
                last_ui = Some((base,world_row));
                ui_spans.clear();
                // Expand each UI source row once, independent of world-row changes.
                // Merge adjacent opaque pixels; repeated rows copy only those spans.
                let mut paint = |a: usize, b: usize, color| {
                    if a >= b { return; }
                    ui_expanded[a-ox..b-ox].fill(color);
                    if let Some(last) = ui_spans.last_mut().filter(|last| last.1 == a) { last.1 = b; }
                    else { ui_spans.push((a,b)); }
                };
                for (sx, &color) in legacy[base..base + sw].iter().enumerate() {
                    if !world_row || hud[base + sx] {
                        paint(ui_edges[sx],ui_edges[sx+1],color);
                    } else if vx != 0 || right != w {
                        let (a,b) = (ui_edges[sx],ui_edges[sx+1]);
                        if a < vx { paint(a,b.min(vx),color); }
                        if b > right { paint(a.max(right),b,color); }
                    }
                }
            }
            for &(a,b) in &ui_spans { row[a..b].copy_from_slice(&ui_expanded[a-ox..b-ox]); }
            if let Some(art) = sides {
                let base = (y - oy) * sh / uh * SIDE_W;
                let (art_l, art_r) = (&art.left[base..base + SIDE_W], &art.right[base..base + SIDE_W]);
                let margins = (0..ox).map(|x| (x, art_l)).chain((ox + uw..w).map(|x| (x, art_r)));
                for (x, src) in margins {
                    match src.get(side_cols[x]) {
                        Some(&p) if p != CLEAR => row[x] = p,
                        Some(_) if world_row && (vx..right).contains(&x) => {}
                        _ => row[x] = 0,
                    }
                }
            }
        }
    }
}

/// Output rectangle of the native world for a guest view (640x480 coordinates).
/// Views follow the 4:3 UI scaling: the tank (480x256) and shrunk views are
/// framed by its artwork. Only widening views (`view_widens`) extend
/// horizontally to the output edges.
pub fn world_rect((x, y, w, h): (usize, usize, usize, usize), (ow, oh): (usize, usize), wide: bool) -> (usize, usize, usize, usize) {
    let (uw, uh, ox, oy) = ui_rect(ow, oh, 640, 480);
    // Same column/row boundaries as compose's UI scaling.
    let edge = |a: usize, full: usize, o: usize, u: usize| o + (a.min(full) * u).div_ceil(full);
    let (x0, x1) = if wide { (0, ow) } else { (edge(x, 640, ox, uw), edge(x + w, 640, ox, uw)) };
    let (y0, y1) = (edge(y, 480, oy, uh), edge(y + h, 480, oy, uh));
    (x0, y0, ((x1 - x0) & !1).max(2), (y1 - y0).max(2))
}

pub fn ui_rect(w: usize, h: usize, sw: usize, sh: usize) -> (usize, usize, usize, usize) {
    let scale = (w as f64 / sw as f64).min(h as f64 / sh as f64);
    let uw = (sw as f64 * scale).round() as usize;
    let uh = (sh as f64 * scale).round() as usize;
    (uw, uh, (w - uw) / 2, (h - uh) / 2)
}

pub fn menu(s: &Display, out: &mut Vec<u32>) -> (usize, usize) {
    let art = &s.art;
    art.page(out, 1, "DISPLAY AND INPUT", "ARROWS/DPAD CHANGE  ENTER/A OK  CLICK/X SELECT  ESC/B BACK");
    let (w, h) = s.size();
    let rows = [("RESOLUTION", format!("{w}X{h}")), ("LOOK SPEED", format!("{:.2}", s.sensitivity)),
        ("STICK DEADZONE", format!("{:.3}", s.deadzone)),
        ("INVERT LOOK", if s.invert_y { "ON" } else { "OFF" }.into()),
        ("FULLSCREEN", if s.fullscreen { "ON" } else { "OFF" }.into()),
        (if s.options_original { "BACK TO GAME OPTIONS" } else { "BACK TO GAME" }, String::new())];
    for (i, (label, value)) in rows.iter().enumerate() {
        let y = 146 + i * 32;
        art.fill(out, 14, y + 24, 610, 1, 0x78787C);
        art.label(out, 46, y, label, i == s.row);
        art.text(out, 364, y, value, 175);
    }
    for (i, line) in ["WASD MOVE  MOUSE LOOK  SPACE JUMP  E USE", "SHIFT RUN  CTRL CROUCH  ESC MENU",
        "F1 GUIDES   F4 / BACK ACTIONS", "Q/E OR LT/RT OPTIONS PAGES"].iter().enumerate() {
        art.text(out, 46, 350 + i * 28, line, 125);
    }
    (640, 480)
}

/// The original Options bitmap and font 3, not a new approximation of their style.
pub struct MenuArt {
    background: Vec<u32>,
    font: Vec<u8>,
    height: usize,
    stride: usize,
    spacing: i16,
    palette: [u32; 256],
}
impl Default for MenuArt {
    fn default() -> Self {
        Self { background: Vec::new(), font: Vec::new(), height: 0, stride: 0, spacing: 0, palette: [0; 256] }
    }
}

pub fn prepare_menu(m: &mut Machine, c: &Cpu) {
    if !m.display.art.background.is_empty() { return; }
    let this = rt(0x09E5DC);
    let table = rt(0x20732F) + 3 * 32;
    let font = c.rd32(table);
    let (height, stride) = (c.rd16(table + 20) as usize, c.rd16(table + 30) as usize);
    if font == 0 || height == 0 || height > 64 || stride < 2 || stride > 4096 { return; }
    let handle = c.rd32(this + 0xE3538) as usize;
    let Some(file) = m.files.get(handle).and_then(Option::as_ref) else { return; };
    let Some(path) = m.vfs.resolve_read(&file.name) else { return; };
    let read = || -> std::io::Result<(Vec<u8>, [u8; 768])> {
        // Reopen read-only: never disturb the guest's shared DOS file position.
        let mut f = std::fs::File::open(path)?;
        let mut background = vec![0; 640 * 480];
        f.seek(SeekFrom::Start(c.rd32(this + 0xE541C) as u64))?;
        f.read_exact(&mut background)?;
        let mut palette = [0; 768];
        f.seek(SeekFrom::Start(c.rd32(this + 0xE5348 + 31 * 4) as u64))?;
        f.read_exact(&mut palette)?;
        Ok((background, palette))
    };
    match read() {
        Ok((mut background, palette)) => {
            crate::rebrand::stamp_menu(&mut background, c);
            let palette: [u32; 256] = std::array::from_fn(|i| {
                let rgb = &palette[i * 3..i * 3 + 3];
                let expand = |v: u8| ((v as u32) << 2) | ((v as u32) >> 4);
                (expand(rgb[0]) << 16) | (expand(rgb[1]) << 8) | expand(rgb[2])
            });
            m.display.art = MenuArt { background: background.iter().map(|&p| palette[p as usize]).collect(),
                font: c.read_bytes(font + 14, stride * 256).to_vec(), height, stride,
                spacing: c.rd16(table + 26) as i16, palette };
        }
        Err(e) => eprintln!("cannot load original menu artwork: {e}"),
    }
}

impl MenuArt {
    pub fn ready(&self) -> bool { self.background.len() == 640 * 480 }
    pub fn fill(&self, out: &mut [u32], x: usize, y: usize, w: usize, h: usize, color: u32) {
        for yy in y..(y + h).min(480) { out[yy * 640 + x..yy * 640 + (x + w).min(640)].fill(color); }
    }
    pub fn text(&self, out: &mut [u32], mut x: usize, y: usize, text: &str, color: usize) {
        if self.stride < 2 { return; }
        for ch in text.to_ascii_uppercase().bytes() {
            let start = ch as usize * self.stride;
            let Some(glyph) = self.font.get(start..start + self.stride) else { return; };
            let w = u16::from_le_bytes([glyph[0], glyph[1]]) as usize;
            if w * self.height + 2 > glyph.len() { continue; }
            for yy in 0..self.height { for xx in 0..w {
                if glyph[2 + yy * w + xx] < 0xFE && x + xx < 640 && y + yy < 480 {
                    out[(y + yy) * 640 + x + xx] = self.palette[color];
                }
            } }
            x = (x as isize + w as isize + self.spacing as isize).max(0) as usize;
        }
    }
    pub fn label(&self, out: &mut [u32], x: usize, y: usize, text: &str, selected: bool) {
        if selected && self.stride >= 2 {
            let width: isize = text.to_ascii_uppercase().bytes().map(|ch| {
                let p = ch as usize * self.stride;
                u16::from_le_bytes([self.font[p],self.font[p+1]]) as isize + self.spacing as isize
            }).sum();
            self.fill(out, x - 2, y - 1, width.max(0) as usize + 4, self.height + 2, self.palette[117]);
        }
        self.text(out, x, y, text, if selected { 255 } else { 125 });
    }
    pub fn header(&self, out: &mut [u32], tab: usize, title: &str, hint: &str) {
        // Exactly the instruction panel's black interior; its bevel starts at x=216.
        self.fill(out, 232, 18, 392, 104, 0x080808);
        for (i, label) in MENU_TABS.iter().enumerate() {
            self.label(out, 238 + i * TAB_WIDTH, 34, label, i == tab);
        }
        self.text(out, 238, 62, title, 175);
        self.text(out, 238, 91, "Q / E OR LT / RT TABS", 125);
        // The old instruction box is too narrow for the full navigation hint.
        self.fill(out, 14, 453, 610, 12, 0x080808);
        self.text(out, 24, 453, hint, 125);
    }
    pub fn cursor(&self, out: &mut [u32], x: i32, y: i32) {
        let (x,y) = (x.clamp(0,633) as usize,y.clamp(0,473) as usize);
        self.fill(out,x,y,7,7,0);
        self.fill(out,x+3,y+1,1,5,self.palette[255]);
        self.fill(out,x+1,y+3,5,1,self.palette[117]);
    }
    pub fn page(&self, out: &mut Vec<u32>, tab: usize, title: &str, hint: &str) {
        out.resize(640 * 480, 0);
        if self.background.len() == out.len() { out.copy_from_slice(&self.background); }
        else { out.fill(0); }
        self.fill(out, 14, 136, 610, 330, 0x080808);
        self.header(out, tab, title, hint);
    }
}

/// All host pages use the selected output resolution, with a square-pixel UI.
pub fn present_menu(d: &Display, page: &[u32], out: &mut Vec<u32>) -> (usize, usize) {
    let size = d.size();
    compose(page, (640, 480), size, &[], (0,0), &[], (0, 0, 0, 0), false, &[0; 256], None, out);
    size
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tint_and_black_hud_pixels_are_not_stretched_world_overlays() {
        let dir = std::env::temp_dir().join(format!("cybermage-display-{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&dir).unwrap();
        let mut m = Machine::new(crate::vfs::Vfs::new(dir.join("iso"), dir.join("run")));
        let mut c = Cpu::new();
        let (this, map, fb): (u32,u32,u32) = (0x100000,0x20000,0x30000);
        c.wr32(this + 0xBDFD8,map);
        for i in 0..256 { c.wr8(map+i,(i as u8).wrapping_add(1)); }
        c.wr32(this + 0xE34A8,fb.wrapping_sub(this));
        c.wr32(this + 0xE34B4,2); c.wr32(this + 0xE34AC,2);
        c.r[x86::ESP] = 0x50000;
        c.push32(2); c.push32(2); c.push32(0x1234);
        c.r[EAX] = this;
        m.display.pending = true; m.display.recording_hud = true;
        m.display.pending_rect = (0,0,4,2);
        m.display.pending_world_size = (4,2);
        m.display.pending_world = vec![0,1,2,3,4,5,6,7];
        c.wlog_on = true;
        tint(&mut m,&mut c).unwrap();
        assert_eq!(m.display.pending_world,vec![1,2,3,4,5,6,7,8]);
        assert_eq!(c.rd8(fb),1);
        assert!(c.wlog.is_empty());
        assert_eq!(c.eip,0x1234);
        c.wlog_on = false;
        m.display.recording_hud = false;
        // A HUD write of black over an already-black background must be opaque.
        c.write_bytes(crate::machine::STUB_BASE + 0x800,&[0x0F,0xFF,0,1]);
        m.display.original_hud = 0x210000;
        c.write_bytes(0x210000,&[0xC6,0x00,0x00,0xC3]); // mov byte [eax],0; ret
        c.push32(0x1234);
        c.r[EAX] = this;
        hud(&mut m,&mut c).unwrap();
        assert!(m.display.pending_hud[0]);
        assert_eq!(m.display.pending_hud.iter().filter(|&&b| b).count(),1);
        assert_eq!(c.eip,0x1234);
        c.wlog.reserve(8192);
        m.hook_sig(&mut c,0x10000,"test reused HUD log",1,false,|_,c| {assert!(c.wlog.capacity()>=8192);c.wr8(c.r[EAX],0);c.ret_near(0);Ok(())});
        m.display.original_hud=rt(0x10000); c.r[EAX]=this; c.push32(0x1234); hud(&mut m,&mut c).unwrap();
        assert!(c.wlog.is_empty()); assert!(c.wlog.capacity()>=8192);
        m.display.original_hud=0x210000; c.wlog.push((0x123456,7)); c.r[EAX]=this; c.push32(0x1234); hud(&mut m,&mut c).unwrap();
        assert_eq!(c.wlog,[(0x123456,7)]); // preserve a nonempty caller's log
        c.wlog.clear(); c.push32(0x1234); menu_overlay(&mut m,&mut c).unwrap();
        assert!(c.wlog_on); assert!(c.wlog.capacity()>=8192); c.wr8(this+1,0);
        let old_log=std::mem::take(&mut m.display.menu_log); finish_overlay(&mut m.display,&mut c,this,old_log);
        assert!(m.display.pending_hud[1]); assert!(!m.display.pending_hud[0]); assert!(c.wlog.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn death_columns_keep_native_pixels_and_menu_exit_releases_the_snapshot() {
        let pal = std::array::from_fn(|i| i as u32+0x550000);
        let backdrop: Vec<u8> = (0..640*480).map(|i| (i/640) as u8).collect();
        for (w,h) in [(640,480),(1920,1080),(3840,1080),(3840,2160)] {
            let mut death = DeathMelt { size: (w,h), frame: (0..w*h).map(|i| i as u32+0x1000000).collect(),
                backdrop: backdrop.clone(), columns: [7;640], melting: true };
            let mut out = Vec::new();
            assert_eq!(death.render(&pal,&mut out),(w,h));
            assert_eq!(out,death.frame); // unshifted native FOV, including margins
            death.columns = std::array::from_fn(|x| [7,15,247,487][x%4]);
            death.render(&pal,&mut out);
            for x in 0..w { for y in [0,h/3,h-1] {
                let sx = x*640/w;
                let shift = [0,8,240,480][sx%4]*h/480;
                let expected = if y < shift {pal[((480-h*640/w)/2+y*640/w) as u8 as usize]}
                    else {death.frame[(y-shift)*w+x]};
                assert_eq!(out[y*w+x],expected);
            } }
            death.columns.fill(487);
            death.render(&pal,&mut out);
            assert!(out.iter().all(|&p| (0x550000..0x550100).contains(&p)));
        }
        let mut m = Machine::new(crate::vfs::Vfs::new(Path::new("__death_test__").into(),Path::new("__death_test__").into()));
        let mut c = Cpu::new();
        m.display.death = Some(DeathMelt {size: (640,480), frame: vec![1], backdrop, columns: [487;640], melting: true});
        c.r[x86::ESP] = 0x30000;
        c.wr32(rt(0x09E5DC)+0xBDEBC,MENU_VIEW); c.wr32(rt(0x20687C),1);
        c.push32(rt(0x04F1B6)); publish(&mut m,&mut c).unwrap();
        assert!(m.display.death.as_ref().is_some_and(|d| !d.melting && d.frame.is_empty()));
        c.wr32(rt(0x20687C),0);
        c.push32(rt(0x04F1B6)); publish(&mut m,&mut c).unwrap();
        assert!(m.display.death.is_none());
        m.display.mode = 0; m.display.presented = true;
        death_melt(&mut m,&mut c).unwrap(); // 640x480 retains the original DOS effect
        assert!(m.display.death.is_none());
    }
    #[test]
    #[ignore = "requires original game data; compares the native melt loop to x86"]
    fn death_column_matches_original_pixels_cursors_and_registers() {
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        let exe = std::fs::read(project.join("iso/CYBERMAG/CM_HI.EXE")).unwrap();
        let mut c = Cpu::new();
        for (addr,bytes) in crate::le::load(&exe).unwrap().data { c.write_bytes(addr,&bytes); }
        let path = std::path::PathBuf::from("__death_column_no_files__");
        let mut m = Machine::new(crate::vfs::Vfs::new(path.clone(),path));
        let (fb,bg,sp) = (0x100000,0x200000,0x30000);
        let frame: Vec<u8> = (0..640*480).map(|i| (i*13+i/640) as u8).collect();
        let backdrop: Vec<u8> = (0..640*480).map(|i| (i*7+i/640) as u8).collect();
        c.write_bytes(bg,&backdrop);
        let mut native_time = std::time::Duration::ZERO;
        let mut guest_time = std::time::Duration::ZERO;
        for cursor in (7..480).step_by(8) { for x in [0,317,639] {
            c.write_bytes(fb,&frame); c.r[x86::ESP] = sp;
            c.wr32(sp+0x500,bg); c.wr32(sp+0x508,fb+479*640); c.wr16(sp+x*2,cursor as u16);
            c.r[EAX] = cursor; c.r[x86::ECX] = x; c.r[EDX] = x;
            let input = c.r;
            let start = std::time::Instant::now();
            death_column(&mut m,&mut c).unwrap();
            native_time += start.elapsed();
            let expected = c.read_bytes(fb,640*480).to_vec();
            let registers = c.r;
            assert_eq!(c.rd16(sp+x*2),cursor as u16+8);
            c.write_bytes(fb,&frame); c.wr16(sp+x*2,cursor as u16); c.r = input;
            c.eip = rt(0x01551A);
            let start = std::time::Instant::now();
            while c.eip != rt(0x0154F5) { c.step(&mut m).unwrap(); }
            guest_time += start.elapsed();
            assert_eq!(c.read_bytes(fb,640*480),expected,"cursor {cursor}, column {x}");
            assert_eq!(c.rd16(sp+x*2),cursor as u16+8);
            assert_eq!(c.r,registers,"cursor {cursor}, column {x}");
        } }
        eprintln!("death columns: x86 {guest_time:?}, native {native_time:?}");
    }
    #[test]
    #[ignore = "requires original game data; isolated widescreen death sequence"]
    fn death_melt_keeps_widescreen_and_returns_to_native_menu() {
        use crate::{gamepad, Emu};
        use std::time::{Duration, Instant};
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        let root = std::env::temp_dir().join(format!("cybermage-death-{}",std::process::id()));
        let run = root.join("run");
        let game = project.join("iso/CYBERMAG");
        let mut c = Cpu::new();
        let mut m = Machine::new(crate::vfs::Vfs::new(game.clone(),run.clone()));
        crate::write_default_config(&run);
        std::fs::copy(project.join("run/CMDATA.CNF"),run.join("CMDATA.CNF")).unwrap();
        m.display.mode = parse_mode("1920x1080").unwrap();
        m.load_program(&mut c,&std::fs::read(game.join("CM_HI.EXE")).unwrap(),"").unwrap();
        crate::native::install(&mut m,&mut c);
        m.gamepad.connected = true;
        let mut emu = Emu { cpu: c, m, ips: 100_000_000.0, cfg_trace: false,
            watches: Vec::new(), profile: None, counter: None, profile_start: 0.0,
            scene_override: None, vehicle_anywhere: None, realtime_last: None };
        for frame in 0..280 {
            if frame == 180 {gamepad::tap(&mut emu.m.kbd_queue,0x1C,false);}
            gamepad::controller_input(&emu.m,&mut emu.cpu);
            match emu.run_for(0.05) {
                Ok(()) => {},
                Err(Fault::Stop) if emu.m.spawn_request.take().is_some() => {emu.cpu.set_cf(false); emu.cpu.r[EAX] = 0;},
                Err(e) => panic!("{}",emu.crash_report(e)),
            }
        }
        assert!(emu.m.display.presented);
        emu.cpu.wr32(rt(0x20687C),1); // original HandleNextFrame enters PCDeath
        let deadline = Instant::now()+Duration::from_secs(20);
        let mut melts = 0;
        while Instant::now() < deadline {
            emu.run_frame(Instant::now()+Duration::from_millis(100)).unwrap();
            if emu.cpu.eip == rt(0x01554A) {
                let path = project.join(format!("out/death-check/melt-{melts}.bmp"));
                if melts < 3 {emu.screenshot(&path);}
                assert!(emu.m.display.presented,"death melt reverted to letterboxed DOS rendering");
                melts += 1;
                if melts == 10 {gamepad::tap(&mut emu.m.kbd_queue,0x01,false);}
            }
            if pause_menu(&emu.cpu) && melts > 10 {break;}
        }
        assert!(melts >= 10,"did not exercise original death melt");
        assert!(pause_menu(&emu.cpu),"death did not reach original menu");
        emu.screenshot(&project.join("out/death-check/menu.bmp"));
        assert!(emu.m.display.death.as_ref().is_some_and(|d| !d.melting));
        for (x,y) in [(200,286),(440,286),(200,370),(440,370)] {assert!(emu.m.display.hud[y*640+x],"death menu button lost");}
        // Nested Load uses a separate background call: its mask must replace
        // the death menu's four-button mask, not alternate stale masks each flip.
        gamepad::tap(&mut emu.m.kbd_queue,0x26,false); // original Load shortcut
        let deadline = Instant::now()+Duration::from_secs(10);
        let mut loads = 0;
        while Instant::now() < deadline && loads < 4 {
            emu.run_frame(Instant::now()+Duration::from_millis(100)).unwrap();
            if emu.cpu.eip != rt(0x050006) { continue; }
            loads += 1;
            assert!(emu.m.display.death.as_ref().is_some_and(|d| !d.melting));
            for row in 0..11 {
                let x = emu.cpu.rd32(rt(0x095D98)+row*8) as usize;
                let y = emu.cpu.rd32(rt(0x095DF0)+row*8) as usize;
                assert!(emu.m.display.hud[y*640+x],"load row {row} lost on frame {loads}");
            }
            if loads == 1 { emu.screenshot(&project.join("out/death-check/load.bmp")); }
        }
        assert_eq!(loads,4,"death menu did not open Load");
        gamepad::tap(&mut emu.m.kbd_queue,0x01,false);
        let deadline = Instant::now()+Duration::from_secs(10);
        loop {
            emu.run_frame(Instant::now()+Duration::from_millis(100)).unwrap();
            if emu.cpu.eip == rt(0x04F1B6) { break; }
            assert!(Instant::now() < deadline,"Load cancel did not return to death menu");
        }
        for (x,y) in [(200,286),(440,286),(200,370),(440,370)] {assert!(emu.m.display.hud[y*640+x],"death menu button lost after Load cancel");}
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn pause_menu_boxes_shade_the_native_world_and_stay_out_of_the_overlay_mask() {
        let dir = std::env::temp_dir().join(format!("cybermage-menu-shade-{}",std::process::id()));
        let mut m = Machine::new(crate::vfs::Vfs::new(dir.join("iso"), dir.join("run")));
        let mut c = Cpu::new();
        let (this, map): (u32,u32) = (0x100000,0x20000);
        c.wr32(this + 0xBDED8 + 5 * 4, map); // SHADE_MAPS[5]
        for i in 0..256 { c.wr8(map + i, (i as u8).wrapping_add(1)); }
        c.r[x86::ESP] = 0x50000;
        let call = |m: &mut Machine, c: &mut Cpu| {
            c.push32(5); c.push32(2); c.push32(0x1234); // shade, y2, return
            c.r[EAX] = this; c.r[EDX] = 2; c.r[EBX] = 1; c.r[x86::ECX] = 3;
            shade_menu_box(m, c).unwrap();
            assert_eq!((c.eip, c.r[x86::ESP]), (0x1234, 0x50000));
        };
        call(&mut m, &mut c); // inclusive box (2,1)-(3,2), as the original
        for y in 0..4 { for x in 0..5 {
            assert_eq!(c.rd8(this + y * 640 + x), u8::from((2..=3).contains(&x) && (1..=2).contains(&y)));
        } }
        // Recording a pause menu frame: the same shade applies to the native world.
        m.display.mode = 0; // 640x480: world and UI pixels coincide
        m.display.pending_rect = (0, 0, 640, 480);
        m.display.pending_world_size = (640,480);
        m.display.pending_world = vec![7; 640 * 480];
        m.display.pending = true;
        m.display.recording_menu = true;
        c.wlog_on = true;
        call(&mut m, &mut c);
        // Shaded pixels are world, not opaque menu UI (only the argument pushes are logged).
        assert!(!c.wlog.is_empty() && c.wlog.iter().all(|&(a, _)| a.wrapping_sub(this) >= 640 * 480));
        let w = &m.display.pending_world;
        assert_eq!((w[640 + 2], w[2 * 640 + 3], w[640 + 1], w[3 * 640 + 2]), (8, 8, 7, 7));
        assert_eq!(w.iter().filter(|&&p| p == 8).count(), 4);
        std::fs::remove_dir_all(dir).ok();
    }
    #[test]
    fn adaptive_world_keeps_output_hud_and_insets_independent() {
        let mut d = Display::new(Path::new("__adaptive_no_config__"));
        d.fixed_pixels = false; d.world_pixels = 1280 * 720; d.mode = 14;
        assert_eq!(d.size(),(3840,2160));
        assert_eq!(d.raster_size(3840,2160),(1280,720));
        assert_eq!(d.raster_size(480,256),(480,256));
        d.presented = true;
        for _ in 0..59 { d.tune_resolution(0.020); }
        assert_eq!(d.world_pixels,1280 * 720);
        d.tune_resolution(0.020);
        assert!(d.world_pixels < 1280 * 720);
        for _ in 0..6000 { d.tune_resolution(0.050); }
        assert_eq!(d.world_pixels,640 * 360);
        for _ in 0..6000 { d.tune_resolution(0.005); }
        assert_eq!(d.raster_size(3840,2160),(3840,2160));
        d.fixed_pixels = true;
        for _ in 0..60 { d.tune_resolution(0.050); }
        assert_eq!(d.raster_size(3840,2160),(3840,2160));
        let legacy: Vec<u32> = (0..640*480).map(|i| 0x100000 | i as u32).collect();
        let hud: Vec<bool> = (0..640*480).map(|i| i % 11 == 0).collect();
        let pal = std::array::from_fn(|i| i as u32 * 0x010101);
        let (w,h) = (1280,720);
        for rect in [(0,0,w,h),(200,40,800,500),(w-30,h-20,60,40)] {
            let (vx,vy,vw,vh) = rect;
            let (rw,rh) = (22,13); // non-integer scaling, including cropped insets
            let world: Vec<u8> = (0..rw*rh).map(|i| i as u8).collect();
            let mut actual = Vec::new();
            compose(&legacy,(640,480),(w,h),&world,(rw,rh),&hud,rect,true,&pal,None,&mut actual);
            let (uw,uh,ox,oy) = ui_rect(w,h,640,480);
            for y in 0..h { for x in 0..w {
                let ui = (ox..ox+uw).contains(&x) && (oy..oy+uh).contains(&y);
                let i = if ui {(y-oy)*480/uh*640+(x-ox)*640/uw} else {0};
                let inside = (vx..vx+vw).contains(&x) && (vy..vy+vh).contains(&y);
                let expected = if inside && !(ui && hud[i]) {pal[world[(y-vy)*rh/vh*rw+(x-vx)*rw/vw] as usize]}
                    else if ui {legacy[i]} else {0};
                assert_eq!(actual[y*w+x],expected,"{rect:?} ({x},{y})");
            } }
        }
    }
    #[test]
    #[ignore = "uncapped composition microbenchmark; run with --release --ignored --nocapture"]
    fn composition_frame_benchmark() {
        let legacy: Vec<u32> = (0..640*480).map(|i| i as u32 * 31).collect();
        let hud: Vec<bool> = (0..640*480).map(|i| i/640 > 425 || (i/640 > 340 && (250..390).contains(&(i%640)))).collect();
        let pal = std::array::from_fn(|i| i as u32 * 0x010101);
        for (w,h,rw,rh) in [(1920,1080,1280,720),(3840,2160,1280,720),(3840,2160,640,360)] {
            let world: Vec<u8> = (0..rw*rh).map(|i| i as u8).collect();
            let mut out = Vec::new();
            let start = std::time::Instant::now();
            for _ in 0..300 {
                compose(&legacy,(640,480),(w,h),&world,(rw,rh),&hud,(0,0,w,h),true,&pal,None,&mut out);
                std::hint::black_box(&out);
            }
            eprintln!("compose {w}x{h}, world {rw}x{rh}: {:.3} ms/frame",start.elapsed().as_secs_f64()*1000.0/300.0);
        }
    }
    #[test]
    fn scanline_composition_matches_pixel_reference() {
        let legacy: Vec<u32> = (0..640*480).map(|i| if i % 7 == 0 { 0 } else { i as u32 }).collect();
        let hud: Vec<bool> = (0..640*480).map(|i| i % 7 == 0 || i % 11 == 0).collect();
        let pal = std::array::from_fn(|i| i as u32 * 0x010101);
        for &(w,h) in &[(800,600),(1280,720),(1920,1080),(3440,1440),(3840,2160),(5120,1440),(5120,2160)] {
            for (native,rect) in [(false,(0,0,w,h)),(true,(0,0,w,h)),(true,(w/8,h/8,w*3/4,h*3/4)),(true,(w-30,h-20,60,40))] {
                let (vx,vy,vw,vh) = rect;
                let world: Vec<u8> = (0..vw*vh).map(|i| i as u8).collect();
                let mut actual = vec![0xDEADBEEF; w*h];
                compose(&legacy,(640,480),(w,h),&world,(rect.2,rect.3),&hud,rect,native,&pal,None,&mut actual);
                let (uw,uh,ox,oy) = ui_rect(w,h,640,480);
                for y in 0..h { for x in 0..w {
                    let ui = x >= ox && x < ox+uw && y >= oy && y < oy+uh;
                    let i = if ui { (y-oy)*480/uh*640+(x-ox)*640/uw } else { 0 };
                    let in_world = native && x >= vx && x < vx+vw && y >= vy && y < vy+vh;
                    let expected = if in_world && !(ui && hud[i]) { pal[world[(y-vy)*vw+x-vx] as usize] }
                        else if ui { legacy[i] } else { 0 };
                    assert_eq!(actual[y*w+x],expected,"{w}x{h} pixel ({x},{y}), native={native}");
                } }
            }
        }
    }
    #[test]
    fn controller_only_options_and_help_preserve_guest_continuations() {
        let path = std::env::temp_dir().join(format!("cybermage-pad-help-{}",std::process::id()));
        let mut m = Machine::new(crate::vfs::Vfs::new(path.join("iso"),path.join("run")));
        let mut c = Cpu::new(); c.r[x86::ESP] = 0x30000;
        for row in 0..10 {
            c.wr32(0x30020,row);
            controller_options_rows(&mut m,&mut c).unwrap();
            assert_eq!(c.r[x86::ESI],3); assert_eq!(c.r[x86::EDI],234);
            assert_eq!(c.rd32(0x30024),12);
            assert_eq!(c.rd32(0x30020),row.max(3));
            assert_eq!(c.eip,rt(0x053F11));
            c.wr32(0x30020,row);
            controller_options_select(&mut m,&mut c).unwrap();
            assert_eq!(c.r[x86::ECX],row.max(3));
            assert_eq!(c.eip,rt(0x0544F3));
            assert_eq!(c.r[x86::ESP],0x30000);
        }
        c.push32(0x123456); let regs = c.r;
        controller_help(&mut m,&mut c).unwrap();
        assert!(m.display.help_open); assert_eq!(c.eip,0x123456);
        assert_eq!(c.r[x86::ESP],regs[x86::ESP]+4);
        for i in 0..8 { if i != x86::ESP { assert_eq!(c.r[i],regs[i]); } }
        assert!(m.kbd_queue.is_empty());
        c.r[EAX] = 0x100000; c.wr32(0x100000+0xBDEC0,12);
        c.push32(rt(0x014056));
        controller_help(&mut m,&mut c).unwrap();
        assert_eq!(c.r[x86::ESP],0x30000);
        assert_eq!(c.eip,rt(0x0140D8)); // never return to the old dismissal wait
        assert_eq!(c.rd32(0x100000+0xBDEC4),13);
        let pixels = help_image(); assert_eq!(pixels.len(),1280*960);
        assert!(pixels.iter().any(|&p| p & 0x00FF00 > 0x00A000));
        let mut output = Vec::new();
        for mode in 0..MODES.len() {
            m.display.mode = mode;
            let (w,h) = present_help(&m.display,&mut output);
            assert_eq!(output.len(),w*h);
            let (uw,uh,ox,oy) = ui_rect(w,h,1280,960);
            for (x,y) in [(ox,oy),(ox+uw/2,oy+uh/2),(ox+uw-1,oy+uh-1)] {
                let sx = (x-ox)*1280/uw; let sy = (y-oy)*960/uh;
                assert_eq!(output[y*w+x],pixels[sy*1280+sx]);
            }
            assert!(output[..oy*w].iter().all(|&p| p == 0));
        }
        settings_key(&mut m.display,0x4D); assert!(m.display.help_open);
        settings_key(&mut m.display,0x01); assert!(!m.display.help_open);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn options_pages_resume_guest_stack_and_use_original_font_pixels() {
        let path = std::env::temp_dir().join(format!("cybermage-menu-{}",std::process::id()));
        let mut m = Machine::new(crate::vfs::Vfs::new(path.join("iso"),path.join("run")));
        let mut c = Cpu::new();
        c.r[x86::ESP] = 0x30000;
        c.push32(rt(0x055692)); // TitleMenu's call to Options
        c.wr32(rt(0x09E5DC)+0x14480C,5); // title player state looks like gameplay
        m.display.original_options = 0x210000;
        m.display.options_return = 0x220000;
        options(&mut m,&mut c).unwrap();
        assert_eq!(c.eip,0x210000);
        let esp = c.r[x86::ESP];
        assert!(!m.display.options_open); // DOS options, not the new page, opens first
        assert!(!m.display.options_in_game);
        assert!(settings_key(&mut m.display,0x0F));
        assert!(m.display.options_open);
        settings_key(&mut m.display,0x48); assert_eq!(m.display.row,SETTINGS_BACK);
        settings_key(&mut m.display,0x50); assert_eq!(m.display.row,0);
        settings_key(&mut m.display,0x50);
        settings_key(&mut m.display,0x4D); assert_eq!(m.display.sensitivity,1.25);
        settings_key(&mut m.display,0x01);
        assert!(!m.display.options_open);
        assert!(m.display.options_original);
        assert_eq!(c.r[x86::ESP],esp); // closing a host page never pops a guest frame
        c.ret_near(0); assert_eq!(c.eip,0x220000);
        options_return(&mut m,&mut c).unwrap();
        assert!(!m.display.options_original);
        assert_eq!(c.eip,rt(0x055692));
        c.push32(rt(0x04F1E2));
        options(&mut m,&mut c).unwrap();
        assert!(m.display.options_in_game);
        c.ret_near(0);
        options_return(&mut m,&mut c).unwrap();
        assert!(!m.display.options_in_game);
        assert_eq!(c.eip,rt(0x04F1E2));
        let mut art = MenuArt { height: 2, stride: 6, spacing: -1,
            font: vec![0; 256*6], ..MenuArt::default() };
        art.palette[125] = 0x00FF00;
        art.font[b'A' as usize*6..b'A' as usize*6+6].copy_from_slice(&[2,0,1,0xFE,0xFF,1]);
        let mut pixels = vec![0x123456; 640*480];
        art.text(&mut pixels,10,10,"A",125);
        assert_eq!(pixels[10*640+10],0x00FF00);
        assert_eq!(pixels[10*640+11],0x123456);
        assert_eq!(pixels[11*640+10],0x123456);
        assert_eq!(pixels[11*640+11],0x00FF00);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn startup_viewport_is_maximized_and_fullscreen_settings_round_trip() {
        let path = std::env::temp_dir().join(format!("cybermage-window-{}",std::process::id()));
        let mut m = Machine::new(crate::vfs::Vfs::new(path.join("iso"),path.join("run")));
        let mut c = Cpu::new();
        c.write_bytes(rt(0x01025B),&[0xBE,2,0,0,0]);
        c.write_bytes(rt(0x0102AA),&[0x89,0x35]); c.wr32(rt(0x0102AC),rt(0x1E3CF4));
        install_default_viewport(&mut c);
        c.eip = rt(0x01025B); c.step(&mut m).unwrap();
        c.eip = rt(0x0102AA); c.step(&mut m).unwrap();
        assert_eq!(c.rd32(rt(0x1E3CF4)),0);
        assert!(!m.display.fullscreen);
        m.display.options_open = true;
        m.display.row = 4;
        settings_key(&mut m.display,0x1C);
        assert!(m.display.fullscreen && m.display.options_open);
        let file = path.join("run/NATIVE.CNF");
        m.display.save(&file).unwrap();
        let loaded = Display::new(&file);
        assert!(loaded.fullscreen);
        assert_eq!(loaded.size(),m.display.size());
        settings_key(&mut m.display,0x4B);
        assert!(!m.display.fullscreen);
        settings_key(&mut m.display,0x50);
        assert_eq!(m.display.row,SETTINGS_BACK);
        settings_key(&mut m.display,0x1C);
        assert!(!m.display.options_open);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn saves_get_local_timestamps_without_text_entry_or_other_slot_changes() {
        assert_eq!(format_save_name([2024,2,29,7,5,9]),"2024-02-29 07:05:09");
        assert_eq!(MENU_TABS,["GAME","DISPLAY","ACTIONS"]);
        let path = std::env::temp_dir().join(format!("cybermage-save-name-{}",std::process::id()));
        let mut m = Machine::new(crate::vfs::Vfs::new(path.join("iso"),path.join("run")));
        let mut c = Cpu::new();
        c.r[x86::ESP] = 0x1000;
        let regs = c.r;
        let table = rt(0x1E3297);
        let old = [0xA5;24*10+8];
        c.write_bytes(table,&old);
        c.wr32(0x106C,4); // fourth manual slot, one-based in FileMenu
        automatic_save_name(&mut m,&mut c).unwrap();
        let names = c.read_bytes(table,old.len());
        assert_eq!(&names[..72],&old[..72]);
        assert_eq!(&names[96..],&old[96..]);
        let name = std::str::from_utf8(&names[72..91]).unwrap();
        for (i,ch) in name.bytes().enumerate() {
            assert!(match i { 4|7 => ch == b'-', 10 => ch == b' ', 13|16 => ch == b':', _ => ch.is_ascii_digit() });
        }
        assert_eq!(&names[91..96],&[0;5]);
        assert_eq!(c.r,regs); // no stack unwind or synthetic key presses
        assert_eq!(c.eip,rt(0x04FE20));
        assert_eq!(c.rd32(0x1050),1);
        assert_eq!(c.rd32(0x1068),0);
        assert_eq!(c.rd32(0x104C),3);
        assert_eq!(c.rd8(rt(0x207CD4)),1);
        assert!(m.kbd_queue.is_empty());
        let saved = c.read_bytes(table,old.len()).to_vec();
        for invalid in [0,11,u32::MAX] {
            c.wr32(0x106C,invalid);
            automatic_save_name(&mut m,&mut c).unwrap();
            assert_eq!(c.rd32(0x106C),0);
            assert_eq!(c.read_bytes(table,old.len()),saved);
        }
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn quit_confirmation_returns_yes_without_drawing_or_exiting_early() {
        let path = std::env::temp_dir().join(format!("cybermage-quit-{}",std::process::id()));
        let mut m = Machine::new(crate::vfs::Vfs::new(path.join("iso"),path.join("run")));
        let mut c = Cpu::new();
        c.r[x86::ESP] = 0x30000;
        c.push32(0x123456);
        c.wlog_on = true;
        quit_confirm(&mut m,&mut c).unwrap();
        assert_eq!(c.r[EAX],1);
        assert_eq!(c.eip,0x123456);
        assert_eq!(c.r[x86::ESP],0x30000);
        assert!(c.wlog.is_empty());
        assert!(m.exit_code.is_none()); // leave shutdown to the original caller
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn vehicle_and_shrunk_views_stay_inside_their_4_3_frames() {
        // ConWinInfo 0/1/4, TankWinInfo and CarWinInfo from the original tables.
        let views = [(0,0,640,480),(80,60,480,360),(200,150,240,180),(80,32,480,256),(0,16,640,276)];
        for &(w,h) in MODES {
            assert_eq!(world_rect(views[0],(w,h),view_widens(views[0],1)),(0,0,w,h)); // full view spans the output
            // The aircar's canopy side art has windows: its view continues behind them.
            let (uw,uh,_,oy) = ui_rect(w,h,640,480);
            assert!(view_widens(views[4],CAR_BACKSCREEN) && !view_widens(views[4],TANK_BACKSCREEN));
            assert_eq!(world_rect(views[4],(w,h),true),(0,oy+(16*uh).div_ceil(480),w,(oy+(292*uh).div_ceil(480)-oy-(16*uh).div_ceil(480)).max(2)));
            assert!(views[1..4].iter().all(|&v| !view_widens(v,CAR_BACKSCREEN)));
            let _ = uw;
            let (uw,uh,ox,oy) = ui_rect(w,h,640,480);
            for &(x,y,vw,vh) in &views[1..] {
                let (rx,ry,rw,rh) = world_rect((x,y,vw,vh),(w,h),false);
                // Inside the scaled frame opening, within one UI pixel of its edges.
                let (sx,sy) = (uw as f64/640.0,uh as f64/480.0);
                assert!((rx as f64-(ox as f64+x as f64*sx)).abs() <= 1.0,"{w}x{h} {x}");
                assert!((ry as f64-(oy as f64+y as f64*sy)).abs() <= 1.0,"{w}x{h} {y}");
                assert!(rx+rw <= ox+((x+vw)*uw).div_ceil(640) && rx+rw+sx.ceil() as usize >= ox+(x+vw)*uw/640);
                assert!(ry+rh <= oy+((y+vh)*uh).div_ceil(480) && ry+rh+1 >= oy+(y+vh)*uh/480);
                assert_eq!(rw % 2,0);
            }
        }
        assert_eq!(world_rect((80,32,480,256),(1280,800),false),(240,54,800,426));
        assert_eq!(world_rect((0,16,640,276),(1920,1080),false),(240,36,1440,621));
        assert_eq!(world_rect((0,16,640,276),(1920,1080),true),(0,36,1920,621));
    }
    #[test]
    fn vehicle_side_art_fills_margins_and_shows_the_world_through_windows() {
        for (backscreen,windows) in [(TANK_BACKSCREEN,false),(CAR_BACKSCREEN,true)] {
            let art = vehicle_sides(backscreen).unwrap();
            assert_eq!(art.left.len(),SIDE_W*480); assert_eq!(art.right.len(),SIDE_W*480);
            // Only the aircar has see-through pixels, and only inside its 3D view rows.
            for half in [&art.left,&art.right] {
                assert_eq!(half.iter().any(|&p| p == CLEAR),windows);
                assert!(half.iter().enumerate().all(|(i,&p)| p != CLEAR || (16..292).contains(&(i / SIDE_W))));
            }
        }
        assert!(vehicle_sides(1).is_none() && vehicle_sides(0).is_none());
        let art = SideArt { left: (0..SIDE_W*480).map(|i| if i % 3 == 0 { CLEAR } else { 0x100000 | i as u32 }).collect(),
            right: (0..SIDE_W*480).map(|i| if i % 5 == 0 { CLEAR } else { 0x200000 | i as u32 }).collect() };
        let legacy: Vec<u32> = (0..640*480).map(|i| 0x300000 | i as u32).collect();
        let pal = std::array::from_fn(|i| 0x400000 | i as u32);
        let hud = vec![false; 640*480];
        for &(w,h) in MODES {
            let (uw,uh,ox,oy) = ui_rect(w,h,640,480);
            for rect in [(0,uh/10,w,uh/2),(ox+uw/8,uh/10,uw*3/4,uh/2)] {
                let (vx,vy,vw,vh) = rect;
                let world: Vec<u8> = (0..vw*vh).map(|i| (i % 251) as u8).collect();
                let mut out = vec![0xDEADBEEF; w*h];
                compose(&legacy,(640,480),(w,h),&world,(vw,vh),&hud,rect,true,&pal,Some(&art),&mut out);
                for y in (0..h).step_by(7) { for x in (0..w).step_by(3) {
                    if (ox..ox+uw).contains(&x) { continue; }
                    let sy = (y-oy)*480/uh;
                    let (src,col) = if x < ox { (&art.left,SIDE_W.checked_sub(((ox-x)*640).div_ceil(uw))) }
                        else { (&art.right,Some((x-ox-uw)*640/uw).filter(|&c| c < SIDE_W)) };
                    let in_world = (vx..vx+vw).contains(&x) && (vy..vy+vh).contains(&y);
                    let expected = match col.map(|c| src[sy*SIDE_W+c]) {
                        Some(p) if p != CLEAR => p,
                        Some(_) if in_world => pal[world[(y-vy)*vw+x-vx] as usize],
                        _ => 0,
                    };
                    assert_eq!(out[y*w+x],expected,"{w}x{h} ({x},{y}) rect {rect:?}");
                } }
                // The column next to the UI is the art's innermost column.
                if ox > 0 { assert_eq!(out[oy*w+ox-1],if art.left[SIDE_W-1] == CLEAR {out[oy*w+ox-1]} else {art.left[SIDE_W-1]}); }
            }
        }
    }
    #[test]
    fn modes_and_ui_keep_square_pixels() {
        for &(w, h) in MODES {
            assert!(parse_mode(&format!("{w}x{h}")).is_some());
            let (uw, uh, x, y) = ui_rect(w, h, 640, 480);
            assert!((uw as f64 / uh as f64 - 4.0/3.0).abs() < 0.003);
            assert!(x + uw <= w && y + uh <= h);
        }
        assert_eq!(parse_mode("99999x99999"), None);
        assert_eq!(parse_mode("1920"), None);
        assert_eq!(ui_rect(1920, 1080, 640, 480), (1440, 1080, 240, 0));
    }
}
