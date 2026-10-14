//! Standard twin-stick controls on Windows XInput (also Steam Input's Xbox output).
use crate::{display, le::rt, machine::Machine};
use std::collections::VecDeque;
use x86::{Cpu, Fault, EAX};

pub const UP: u16 = 0x0001;
pub const DOWN: u16 = 0x0002;
pub const LEFT: u16 = 0x0004;
pub const RIGHT: u16 = 0x0008;
pub const START: u16 = 0x0010;
pub const BACK: u16 = 0x0020;
pub const L3: u16 = 0x0040;
pub const R3: u16 = 0x0080;
pub const LB: u16 = 0x0100;
pub const RB: u16 = 0x0200;
pub const A: u16 = 0x1000;
pub const B: u16 = 0x2000;
pub const X: u16 = 0x4000;
pub const Y: u16 = 0x8000;
const TRIGGER_LT: u16 = 0x0400;
const TRIGGER_RT: u16 = 0x0800;
// Menu-only inDevice reads: pause, save/load (two), title (two), title load.
const MENU_DEVICE_READS: &[u32] = &[0x04EDE4,0x04F9D6,0x04FBF9,0x0553B8,0x055E57,0x0567A1];

fn buttons(p: Pad) -> u16 {
    p.buttons | if p.lt > 0.12 { TRIGGER_LT } else { 0 }
        | if p.rt > 0.12 { TRIGGER_RT } else { 0 }
}

#[derive(Clone, Copy, Default)]
pub struct Pad {
    pub connected: bool,
    pub buttons: u16,
    pub left: (f32, f32),
    pub right: (f32, f32),
    pub lt: f32,
    pub rt: f32,
    pub guest_this: u32,
    pub apply_stub: u32,
    pub last_time: f64,
    pub pitch_fraction: f32,
    pub game_buttons: u16,
    pub jump_held: bool,
}
impl Pad {
    pub fn down(self, button: u16) -> bool { self.buttons & button != 0 }
}

#[cfg(windows)]
pub fn poll(deadzone: f32) -> Pad {
    #[repr(C)]
    #[derive(Default)]
    struct Gamepad { buttons: u16, lt: u8, rt: u8, lx: i16, ly: i16, rx: i16, ry: i16 }
    #[repr(C)]
    #[derive(Default)]
    struct State { packet: u32, pad: Gamepad }
    #[link(name = "xinput9_1_0")]
    extern "system" { fn XInputGetState(index: u32, state: *mut State) -> u32; }
    for i in 0..4 {
        let mut s = State::default();
        if unsafe { XInputGetState(i, &mut s) } == 0 {
            let p = s.pad;
            let axis = |v: i16| if v < 0 { v as f32 / 32768.0 } else { v as f32 / 32767.0 };
            return Pad { connected: true, buttons: p.buttons,
                left: stick(axis(p.lx), axis(p.ly), deadzone), right: stick(axis(p.rx), axis(p.ry), deadzone),
                lt: p.lt as f32 / 255.0, rt: p.rt as f32 / 255.0, ..Pad::default() };
        }
    }
    Pad::default()
}
#[cfg(not(windows))]
pub fn poll(_deadzone: f32) -> Pad { Pad::default() }

/// Radial deadzone with rescaling; diagonal motion never exceeds full speed.
pub fn stick(x: f32, y: f32, deadzone: f32) -> (f32, f32) {
    let length = x.hypot(y);
    if length <= deadzone { return (0.0, 0.0); }
    let scale = ((length.min(1.0) - deadzone) / (1.0 - deadzone)) / length;
    (x * scale, y * scale)
}

pub fn movement(x: f32, y: f32, sprint: bool) -> (u16, i32) {
    let magnitude = x.hypot(y).min(1.0);
    let angle = (-x).atan2(y) * 65536.0 / std::f32::consts::TAU;
    (angle as i32 as u16, (magnitude * if sprint { 768.0 } else { 640.0 }).round() as i32)
}

pub fn input_hook(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    controller_input(m,c);
    if c.wlog_on || !m.gamepad.connected {
        c.eip = m.display.original_input;
        return Ok(());
    }
    if m.display.pending_game_options && c.rd32(rt(0x203F44)) == 0 {
        c.wr8(rt(0x207C55),1); // open the normal pause menu; it then selects Options
    }
    if m.gamepad.apply_stub == 0 {
        let p = m.heap_alloc(4).expect("controller return trap");
        m.hook_sig(c, crate::le::link(p), "Xox::GamepadApply", 0, false, apply);
        m.gamepad.apply_stub = p;
    }
    m.gamepad.guest_this = c.r[EAX];
    // Continue asynchronously through the original handler. Its help/menu loops
    // must remain interruptible by the frontend, not trapped inside call_guest.
    c.push32(m.gamepad.apply_stub);
    c.eip = m.display.original_input;
    Ok(())
}

