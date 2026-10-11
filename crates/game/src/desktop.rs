//! Modern desktop controls reuse the controller's original-game command transport.
use crate::{gamepad::{self, Pad}, le::rt};
use minifb::{Key, MouseMode, Window};
use x86::Cpu;

#[derive(Clone, Default)]
pub struct Snapshot {
    pub keys: Vec<Key>, pub mouse: u8, pub delta: (f32,f32), pub wheel: f32,
    pub point: Option<(f32,f32)>,
}
#[derive(Default)]
pub struct Input {
    raw: Snapshot, blocked: Vec<Key>, blocked_mouse: u8,
    pub mouse: u8, pub look: (f32,f32), pitch_fraction: f32,
    pub controller_guide: bool,
    point_changed: bool,
    locked: bool, playing: bool, focused: bool, cursor_hidden: bool,
    #[cfg(not(windows))] last_position: Option<(f32,f32)>,
}
impl Input {
    pub fn suppress(&mut self) {
        self.blocked = self.raw.keys.clone(); self.blocked_mouse = self.raw.mouse;
        self.look = (0.0,0.0); self.pitch_fraction=0.0; self.mouse = 0;
    }
    pub fn click(&self, next: &Snapshot) -> bool { next.mouse & !self.raw.mouse & !self.blocked_mouse & 1 != 0 }
    /// Call before update: preserve rising edges separately from held commands.
    pub fn edges(&self, next: &Snapshot) -> Vec<Key> {
        next.keys.iter().copied().filter(|k| !self.raw.keys.contains(k) && !self.blocked.contains(k)).collect()
    }
    pub fn sample(&mut self, window: &mut Window, capture: bool, render: (usize,usize)) -> Snapshot {
        if !window.is_active() {
            self.release_cursor(window); self.focused=false;
            return Snapshot::default();
        }
        let keys = window.get_keys();
        #[cfg(windows)]
        let (keys,mouse) = {
            // minifb may miss releases delivered to another app while unfocused.
            #[link(name="user32")]
            extern "system" { fn GetAsyncKeyState(key:i32)->i16; }
            let held=|key| (unsafe {GetAsyncKeyState(key)}) < 0;
            let keys=keys.into_iter().filter(|&key| virtual_key(key).is_none_or(held)).collect();
            (keys,u8::from(held(1)) | (u8::from(held(2))<<1))
        };
        #[cfg(not(windows))]
        let mouse = u8::from(window.get_mouse_down(minifb::MouseButton::Left))
            | (u8::from(window.get_mouse_down(minifb::MouseButton::Right)) << 1);
        let wheel = window.get_scroll_wheel().map_or(0.0,|(_,y)| y);
        let position = window.get_unscaled_mouse_pos(MouseMode::Pass);
        let point = if capture {None} else {position.and_then(|p| menu_point(p,window.get_size(),render))};
        let delta = self.capture(window,capture);
        let shot=Snapshot { keys, mouse, delta, wheel, point };
        if !self.focused {self.raw=shot.clone(); self.suppress(); self.focused=true;}
        shot
    }
    pub fn update(&mut self, next: Snapshot, mut pad: Pad, playing: bool, c: &Cpu) -> (Pad,[bool;256]) {
        if self.playing != playing {
            self.blocked = self.raw.keys.clone();
            self.look=(0.0,0.0); // mouse buttons retain their shared Keys context (including hand aim)
        }
        self.playing = playing;
        self.blocked.retain(|k| next.keys.contains(k)); self.blocked_mouse &= next.mouse;
        let keys: Vec<Key> = next.keys.iter().copied().filter(|k| !self.blocked.contains(k)).collect();
        let mouse = next.mouse & !self.blocked_mouse;
        let down = |k| keys.contains(&k);
        self.point_changed = next.point.zip(self.raw.point).is_some_and(|(a,b)| a != b) || next.mouse != self.raw.mouse;
        let desktop_used = next.keys != self.raw.keys || self.point_changed || next.delta != (0.0,0.0) || next.wheel != 0.0;
        if pad.buttons != 0 || pad.left != (0.0,0.0) || pad.right != (0.0,0.0) || pad.lt > 0.12 || pad.rt > 0.12 { self.controller_guide = true; }
        if desktop_used { self.controller_guide = false; }
        let mut extra = [false;256];
        let mut bind = |addr, held| {
            let sc = c.rd32(rt(addr));
            if held && sc < 128 { extra[sc as usize + if c.rd32(rt(addr+4))==4 {128} else {0}] = true; }
        };
        if playing {
            let (x,y) = (i32::from(down(Key::D))-i32::from(down(Key::A)),i32::from(down(Key::W))-i32::from(down(Key::S)));
            if x != 0 || y != 0 { pad.left = gamepad::stick(x as f32,y as f32,0.0); }
            for (key,button) in [(Key::Space,gamepad::A),(Key::LeftCtrl,gamepad::B),(Key::RightCtrl,gamepad::B),
                (Key::LeftShift,gamepad::L3),(Key::RightShift,gamepad::L3),(Key::E,gamepad::X),
                (Key::Key1,gamepad::LB),(Key::Key2,gamepad::RB),(Key::Key3,gamepad::Y),(Key::I,gamepad::Y),
                (Key::R,gamepad::R3),(Key::PageUp,gamepad::UP),(Key::PageDown,gamepad::DOWN),
                (Key::Q,gamepad::LEFT),(Key::F,gamepad::RIGHT)] { if down(key) { pad.buttons |= button; } }
            bind(0x1E3C78,down(Key::M) || down(Key::Tab));
            for (key,sc) in [(Key::P,0x19),(Key::Backslash,0x2B),(Key::LeftBracket,0x1A),(Key::RightBracket,0x1B),
                (Key::Equal,0x0D),(Key::Minus,0x0C),(Key::Comma,0x33),(Key::Period,0x34),
                (Key::Z,0x1E),(Key::C,0x20),(Key::V,0x1F)] { if down(key) { extra[sc]=true; } }
        } else {
            for (key,button) in [(Key::Up,gamepad::UP),(Key::W,gamepad::UP),(Key::Down,gamepad::DOWN),(Key::S,gamepad::DOWN),
                (Key::Left,gamepad::LEFT),(Key::A,gamepad::LEFT),(Key::Right,gamepad::RIGHT),(Key::D,gamepad::RIGHT),
                (Key::Enter,gamepad::A),(Key::NumPadEnter,gamepad::A),(Key::Space,gamepad::A),(Key::Backspace,gamepad::X),
                (Key::Y,gamepad::RB),(Key::N,gamepad::LB)] { if down(key) { pad.buttons |= button; } }
        }
        if mouse & 1 != 0 { pad.rt = 1.0; }
        if mouse & 2 != 0 { pad.lt = 1.0; }
        if down(Key::Escape) { pad.buttons |= gamepad::START; }
        if down(Key::F4) { pad.buttons |= gamepad::BACK; }
        // A focused desktop is always a usable input device; no gamepad is required.
        pad.connected = true;
        self.look.0 += next.delta.0; self.look.1 += next.delta.1;
        self.mouse = mouse; self.raw = next;
        (pad,extra)
    }
    pub fn release_cursor(&mut self, window: &mut Window) {
        self.capture(window,false); self.cursor_visibility(window,false);
    }
    fn cursor_visibility(&mut self, window: &mut Window, hidden: bool) {
        // minifb uses Win32 ShowCursor's counter, not an idempotent boolean setter.
        if self.cursor_hidden != hidden {window.set_cursor_visibility(!hidden); self.cursor_hidden=hidden;}
    }
    fn capture(&mut self, window: &mut Window, capture: bool) -> (f32,f32) {
        self.cursor_visibility(window,capture || (window.is_active() && self.controller_guide));
        #[cfg(windows)] {
            use std::ffi::c_void;
            #[repr(C)] #[derive(Default,Clone,Copy)] struct Point {x:i32,y:i32}
            #[repr(C)] #[derive(Default)] struct Rect {left:i32,top:i32,right:i32,bottom:i32}
            #[link(name="user32")]
            extern "system" {
                fn GetCursorPos(point:*mut Point)->i32; fn SetCursorPos(x:i32,y:i32)->i32;
                fn GetClientRect(hwnd:*mut c_void,rect:*mut Rect)->i32;
                fn ClientToScreen(hwnd:*mut c_void,point:*mut Point)->i32;
                fn ClipCursor(rect:*const Rect)->i32;
            }
            if !capture {
                if self.locked { unsafe { ClipCursor(std::ptr::null()); } self.locked=false; }
                return (0.0,0.0);
            }
            let hwnd=window.get_window_handle(); let mut r=Rect::default(); let mut origin=Point::default(); let mut position=Point::default();
            if unsafe { GetClientRect(hwnd,&mut r) }==0 || unsafe { ClientToScreen(hwnd,&mut origin) }==0 || unsafe { GetCursorPos(&mut position) }==0 { return (0.0,0.0); }
            let centre=Point{x:origin.x+(r.right-r.left)/2,y:origin.y+(r.bottom-r.top)/2};
            r.left+=origin.x; r.right+=origin.x; r.top+=origin.y; r.bottom+=origin.y;
            let delta=if self.locked {((position.x-centre.x) as f32,(position.y-centre.y) as f32)} else {(0.0,0.0)};
            unsafe { ClipCursor(&r); SetCursorPos(centre.x,centre.y); }
            self.locked=true;
            delta
        }
        #[cfg(not(windows))] {
            // ponytail: Windows package uses recentered relative motion; other hosts have bounded position deltas.
            let point=window.get_unscaled_mouse_pos(MouseMode::Pass);
            let delta=if capture && self.locked {point.zip(self.last_position).map_or((0.0,0.0),|(p,q)|(p.0-q.0,p.1-q.1))} else {(0.0,0.0)};
            self.last_position=point; self.locked=capture; delta
        }
    }
}