fn apply(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let p = m.gamepad;
    let this = p.guest_this;
    let dt = (m.time - p.last_time).clamp(0.0, 0.1) as f32;
    m.gamepad.last_time = m.time;
    let jump = p.game_buttons & A != 0;
    if jump && !p.jump_held && c.rd32(rt(0x203F44)) == 0 && c.rd32(rt(0x1E3968)) == 0 { request_jump(c, this); }
    m.gamepad.jump_held = jump;
    let state = c.rd32(this + 0x14480C);
    if p.connected && c.rd32(rt(0x203F44)) == 0 && c.rd32(rt(0x1E3968)) == 0 && (1..=7).contains(&state) && state != 3 {
        if p.game_buttons & TRIGGER_LT != 0 && state != 7 {
            // Ctrl means jump when running, hand mode when stationary. LT is
            // explicitly the hand action; never turn it into another jump.
            c.wr32(this + 0x144B08,0);
        } else if p.left != (0.0, 0.0) {
            let (angle, speed) = movement(p.left.0, p.left.1, p.down(L3));
            c.wr16(this + 0x130D10, angle);
            c.wr32(this + 0x144B08, speed as u32);
        }
        if p.right.0 != 0.0 {
            c.wr32(this + 0x144B0C, (-p.right.0 * 128.0 * m.display.sensitivity).round() as i32 as u32);
        }
        if p.right.1 != 0.0 {
            let sign = if m.display.invert_y { 1.0 } else { -1.0 };
            let pitch = c.rd32(this + 0xE33F8) as i32;
            let step = p.right.1 * sign * 300.0 * m.display.sensitivity * dt + p.pitch_fraction;
            let delta = step.trunc() as i32;
            m.gamepad.pitch_fraction = step - delta as f32;
            c.wr32(this + 0xE33F8, (pitch + delta).clamp(-224, 224) as u32);
        } else { m.gamepad.pitch_fraction = 0.0; }
        crate::desktop::look(m,c,this);
    }
    // The original caller's return address remains beneath our return trap.
    c.ret_near(0);
    Ok(())
}

/// Original ProcessInputEvents jump setup (0x014D5E..0x014E63), without
/// Ctrl's stationary hand-mode fallback. Keep the original grounded/flight guards.
fn request_jump(c: &mut Cpu, this: u32) -> bool {
    if ![1,2,5].contains(&c.rd32(this + 0x14480C))
        || [0x144B40,0x144B20,0x144B1C,0x144B30,0x144B50].iter().any(|&o| c.rd32(this+o) != 0)
        || c.rd32(this + 0x144B3C) == u32::MAX { return false; }
    let obj = c.rd32(this + 0x130D04);
    let height = c.rd32(this + 0x66CE8 + obj * 4) as i32;
    c.wr32(this + 0x130D1C, height.wrapping_add(height / 2).wrapping_add(0x1000) as u32);
    for o in [0x144B34,0x144B38,0x144B3C,0x144B28] { c.wr32(this+o,0); }
    let view = c.rd32(this + 0xBDEBC);
    c.wr32(this + 0x4B170 + view * 36,0);
    let strength = (4 - c.rd32(this + 0x144B00) as i32 / 24).clamp(1,4);
    c.wr32(this + 0x144B1C,strength as u32);
    c.wr32(this + 0x144B20,1);
    let prop = c.rd32(this + 0x13141C + c.rd32(this + 0x130D08) * 4);
    let floor = if c.rd32(prop + 0x7C) != 0 {
        let z = c.rd32(prop + 0x80); c.wr32(prop + 0x7C,0); z
    } else { c.rd32(this + 0x78D8C + obj * 4) };
    c.wr32(this + 0x144B2C,floor);
    c.wr32(this + 0x144B24,c.rd32(this + 0x144AE8));
    true
}

pub fn gameplay(m: &Machine, c: &Cpu) -> bool {
    let state = c.rd32(rt(0x09E5DC) + 0x14480C);
    !m.display.help_open && !m.display.options_open && !m.display.pending_game_options && m.display.presented && !crate::display::pause_menu(c) && c.rd32(rt(0x203F44)) == 0 && c.rd32(rt(0x1E3968)) == 0
        && (1..=7).contains(&state) && state != 3
}

/// Menu keyboard handling is unconditional, but the subsequent pointer branch
/// reads the exclusive saved device. Select mouse only at those reads; changing
/// the global would also change Options/configuration and gameplay semantics.
pub fn install_menu_routes(c: &mut Cpu) {
    for &addr in MENU_DEVICE_READS {
        let p = rt(addr);
        assert_eq!(c.rd8(p),0xA1);
        assert_eq!(c.rd32(p+1),rt(0x1E38CC));
        c.write_bytes(p,&[0xB8,1,0,0,0]); // mov eax,1 instead of mov eax,[inDevice]
    }
}

// Arrow blocks: entry, continuation, current/next stack slots, grid columns, first/last.
// Host pages already wrap; these are the original pause/title/file/options menus.
pub const MENU_NAVIGATION: [(u32,u32,u32,u32,i32,i32,i32);5] = [
    (0x04E9EA,0x04EA99,0x44,0x40,2,1,6),
    (0x054F18,0x054FD8,0x54,0x60,3,1,6),
    (0x04F5AB,0x04F5ED,0x78,0x7C,1,2,11),
    (0x05595C,0x0559A8,0x94,0xA0,1,2,11),
    (0x05403D,0x054091,0x20,0x20,1,3,9),
];

pub fn menu_navigation(m: &mut Machine, c: &mut Cpu) -> Result<(), Fault> {
    let i = MENU_NAVIGATION.iter().position(|&(addr,..)| rt(addr) == c.cur_eip).expect("menu arrow hook");
    if !m.gamepad.connected || c.wlog_on {
        c.eip = m.display.original_navigation[i];
        return Ok(());
    }
    let (_,next,current,target,columns,mut first,last) = MENU_NAVIGATION[i];
    let sp = c.r[x86::ESP];
    if i == 0 && c.rd32(sp+0x28) != 1 { first = 3; } // death: no Resume/Save
    let selected = (c.rd32(sp+current) as i32).clamp(first,last);
    let mut choice = selected;
    let mut pressed = false;
    for (sc,delta) in [(0x4B,-1),(0x4D,1),(0x48,-columns),(0x50,columns)] {
        if columns == 1 && (sc == 0x4B || sc == 0x4D) { continue; } // sliders / pointer
        let key = rt(0x207C54)+sc;
        if c.rd8(key) == 0 { continue; }
        c.wr8(key,0);
        pressed = true;
        let row = selected+delta;
        choice = if row > last {first} else if row < first {last} else {row};
    }
    c.wr32(sp+target,choice as u32);
    if i == 1 && pressed { c.wr32(sp+0x64,c.rd32(rt(0x207473)).wrapping_add(0x960)); }
    if i == 4 && choice != selected { c.wr32(sp+0x18,1); }
    c.eip = rt(next); // original selection sound, highlight, pointer placement and actions
    Ok(())
}

/// Shared internal DOS keyboard/pointer transport. Desktop/XInput commands
/// both use these routes; saved legacy bindings remain compatible.
pub fn controller_input(m: &Machine, c: &mut Cpu) {
    if m.verifier.is_some() || c.wlog_on { return; }
    let this = rt(0x09E5DC);
    c.wr32(this+0xE3510,1);
    c.wr32(this+0xE3518,0);
}

/// Update the shared virtual DOS cursor/buttons; desktop position can override the stick.
pub fn pointer(m: &mut Machine, p: Pad, playing: bool, menu_buttons: u16, dt: f32) {
    let mouse = &mut m.mouse;
    mouse.mickey_x = 0; mouse.mickey_y = 0;
    if p.connected && !playing {
        let speed = 480.0 * dt.clamp(0.0,0.1);
        let x = (mouse.x + (p.right.0 * speed).round() as i32).clamp(0,639);
        let y = (mouse.y * 2 - (p.right.1 * speed).round() as i32).clamp(0,479);
        m.display.ui_pointer=(x,y);
        mouse.x=x; mouse.y=y/2;
    }
    let buttons = if p.connected && !playing { menu_buttons } else { 0 };
    for i in 0..3 {
        let was = mouse.buttons & (1<<i) != 0;
        let now = buttons & (1<<i) != 0;
        if now && !was { mouse.press_count[i] = mouse.press_count[i].wrapping_add(1); }
        if was && !now { mouse.release_count[i] = mouse.release_count[i].wrapping_add(1); }
    }
    mouse.buttons = buttons;
}