#[cfg(windows)]
fn virtual_key(key:Key) -> Option<i32> {
    let code=key as i32;
    Some(match key {
        _ if (Key::A as i32..=Key::Z as i32).contains(&code) => 0x41+code-Key::A as i32,
        _ if (Key::Key0 as i32..=Key::Key9 as i32).contains(&code) => 0x30+code-Key::Key0 as i32,
        _ if (Key::F1 as i32..=Key::F15 as i32).contains(&code) => 0x70+code-Key::F1 as i32,
        Key::Space=>0x20, Key::Enter | Key::NumPadEnter=>0x0D, Key::Escape=>0x1B, Key::Tab=>9, Key::Backspace=>8,
        Key::LeftCtrl=>0xA2, Key::RightCtrl=>0xA3, Key::LeftShift=>0xA0, Key::RightShift=>0xA1,
        Key::Up=>0x26, Key::Down=>0x28, Key::Left=>0x25, Key::Right=>0x27, Key::PageUp=>0x21, Key::PageDown=>0x22,
        Key::LeftBracket=>0xDB, Key::RightBracket=>0xDD, Key::Equal=>0xBB, Key::Minus=>0xBD,
        Key::Comma=>0xBC, Key::Period=>0xBE, Key::Backslash=>0xDC,
        _=>return None,
    })
}