/// Shared DOS keys: releasing one input source must not release the other.
pub struct Keys {
    pad: [bool; 256], desktop: [bool;256],
    game_buttons: u16, menu_buttons: u16, blocked: u16,
}
impl Default for Keys {
    fn default() -> Self {
        Self { pad: [false; 256], desktop: [false;256], game_buttons: 0, menu_buttons: 0, blocked: 0 }
    }
}
impl Keys {
    pub fn pad(&mut self, q: &mut VecDeque<u8>, desired: [bool; 256]) {
        for (i, &down) in desired.iter().enumerate() {
            let was = self.pad[i] || self.desktop[i];
            let now = down || self.desktop[i];
            if was != now { transition(q, (i & 127) as u8, i >= 128, now); }
        }
        self.pad = desired;
    }
    pub fn desktop(&mut self, q: &mut VecDeque<u8>, desired: [bool;256]) {
        for (i,&down) in desired.iter().enumerate() {
            let was = self.pad[i] || self.desktop[i];
            let now = self.pad[i] || down;
            if was != now { transition(q,(i & 127) as u8,i >= 128,now); }
        }
        self.desktop = desired;
    }
    /// A held button keeps the context in which it was pressed. Otherwise the
    /// press that opens an action screen can become Enter and immediately close it.
    pub fn controller(&mut self, q: &mut VecDeque<u8>, p: Pad, c: &Cpu, playing: bool) -> u16 {
        self.blocked &= buttons(p);
        let held = buttons(p) & !self.blocked;
        let pressed = held & !(self.game_buttons | self.menu_buttons);
        self.game_buttons = (self.game_buttons | if playing { pressed } else { 0 }) & held;
        self.menu_buttons = (self.menu_buttons | if playing { 0 } else { pressed }) & held;
        let mut desired = bindings(Pad { buttons: 0, lt: 0.0, rt: 0.0, ..p }, c, playing);
        for (buttons, context) in [(self.game_buttons,true),(self.menu_buttons,false)] {
            let digital = bindings(Pad { buttons, ..Pad::default() }, c, context);
            for (key, down) in desired.iter_mut().zip(digital) { *key |= down; }
        }
        self.pad(q, desired);
        self.game_buttons
    }
    pub fn menu_mouse(&self) -> u16 {
        u16::from(self.menu_buttons & TRIGGER_RT != 0)
            | (u16::from(self.menu_buttons & TRIGGER_LT != 0) << 1)
    }
    /// Host overlays consume their buttons until release, including the press
    /// that closes them. Never leak that A/Start press back into the DOS game.
    pub fn suppress_controller(&mut self, q: &mut VecDeque<u8>, p: Pad) {
        self.blocked = buttons(p);
        self.game_buttons = 0; self.menu_buttons = 0;
        self.pad(q,[false;256]);
    }
    pub fn clear(&mut self, q: &mut VecDeque<u8>) {
        for i in 0..256 {
            if self.pad[i] || self.desktop[i] { transition(q, (i & 127) as u8, i >= 128, false); }
        }
        self.pad.fill(false); self.desktop.fill(false);
    }
}
pub fn transition(q: &mut VecDeque<u8>, sc: u8, ext: bool, down: bool) {
    if ext { q.push_back(0xE0); }
    q.push_back(sc | if down { 0 } else { 0x80 });
}
pub fn tap(q: &mut VecDeque<u8>, sc: u8, ext: bool) {
    transition(q, sc, ext, true);
    transition(q, sc, ext, false);
}

pub fn bindings(p: Pad, c: &Cpu, playing: bool) -> [bool; 256] {
    let mut keys = [false; 256];
    let mut key = |sc: u8, ext: bool, down: bool| {
        if down { keys[sc as usize + if ext { 128 } else { 0 }] = true; }
    };
    if playing {
        // Respect the player's original rebindings (bit 4 denotes E0 keys).
        for (addr, down) in [
            (0x1E3C10, p.rt > 0.12 || p.down(TRIGGER_RT)),
            (0x1E3C40, p.lt > 0.12 || p.down(TRIGGER_LT)), // original contextual hand control
            (0x1E3C18, p.down(X)),
            (0x1E3C48, p.down(B)), (0x1E3C60, p.down(R3)),
            (0x1E3C68, p.down(UP)), (0x1E3C70, p.down(DOWN)),
            (0x1E3C80, p.down(LEFT)), (0x1E3C20, p.down(RIGHT)),
        ] {
            let sc = c.rd32(rt(addr));
            if sc < 128 { key(sc as u8, c.rd32(rt(addr + 4)) == 4, down); }
        }
        key(0x3B, false, p.down(LB)); // F1 weapons (Origin reference card)
        key(0x3C, false, p.down(RB)); // F2 Darklight powers
        key(0x3D, false, p.down(Y));  // keys/inventory selector
    } else {
        key(0x48, true, p.down(UP) || p.left.1 > 0.5);
        key(0x50, true, p.down(DOWN) || p.left.1 < -0.5);
        key(0x4B, true, p.down(LEFT) || p.left.0 < -0.5);
        key(0x4D, true, p.down(RIGHT) || p.left.0 > 0.5);
        key(0x1C, false, p.down(A));
        key(0x0E, false, p.down(X)); // clear input in drop-down screens
        key(0x01, false, p.down(B));
        key(0x15, false, p.down(RB)); // Yes in confirmation dialogs
        key(0x31, false, p.down(LB)); // No
    }
    key(0x01, false, p.down(START));
    keys
}