pub fn pointer(m: &mut crate::machine::Machine, playing: bool) {
    if playing || m.desktop.controller_guide || !m.desktop.point_changed { return; }
    if let Some((x,y)) = m.desktop.raw.point {
        m.display.ui_pointer=(x as i32,y as i32);
        m.mouse.x=x as i32; m.mouse.y=(y/2.0) as i32; // DOS mouse uses half-height coordinates
    }
}

/// Undo both window-to-output and output-to-4:3-UI letterboxing.
pub fn menu_point(point:(f32,f32), window:(usize,usize), render:(usize,usize)) -> Option<(f32,f32)> {
    if window.0==0 || window.1==0 { return None; }
    let s=(window.0 as f32/render.0 as f32).min(window.1 as f32/render.1 as f32);
    let (x,y)=((point.0-(window.0 as f32-render.0 as f32*s)/2.0)/s,(point.1-(window.1 as f32-render.1 as f32*s)/2.0)/s);
    let u=(render.0 as f32/640.0).min(render.1 as f32/480.0);
    let (x,y)=((x-(render.0 as f32-640.0*u)/2.0)/u,(y-(render.1 as f32-480.0*u)/2.0)/u);
    ((0.0..640.0).contains(&x) && (0.0..480.0).contains(&y)).then_some((x,y))
}

pub fn look(m:&mut crate::machine::Machine,c:&mut Cpu,this:u32) {
    let (dx,dy)=std::mem::take(&mut m.desktop.look);
    if dx==0.0 && dy==0.0 { return; }
    let obj=c.rd32(this+0x130D04);
    let angle=this+0x7E65C+obj*2;
    c.wr16(angle,c.rd16(angle).wrapping_sub((dx*10.0*m.display.sensitivity).round() as i32 as u16));
    let pitch=c.rd32(this+0xE33F8) as i32;
    let sign=if m.display.invert_y {-1.0} else {1.0};
    let step=dy*0.5*sign*m.display.sensitivity+m.desktop.pitch_fraction;
    let delta=step.trunc() as i32;
    m.desktop.pitch_fraction=step-delta as f32;
    c.wr32(this+0xE33F8,(pitch+delta).clamp(-224,224) as u32);
}