/// Less-used original commands without awkward multi-button chords.
#[derive(Clone, Copy)]
pub enum Command { Key(u8), Bound(u32), Options, Help }
pub(crate) const ACTIONS: &[(&str,Command)] = &[
    ("AUTOMAP ON/OFF",Command::Bound(0x1E3C78)),
    ("ENHANCEMENTS",Command::Options),
    ("PAUSE ON/OFF",Command::Key(0x19)),
    ("CONTROL GUIDES",Command::Help),
    ("SKIP NPC SPEECH",Command::Key(0x2B)),
    ("CYCLE VDU MONITORS",Command::Bound(0x1E3C80)),
    ("ENLARGE GAME VIEW",Command::Key(0x1B)),
    ("SHRINK GAME VIEW",Command::Key(0x1A)),
    ("MAP ZOOM - LARGER AREA",Command::Key(0x34)),
    ("MAP ZOOM - SMALLER AREA",Command::Key(0x33)),
    ("ENLARGE AUTOMAP",Command::Key(0x0D)),
    ("SHRINK AUTOMAP",Command::Key(0x0C)),
    ("NARROW AIRCAR GUN AIM",Command::Key(0x1E)),
    ("WIDEN AIRCAR GUN AIM",Command::Key(0x20)),
    ("CENTER AIRCAR GUN AIM",Command::Key(0x1F)),
    ("EXIT VEHICLE / LAND",Command::Bound(0x1E3C20)),
    ("WEAPONS",Command::Key(0x3B)),
    ("DARKLIGHT POWERS",Command::Key(0x3C)),
    ("INVENTORY",Command::Key(0x3D)),
    ("ORIGINAL MAIN MENU",Command::Key(0x01)),
];
#[derive(Default)]
pub struct Actions {
    pub open: bool, pub selected: usize,
    release: Option<(std::time::Instant,u8,bool)>,
}
impl Actions {
    pub fn navigate(&mut self, delta: i32) {
        self.selected = (self.selected as i32 + delta).rem_euclid(ACTIONS.len() as i32) as usize;
    }
    pub fn page(&mut self, delta: i32) {
        let page = (self.selected as i32 / 8 + delta).rem_euclid(ACTIONS.len().div_ceil(8) as i32);
        self.selected = (page as usize * 8 + self.selected % 8).min(ACTIONS.len()-1);
    }
    pub fn choose(&mut self, q: &mut VecDeque<u8>, c: &Cpu) -> Command {
        self.open = false;
        let command = ACTIONS[self.selected].1;
        let key = match command {
            Command::Key(sc) => Some((sc,false)),
            Command::Bound(addr) => {
                let sc = c.rd32(rt(addr));
                if sc < 128 { Some((sc as u8,addr != 0x0962B8 && c.rd32(rt(addr+4)) == 4)) } else { None }
            }
            _ => None,
        };
        self.release_now(q);
        if let Some((sc,ext)) = key {
            // Aircar A/D check the held bit, not the press latch. An immediate
            // make/break pair can disappear before the guest polls its keyboard.
            // ponytail: 200ms pulse; add hold-to-repeat if very low FPS needs longer holds.
            transition(q,sc,ext,true);
            self.release = Some((std::time::Instant::now()+std::time::Duration::from_millis(200),sc,ext));
        }
        command
    }
    pub fn update(&mut self, q: &mut VecDeque<u8>) {
        if self.release.is_some_and(|(time,_,_)| std::time::Instant::now() >= time) { self.release_now(q); }
    }
    pub fn release_now(&mut self, q: &mut VecDeque<u8>) {
        if let Some((_,sc,ext)) = self.release.take() { transition(q,sc,ext,false); }
    }
    pub fn draw(&self, art: &display::MenuArt, out: &mut Vec<u32>) -> (usize,usize) {
        let page = self.selected / 8;
        art.page(out,2,&format!("ACTIONS - PAGE {} / {}",page+1,ACTIONS.len().div_ceil(8)),
            "ARROWS SELECT  PGUP/PGDN/LB/RB PAGE  ENTER/A OK  ESC/B BACK");
        for (row,i) in (page*8..((page+1)*8).min(ACTIONS.len())).enumerate() {
            art.separator(out,146+(row+1)*32-6);
            art.label(out,46,146+row*32,ACTIONS[i].0,i == self.selected);
        }
        (640,480)
    }
    pub fn command(&self) -> Command { ACTIONS[self.selected].1 }
    pub fn point(&mut self, x: f32, y: f32) -> bool {
        let i = self.selected / 8 * 8 + ((y - 140.0) / 32.0) as usize;
        if (14.0..624.0).contains(&x) && (140.0..396.0).contains(&y) && i < ACTIONS.len() {
            self.selected = i; return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn controls(c: &mut Cpu) {
        for (addr,sc) in [(0x1E3C10,0x39),(0x1E3C18,0x1C),(0x1E3C40,0x1D),(0x1E3C48,0x52),
            (0x1E3C60,0x13),(0x1E3C68,0x11),(0x1E3C70,0x2D),(0x1E3C78,0x32),
            (0x1E3C80,0x2F),(0x1E3C20,0x0F)] {
            c.wr32(rt(addr),sc); c.wr32(rt(addr+4),1);
        }
        c.wr32(rt(0x0962B8),0x35);
    }
    #[test]
    fn xbox360_layout_and_held_inputs() {
        let mut c = Cpu::new(); controls(&mut c);
        for (button,sc) in [(LB,0x3B),(RB,0x3C),(Y,0x3D),(X,0x1C),(B,0x52),
            (R3,0x13),(UP,0x11),(DOWN,0x2D),(LEFT,0x2F),(RIGHT,0x0F),
            (START,0x01),(TRIGGER_RT,0x39),(TRIGGER_LT,0x1D)] {
            let actual = bindings(Pad { buttons: button, ..Pad::default() },&c,true);
            assert_eq!(actual.iter().position(|&k| k),Some(sc));
            assert_eq!(actual.iter().filter(|&&k| k).count(),1);
        }
        assert!(!bindings(Pad { buttons: A | BACK, ..Pad::default() },&c,true).contains(&true));
        let mut keys = Keys::default(); let mut q = VecDeque::new();
        let x = Pad { connected: true, buttons: X, ..Pad::default() };
        keys.controller(&mut q,x,&c,true);
        keys.controller(&mut q,x,&c,false); // hand screen must not turn held X into Backspace
        assert_eq!(q.drain(..).collect::<Vec<_>>(),vec![0x1C]);
        keys.controller(&mut q,Pad::default(),&c,false);
        assert_eq!(q.pop_front(),Some(0x9C));
        keys.controller(&mut q,x,&c,false);
        keys.controller(&mut q,x,&c,true);
        keys.controller(&mut q,Pad::default(),&c,true);
        assert_eq!(q.drain(..).collect::<Vec<_>>(),vec![0x0E,0x8E]);
        let a = Pad { buttons: A, ..x };
        keys.suppress_controller(&mut q,a); // A used to dismiss a host overlay
        assert_eq!(keys.controller(&mut q,a,&c,true) & A,0);
        assert!(q.is_empty());
        keys.controller(&mut q,Pad::default(),&c,true);
        assert_ne!(keys.controller(&mut q,a,&c,true) & A,0);
        assert!(q.is_empty()); // native jump, never Ctrl or Enter
        keys.controller(&mut q,a,&c,false);
        assert!(q.is_empty());
        keys.controller(&mut q,Pad::default(),&c,false);
        assert_eq!(keys.controller(&mut q,a,&c,false) & A,0);
        assert_eq!(q.pop_front(),Some(0x1C));
        keys.controller(&mut q,Pad::default(),&c,false);
        assert_eq!(q.pop_front(),Some(0x9C)); // releasing A releases virtual Enter
        let trigger = Pad { rt: 1.0, ..Pad::default() };
        keys.controller(&mut q,trigger,&c,true);
        keys.controller(&mut q,trigger,&c,false);
        assert_eq!(keys.menu_mouse(),0); // held fire cannot become a menu click
        keys.controller(&mut q,Pad::default(),&c,false);
        keys.controller(&mut q,trigger,&c,false);
        assert_eq!(keys.menu_mouse(),1);
        keys.controller(&mut q,Pad::default(),&c,false);
        assert_eq!(keys.menu_mouse(),0);
        keys.controller(&mut q,Pad { rt: 1.0, lt: 1.0, ..Pad::default() },&c,false);
        assert_eq!(keys.menu_mouse(),3); // both shop mouse actions are reachable
    }
    #[test]
    fn controller_virtual_routes_preserve_legacy_save_preferences() {
        let path = std::env::temp_dir().join(format!("cybermage-input-{}",std::process::id()));
        let mut m = Machine::new(crate::vfs::Vfs::new(path.join("iso"),path.join("run")));
        let mut c = Cpu::new(); let this = rt(0x09E5DC);
        for device in 0..=2 {
            for connected in [false,true] {
                m.gamepad.connected = connected;
                c.wr32(rt(0x1E38CC),device);
                c.wr32(this+0xE3510,0); c.wr32(this+0xE3518,1);
                controller_input(&m,&mut c);
                assert_eq!(c.rd32(this+0xE3510),1);
                assert_eq!(c.rd32(this+0xE3518),0);
                assert_eq!(c.rd32(rt(0x1E38CC)),device);
            }
        }
        for &addr in MENU_DEVICE_READS {
            c.wr8(rt(addr),0xA1); c.wr32(rt(addr)+1,rt(0x1E38CC));
        }
        install_menu_routes(&mut c);
        for &addr in MENU_DEVICE_READS {
            c.eip = rt(addr);
            c.step(&mut m).unwrap();
            assert_eq!(c.r[EAX],1);
            assert_eq!(c.eip,rt(addr)+5);
            assert_eq!(c.rd32(rt(0x1E38CC)),2);
        }
        c.wr32(this+0xE3510,0); c.wr32(this+0xE3518,1);
        c.wlog_on = true;
        controller_input(&m,&mut c);
        assert_eq!((c.rd32(this+0xE3510),c.rd32(this+0xE3518)),(0,1));
        assert!(c.wlog.is_empty());
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn original_controller_menus_wrap_and_preserve_grid_steps() {
        let path = std::path::PathBuf::from("__menu_wrap_no_files__");
        let mut m = Machine::new(crate::vfs::Vfs::new(path.clone(),path));
        let mut c = Cpu::new(); c.r[x86::ESP] = 0x30000;
        m.gamepad.connected = true;
        for (i,&(addr,next,current,target,columns,first,last)) in MENU_NAVIGATION.iter().enumerate() {
            for death in [false,true] {
                c.wr32(0x30028,if death {2} else {1});
                let first = if i == 0 && death {3} else {first};
                for (selected,sc,expected) in [(last,0x50,first),(first,0x48,last),
                    (first,0x50,first+columns),(first+columns,0x48,first)] {
                    c.cur_eip = rt(addr); c.wr32(0x30000+current,selected as u32);
                    c.wr8(rt(0x207C54)+sc,1);
                    menu_navigation(&mut m,&mut c).unwrap();
                    assert_eq!(c.rd32(0x30000+target),expected as u32,"menu {i}");
                    assert_eq!(c.rd8(rt(0x207C54)+sc),0);
                    assert_eq!(c.eip,rt(next));
                }
                if columns > 1 {
                    c.wr32(0x30000+current,last as u32); c.wr8(rt(0x207CA1),1);
                    menu_navigation(&mut m,&mut c).unwrap();
                    assert_eq!(c.rd32(0x30000+target),first as u32);
                }
            }
            m.display.original_navigation[i] = 0x123400+i as u32;
            for (connected,logging) in [(false,false),(true,true)] {
                m.gamepad.connected = connected; c.wlog_on = logging; c.cur_eip = rt(addr);
                menu_navigation(&mut m,&mut c).unwrap();
                assert_eq!(c.eip,m.display.original_navigation[i]);
            }
            m.gamepad.connected = true; c.wlog_on = false;
        }
    }
    #[test]
    fn modern_jump_uses_original_physics_without_hand_mode() {
        let mut c = Cpu::new(); let this = 0x100000;
        c.wr32(this+0x14480C,5); c.wr32(this+0x144B00,24);
        c.wr32(this+0x66CE8,0x1000); c.wr32(this+0x13141C,0x500000);
        c.wr32(0x50007C,1); c.wr32(0x500080,12345);
        c.wr32(this+0x144AE8,640);
        assert!(request_jump(&mut c,this));
        assert_eq!(c.rd32(this+0x14480C),5);
        assert_eq!(c.rd32(this+0x144B1C),3);
        assert_eq!(c.rd32(this+0x144B20),1);
        assert_eq!(c.rd32(this+0x144B2C),12345);
        assert_eq!(c.rd32(this+0x144B24),640);
        assert_eq!(c.rd32(this+0x130D1C),0x2800);
        assert_eq!(c.rd32(0x50007C),0);
        assert!(!request_jump(&mut c,this)); // no mid-air repeated jump
        c.wr32(this+0x144B1C,0); c.wr32(this+0x144B20,0);
        for field in [0x144B40,0x144B30,0x144B50] {
            c.wr32(this+field,1); assert!(!request_jump(&mut c,this)); c.wr32(this+field,0);
        }
        for state in [0,3,4,6,7] {
            c.wr32(this+0x14480C,state); assert!(!request_jump(&mut c,this));
        }
    }
    #[test]
    fn original_utility_actions_are_reachable_without_a_keyboard_page() {
        let mut c = Cpu::new(); controls(&mut c);
        let mut q = VecDeque::new(); let mut panel = Actions::default();
        panel.page(-1); assert_eq!(panel.selected,16);
        panel.page(1); assert_eq!(panel.selected,0);
        panel.navigate(-1); assert_eq!(panel.selected,ACTIONS.len()-1);
        let mut pixels = Vec::new();
        let art = display::MenuArt::default();
        for i in 0..ACTIONS.len() {
            panel.selected = i; panel.open = true;
            assert_eq!(panel.draw(&art,&mut pixels),(640,480));
            let command = panel.choose(&mut q,&c);
            panel.release_now(&mut q);
            match command {
                Command::Key(sc) => assert_eq!(q.drain(..).collect::<Vec<_>>(),vec![sc,sc|0x80]),
                Command::Bound(addr) => {
                    let sc = c.rd32(rt(addr)) as u8;
                    assert_eq!(q.drain(..).collect::<Vec<_>>(),vec![sc,sc|0x80]);
                }
                _ => assert!(q.is_empty()),
            }
            assert!(!panel.open);
        }
        panel.selected = ACTIONS.iter().position(|&(label,_)| label == "NARROW AIRCAR GUN AIM").unwrap();
        panel.choose(&mut q,&c);
        assert_eq!(q.iter().copied().collect::<Vec<_>>(),vec![0x1E]);
        panel.release.as_mut().unwrap().0 = std::time::Instant::now()+std::time::Duration::from_secs(60);
        panel.update(&mut q); assert_eq!(q.len(),1);
        panel.release.as_mut().unwrap().0 = std::time::Instant::now()-std::time::Duration::from_millis(1);
        panel.update(&mut q);
        assert_eq!(q.drain(..).collect::<Vec<_>>(),vec![0x1E,0x9E]);
        assert!(!ACTIONS.iter().any(|&(label,_)| label.contains("KEYBOARD")));
    }
    #[test]
    fn cursor_buttons_are_controller_only_including_disconnect() {
        let path = std::env::temp_dir().join(format!("cybermage-pad-pointer-{}",std::process::id()));
        let mut m = Machine::new(crate::vfs::Vfs::new(path.join("iso"),path.join("run")));
        m.mouse.x = 320; m.mouse.y = 120; m.mouse.mickey_x = 100;
        let p = Pad { connected: true, right: (1.0,1.0), ..Pad::default() };
        pointer(&mut m,p,false,3,1.0/60.0);
        assert_eq!((m.mouse.x,m.mouse.y),(328,116));
        assert_eq!(m.display.ui_pointer,(328,232));
        assert_eq!(m.mouse.buttons,3); assert_eq!(m.mouse.press_count,[1,1,0]);
        pointer(&mut m,p,false,3,0.0);
        assert_eq!(m.mouse.press_count,[1,1,0]);
        pointer(&mut m,p,true,3,1.0/60.0);
        assert_eq!((m.mouse.x,m.mouse.y),(328,116));
        assert_eq!(m.mouse.buttons,0); assert_eq!(m.mouse.release_count,[1,1,0]);
        pointer(&mut m,Pad::default(),false,3,1.0/60.0);
        assert_eq!(m.mouse.buttons,0); assert_eq!(m.mouse.mickey_x,0);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn deadzone_motion_and_key_ownership() {
        assert_eq!(stick(0.1, 0.1, 0.18), (0.0, 0.0));
        let (x,y) = stick(1.0, 1.0, 0.18);
        assert!((x.hypot(y) - 1.0).abs() < 0.0001);
        assert_eq!(movement(0.0, 1.0, false), (0, 640));
        assert_eq!(movement(-1.0, 0.0, false), (0x4000, 640));
        assert_eq!(movement(1.0, 0.0, false), (0xC000, 640));
        assert_eq!(movement(0.0, -1.0, false), (0x8000, 640));
        let mut k = Keys::default();
        let mut q = VecDeque::new();
        let mut desired = [false; 256]; desired[0x39] = true;
        k.pad(&mut q, desired);
        k.pad(&mut q, desired);
        assert_eq!(q.iter().copied().collect::<Vec<_>>(),vec![0x39]);
        k.desktop(&mut q,desired);
        k.pad(&mut q,[false;256]);
        assert_eq!(q.iter().copied().collect::<Vec<_>>(),vec![0x39]);
        k.clear(&mut q);
        assert_eq!(q.pop_back(),Some(0xB9));
    }
}