/// Headless diagnostic holds: START:END:KEY+KEY:DX/s:DY/s:BUTTONS:WHEEL[:X:Y].
pub fn parse_script(spec: &str) -> Result<(f64,f64,Snapshot),String> {
    let fields: Vec<_> = spec.split(':').collect();
    let invalid = || "--kb expects START:END:KEY+KEY:DX/s:DY/s:BUTTONS:WHEEL[:X:Y]".to_string();
    if ![7,9].contains(&fields.len()) { return Err(invalid()); }
    let number = |i:usize| -> Result<f64,String> {
        let v = fields[i].parse::<f64>().map_err(|_| invalid())?;
        if !v.is_finite() {return Err(invalid());} Ok(v)
    };
    let (start,end)=(number(0)?,number(1)?);
    if start < 0.0 || end <= start {return Err(invalid());}
    let bindings = [
        ("W",Key::W),("A",Key::A),("S",Key::S),("D",Key::D),("E",Key::E),("Q",Key::Q),("F",Key::F),("R",Key::R),
        ("I",Key::I),("M",Key::M),("P",Key::P),("Z",Key::Z),("C",Key::C),("V",Key::V),("Y",Key::Y),("N",Key::N),
        ("1",Key::Key1),("2",Key::Key2),("3",Key::Key3),("SPACE",Key::Space),("CTRL",Key::LeftCtrl),
        ("SHIFT",Key::LeftShift),("ENTER",Key::Enter),("ESC",Key::Escape),("TAB",Key::Tab),("BACKSPACE",Key::Backspace),
        ("UP",Key::Up),("DOWN",Key::Down),("LEFT",Key::Left),("RIGHT",Key::Right),("PGUP",Key::PageUp),("PGDN",Key::PageDown),
        ("F1",Key::F1),("F4",Key::F4),("[",Key::LeftBracket),("]",Key::RightBracket),("EQUAL",Key::Equal),("MINUS",Key::Minus),
        (",",Key::Comma),(".",Key::Period),("BACKSLASH",Key::Backslash),
    ];
    let mut keys=Vec::new();
    if fields[2] != "NONE" {
        for name in fields[2].split('+') {
            keys.push(bindings.iter().find(|(n,_)| name.eq_ignore_ascii_case(n)).ok_or_else(invalid)?.1);
        }
    }
    let mouse=fields[5].parse::<u8>().map_err(|_| invalid())?;
    let (dx,dy,wheel)=(number(3)?,number(4)?,number(6)?);
    if mouse > 3 || dx.abs()>5000.0 || dy.abs()>5000.0 || wheel.abs()>100.0 {return Err(invalid());}
    let point=if fields.len()==9 {
        let (x,y)=(number(7)?,number(8)?);
        if !(0.0..640.0).contains(&x) || !(0.0..480.0).contains(&y) {return Err(invalid());}
        Some((x as f32,y as f32))
    } else {None};
    Ok((start,end,Snapshot{keys,mouse,delta:(dx as f32,dy as f32),wheel:wheel as f32,point}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    #[ignore = "requires a Windows desktop; verifies ShowCursor calls stay balanced"]
    fn cursor_visibility_does_not_drift_with_frame_count() {
        #[link(name="user32")]
        extern "system" {fn ShowCursor(show:i32)->i32;}
        let count=|| unsafe {let n=ShowCursor(1); ShowCursor(0); n};
        let mut window=Window::new("CyberMage cursor regression",64,64,minifb::WindowOptions::default()).unwrap();
        let baseline=count(); let mut input=Input::default();
        for _ in 0..100 {input.cursor_visibility(&mut window,true);}
        assert_eq!(count(),baseline-1);
        for _ in 0..100 {input.cursor_visibility(&mut window,false);}
        assert_eq!(count(),baseline);
        input.cursor_visibility(&mut window,true); input.release_cursor(&mut window);
        assert_eq!(count(),baseline);
    }
    #[test]
    fn modern_bindings_diagonals_suppression_pointer_and_mouse_delta_are_stable() {
        #[cfg(windows)] {
            assert_eq!(virtual_key(Key::W),Some(0x57)); assert_eq!(virtual_key(Key::F4),Some(0x73));
            assert_eq!(virtual_key(Key::Escape),Some(0x1B)); assert_eq!(virtual_key(Key::LeftCtrl),Some(0xA2));
        }
        assert!(parse_script("1:2:W+SHIFT:100:0:0:0").is_ok());
        for bad in ["1:0:W:0:0:0:0","1:2:BAD:0:0:0:0","1:2:W:NaN:0:0:0","1:2:W:0:0:4:0","1:2:W:0:0:0:0:640:0"] {assert!(parse_script(bad).is_err());}
        let mut input=Input::default(); let mut c=Cpu::new();
        c.wr32(rt(0x1E3C78),0x32); c.wr32(rt(0x1E3C78)+4,1);
        let shot=Snapshot{keys:vec![Key::W,Key::D,Key::Space,Key::LeftShift,Key::E,Key::M],mouse:3,delta:(10.0,-4.0),..Snapshot::default()};
        let (pad,keys)=input.update(shot.clone(),Pad::default(),true,&c);
        assert!(pad.connected && pad.down(gamepad::A|gamepad::L3|gamepad::X));
        assert!((pad.left.0.hypot(pad.left.1)-1.0).abs()<0.0001); assert!(keys[0x32]);
        assert_eq!((pad.lt,pad.rt),(1.0,1.0));
        input.suppress(); let (pad,keys)=input.update(shot,Pad::default(),false,&c);
        assert!(!pad.down(gamepad::A|gamepad::X)); assert!(!keys.contains(&true)); assert_eq!(input.mouse,0);
        input.update(Snapshot::default(),Pad::default(),false,&c);
        assert_eq!(input.edges(&Snapshot{keys:vec![Key::Space],..Snapshot::default()}),vec![Key::Space]);
        for render in [(640,480),(1920,1080),(5120,1440)] {
            assert_eq!(menu_point((960.0,540.0),(1920,1080),render),Some((320.0,240.0)));
            assert!(menu_point((-1.0,0.0),(1920,1080),render).is_none());
        }
        let path=std::env::temp_dir().join(format!("cybermage-desktop-{}",std::process::id()));
        let mut m=crate::machine::Machine::new(crate::vfs::Vfs::new(path.join("iso"),path.join("run")));
        let this=0x100000; c.wr32(this+0x130D04,1); m.desktop.look=(10.0,-4.0);
        look(&mut m,&mut c,this); let heading=c.rd16(this+0x7E65C+2);
        assert_eq!(heading,0u16.wrapping_sub(100)); assert_eq!(c.rd32(this+0xE33F8) as i32,-2);
        look(&mut m,&mut c,this); assert_eq!(c.rd16(this+0x7E65C+2),heading);
        m.display.sensitivity=0.25;
        for _ in 0..8 {m.desktop.look=(0.0,1.0); look(&mut m,&mut c,this);}
        assert_eq!(c.rd32(this+0xE33F8) as i32,-1);
        let mut input=Input::default();
        let fire=Snapshot{keys:vec![Key::W],mouse:1,..Snapshot::default()};
        let mut owned=gamepad::Keys::default(); let mut queue=std::collections::VecDeque::new();
        let (p,_)=input.update(fire.clone(),Pad::default(),true,&c);
        owned.controller(&mut queue,p,&c,true);
        let (p,_)=input.update(fire,Pad::default(),false,&c);
        owned.controller(&mut queue,p,&c,false);
        assert!(!p.down(gamepad::UP)); assert_eq!(owned.menu_mouse(),0);
        m.desktop=Input::default(); m.mouse.x=222; m.mouse.y=111;
        let point=Snapshot{point:Some((320.0,240.0)),..Snapshot::default()};
        m.desktop.update(point,Pad::default(),false,&c); pointer(&mut m,false);
        assert_eq!((m.mouse.x,m.mouse.y),(222,111),"Entering a menu must preserve its programmed cursor");
        let moved=Snapshot{point:Some((330.0,340.0)),..Snapshot::default()};
        m.desktop.update(moved.clone(),Pad::default(),false,&c); pointer(&mut m,false);
        assert_eq!((m.mouse.x,m.mouse.y),(330,170)); assert_eq!(m.display.ui_pointer,(330,340));
        m.mouse.x=500; m.mouse.y=50; // original keyboard navigation repositions the DOS cursor
        m.desktop.update(moved,Pad::default(),false,&c); pointer(&mut m,false);
        assert_eq!((m.mouse.x,m.mouse.y),(500,50),"An unmoved host mouse must not undo keyboard navigation");
        let hand=Snapshot{mouse:2,..Snapshot::default()}; let mut input=Input::default();
        input.update(hand.clone(),Pad::default(),true,&c);
        let (p,_)=input.update(hand,Pad::default(),false,&c);
        assert_eq!(p.lt,1.0,"Hand aiming must remain held in original state 3");
        std::fs::remove_dir_all(path).unwrap();
    }
}
