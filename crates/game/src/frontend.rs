//! Desktop/Xbox controls, native options/help and audio output.

use crate::{desktop, display, gamepad::{self, Pad}, Config, Emu};
use minifb::{Key, ScaleMode, Window, WindowOptions};
use std::time::{Duration, Instant};
use x86::Fault;

pub(crate) fn create_window(d: &display::Display, position: Option<(isize,isize)>) -> Result<Window,minifb::Error> {
    let (w,h) = d.size();
    let mut window = Window::new(crate::rebrand::NAME,w,h,
        WindowOptions { resize: !d.fullscreen, title: !d.fullscreen, borderless: d.fullscreen,
            none: d.fullscreen, scale_mode: ScaleMode::AspectRatioStretch, ..WindowOptions::default() })?;
    #[cfg(windows)]
    set_window_icon(&window).map_err(|e| minifb::Error::WindowCreate(e.to_string()))?;
    window.set_target_fps(0);
    window.update(); // register minifb's HWND state before synchronous WM_SIZE
    if let Some((x,y)) = position { window.set_position(x,y); }
    if d.fullscreen {
        #[cfg(windows)] {
            use std::ffi::c_void;
            #[link(name = "user32")]
            extern "system" {
                fn SetWindowPos(hwnd: *mut c_void, after: *mut c_void, x: i32, y: i32, w: i32, h: i32, flags: u32) -> i32;
            }
            let r = monitor_rect(&window).map_err(|e| minifb::Error::WindowCreate(e.to_string()))?;
            if unsafe { SetWindowPos(window.get_window_handle(),std::ptr::null_mut(),r.left,r.top,
                r.right-r.left,r.bottom-r.top,0x0004 | 0x0020 | 0x0040) } == 0 {
                return Err(minifb::Error::WindowCreate(std::io::Error::last_os_error().to_string()));
            }
            window.update();
        }
        #[cfg(not(windows))]
        return Err(minifb::Error::WindowCreate("fullscreen is currently supported on Windows".into()));
    }
    Ok(window)
}

// Load the executable's embedded icon, not a file beside the installation.
#[cfg(windows)]
fn set_window_icon(window: &Window) -> std::io::Result<()> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" { fn GetModuleHandleW(name: *const u16) -> *mut c_void; }
    #[link(name = "user32")]
    extern "system" {
        fn LoadImageW(module: *mut c_void, name: *const u16, kind: u32, w: i32, h: i32, flags: u32) -> *mut c_void;
        fn GetSystemMetrics(index: i32) -> i32;
        fn SendMessageW(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> isize;
    }
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        for (kind, metric) in [(0,49), (1,11)] { // ICON_SMALL / ICON_BIG
            let icon = LoadImageW(module,1 as *const u16,1,
                GetSystemMetrics(metric),GetSystemMetrics(metric+1),0x8000); // LR_SHARED
            if icon.is_null() { return Err(std::io::Error::last_os_error()); }
            SendMessageW(window.get_window_handle(),0x0080,kind,icon as isize); // WM_SETICON
        }
    }
    Ok(())
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default,Clone,Copy)]
struct ScreenRect { left: i32, top: i32, right: i32, bottom: i32 }

#[cfg(windows)]
fn monitor_rect(window: &Window) -> std::io::Result<ScreenRect> {
    use std::ffi::c_void;
    #[repr(C)]
    struct MonitorInfo { size: u32, monitor: ScreenRect, work: ScreenRect, flags: u32 }
    #[link(name = "user32")]
    extern "system" {
        fn MonitorFromWindow(hwnd: *mut c_void, flags: u32) -> *mut c_void;
        fn GetMonitorInfoW(monitor: *mut c_void, info: *mut MonitorInfo) -> i32;
    }
    let monitor = unsafe { MonitorFromWindow(window.get_window_handle(),2) };
    let mut info = MonitorInfo { size: std::mem::size_of::<MonitorInfo>() as u32,
        monitor: ScreenRect::default(), work: ScreenRect::default(), flags: 0 };
    if monitor.is_null() || unsafe { GetMonitorInfoW(monitor,&mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(info.monitor)
}

pub fn run(cfg: &Config) {
    let mut emu = Emu::new(cfg);
    let _stream = start_audio(&mut emu);
    emu.m.audio.midi = crate::midi_out::MidiOut::open();
    if emu.m.audio.midi.is_none() { eprintln!("warning: no MIDI output device; music disabled"); }
    let mut window = create_window(&emu.m.display,None).expect("window");
    let mut buf = Vec::new();
    let mut keys = gamepad::Keys::default();
    let mut actions = gamepad::Actions::default();
    let mut previous = Pad::default();
    let mut previous_physical = Pad::default();
    let mut last_nav = (0,0);
    let mut next_nav = Instant::now();
    let mut pointer_last = Instant::now();
    let mut window_mode = (emu.m.display.mode,emu.m.display.fullscreen);
    let mut page_buf = Vec::new();
    let mut deferred_action = None;
    let mut exit_stage = 0;
    let _timer = TimerResolution::new();
    let frame = Duration::from_secs_f64(1.0 / 60.0);
    let mut next_frame = Instant::now() + frame;
    let mut presentation = Duration::from_millis(2);
    let mut world_work = Duration::ZERO;
    let mut vsync_failed = false;
    while window.is_open() {
        let start = Instant::now();
        let dt = start.duration_since(pointer_last).as_secs_f32();
        pointer_last = start;
        actions.update(&mut emu.m.kbd_queue);
        let active = window.is_active();
        let physical_pad = if active { gamepad::poll(emu.m.display.deadzone) } else { Pad::default() };
        let capture = active && gamepad::gameplay(&emu.m,&emu.cpu) && !actions.open && deferred_action.is_none();
        let snapshot = emu.m.desktop.sample(&mut window,capture,emu.m.display.size());
        let keyboard_edges = emu.m.desktop.edges(&snapshot);
        let click = emu.m.desktop.click(&snapshot);
        let wheel = snapshot.wheel;
        let (mut pad, desktop_keys) = emu.m.desktop.update(snapshot,physical_pad,capture,&emu.cpu);
        let mut ran = false;
        let serial = emu.m.display.frame_serial;
        if !active { pad = Pad::default(); }
        if !emu.m.display.help_open { emu.m.display.help_keyboard = !emu.m.desktop.controller_guide; }
        let edge = |button| pad.down(button) && !previous.down(button);
        let mut nav = (0,0);
        if pad.down(gamepad::LEFT) || pad.left.0 < -0.5 { nav.0 = -1; }
        if pad.down(gamepad::RIGHT) || pad.left.0 > 0.5 { nav.0 = 1; }
        if pad.down(gamepad::UP) || pad.left.1 > 0.5 { nav.1 = -1; }
        if pad.down(gamepad::DOWN) || pad.left.1 < -0.5 { nav.1 = 1; }
        if !capture && wheel != 0.0 { nav.1 = if wheel > 0.0 {-1} else {1}; }
        let nav_pulse = nav != (0,0) && (wheel != 0.0 || nav != last_nav || start >= next_nav);
        let nav_repeat = nav_pulse && nav == last_nav;
        if nav_pulse { next_nav = start + Duration::from_millis(if nav != last_nav {350} else {150}); }
        last_nav = nav;
        if !active || !pad.connected {
            actions.release_now(&mut emu.m.kbd_queue);
            keys.clear(&mut emu.m.kbd_queue);
            emu.m.desktop.suppress();
            emu.realtime_last = None;
        }
        emu.m.gamepad = Pad { guest_this: emu.m.gamepad.guest_this, apply_stub: emu.m.gamepad.apply_stub,
            last_time: emu.m.gamepad.last_time, pitch_fraction: emu.m.gamepad.pitch_fraction,
            jump_held: emu.m.gamepad.jump_held && pad.down(gamepad::A), ..pad };
        gamepad::controller_input(&emu.m,&mut emu.cpu);
        display::prepare_menu(&mut emu.m,&emu.cpu);
        let playing = gamepad::gameplay(&emu.m,&emu.cpu);
        if deferred_action.is_some() {
            if playing {
                actions.selected = deferred_action.take().unwrap();
                actions.choose(&mut emu.m.kbd_queue,&emu.cpu);
            } else if active && pad.connected && menu_exit(&mut exit_stage,emu.m.display.options_original,
                emu.cpu.rd32(crate::le::rt(0x203F44)) != 0) {
                gamepad::tap(&mut emu.m.kbd_queue,0x01,false);
            }
        }
        if !guide_input(&mut emu,&mut keys,pad,previous,&keyboard_edges) {
            if emu.m.display.art.ready() && edge(gamepad::BACK) && !emu.m.display.options_open {
                actions.open = !actions.open;
                emu.m.display.pending_game_options = false;
                emu.m.display.tab = if actions.open {2} else {0};
                clear_input(&mut emu,&mut keys,pad);
            }
            let host_page = emu.m.display.options_open || actions.open;
            let menu_visible = emu.m.display.options_original || emu.m.display.pending_game_options || host_page;
            let menu_buttons = pointer_buttons(&emu,&keys,host_page || (menu_visible && emu.m.desktop.controller_guide));
            gamepad::pointer(&mut emu.m,pad,playing && !host_page,menu_buttons,dt);
            desktop::pointer(&mut emu.m,playing && !host_page);
            let mut tab = if menu_visible { options_tab(emu.m.display.tab,physical_pad,previous_physical) } else { None };
            if menu_visible {
                let delta = i32::from(keyboard_edges.contains(&Key::E))-i32::from(keyboard_edges.contains(&Key::Q));
                if delta != 0 { tab = Some((emu.m.display.tab as i32+delta).rem_euclid(3) as usize); }
            }
            // Triggers/Q/E navigate tabs; X or a real click selects under the pointer.
            let point = if menu_visible && tab.is_none() && (edge(gamepad::X) || click) {
                Some((emu.m.display.ui_pointer.0 as f32,emu.m.display.ui_pointer.1 as f32))
            } else { None };
            if menu_visible {
                if let Some((x,y)) = point {
                    if (234.0..622.0).contains(&x) && (28.0..54.0).contains(&y) {
                        tab = Some(((x-234.0)/display::TAB_WIDTH as f32).min((display::MENU_TABS.len()-1) as f32) as usize);
                    }
                }
            }
            if let Some(tab) = tab { select_page(&mut emu,tab,&mut actions,&mut keys,pad); }
            if actions.open {
                emu.realtime_last = None;
                keys.desktop(&mut emu.m.kbd_queue,[false;256]);
                keys.suppress_controller(&mut emu.m.kbd_queue,pad);
                if nav_pulse { actions.navigate(nav.1); }
                if tab.is_none() && (edge(gamepad::LB) || keyboard_edges.contains(&Key::PageUp)) { actions.page(-1); }
                if tab.is_none() && (edge(gamepad::RB) || keyboard_edges.contains(&Key::PageDown)) { actions.page(1); }
                let choose = point.is_some_and(|(x,y)| actions.point(x,y));
                if edge(gamepad::START) {
                    actions.open = false;
                    gamepad::tap(&mut emu.m.kbd_queue,0x01,false);
                } else if edge(gamepad::B) {
                    actions.open = false;
                    emu.m.display.tab = 0;
                } else if edge(gamepad::A) || choose {
                    match actions.command() {
                        gamepad::Command::Help => {
                            actions.open = false;
                            emu.m.display.help_open = true;
                            emu.m.display.tab = 0;
                            clear_input(&mut emu,&mut keys,pad);
                        }
                        gamepad::Command::Options => select_page(&mut emu,1,&mut actions,&mut keys,pad),
                        gamepad::Command::Key(0x01) if emu.m.display.options_original => {
                            actions.open = false;
                            emu.m.display.tab = 0;
                            gamepad::tap(&mut emu.m.kbd_queue,0x01,false);
                        }
                        _ if emu.m.display.options_original => {
                            if emu.m.display.options_in_game {
                                deferred_action = Some(actions.selected);
                                exit_stage = 0;
                                actions.open = false;
                                emu.m.display.tab = 0;
                            }
                        }
                        _ => { actions.choose(&mut emu.m.kbd_queue,&emu.cpu); }
                    }
                }
            } else if emu.m.display.options_open {
                emu.realtime_last = None;
                keys.desktop(&mut emu.m.kbd_queue,[false;256]);
                keys.suppress_controller(&mut emu.m.kbd_queue,pad);
                if nav_pulse {
                    if nav.1 != 0 { display::settings_key(&mut emu.m.display,if nav.1 < 0 {0x48} else {0x50}); }
                    if nav.0 != 0 { emu.m.display.change(nav.0); }
                }
                if let Some((x,y)) = point {
                    let bottom = 140.0 + (display::SETTINGS_BACK + 1) as f32 * display::SETTINGS_ROW_HEIGHT as f32;
                    if (14.0..624.0).contains(&x) && (140.0..bottom).contains(&y) {
                        emu.m.display.row = ((y-140.0)/display::SETTINGS_ROW_HEIGHT as f32) as usize;
                        if emu.m.display.row < display::SETTINGS_BACK { emu.m.display.change(1); }
                        else { close_options(&mut emu,&mut keys,pad); }
                    }
                }
                if edge(gamepad::A) {
                    if emu.m.display.row < display::SETTINGS_BACK { emu.m.display.change(1); }
                    else { close_options(&mut emu,&mut keys,pad); }
                }
                if edge(gamepad::B) || edge(gamepad::START) { close_options(&mut emu,&mut keys,pad); }
            } else if active && pad.connected {
                if emu.m.display.pending_game_options {
                    keys.suppress_controller(&mut emu.m.kbd_queue,pad);
                } else {
                    keys.desktop(&mut emu.m.kbd_queue,desktop_keys);
                    emu.m.gamepad.game_buttons = keys.controller(&mut emu.m.kbd_queue,pad,&emu.cpu,playing);
                }
                if !emu.m.display.pending_game_options && !playing && (nav_repeat || wheel != 0.0) {
                    if nav.0 != 0 { gamepad::tap(&mut emu.m.kbd_queue,if nav.0 < 0 {0x4B} else {0x4D},true); }
                    if nav.1 != 0 { gamepad::tap(&mut emu.m.kbd_queue,if nav.1 < 0 {0x48} else {0x50},true); }
                }
                let menu_buttons = pointer_buttons(&emu,&keys,menu_visible && emu.m.desktop.controller_guide);
                gamepad::pointer(&mut emu.m,pad,playing,menu_buttons,0.0);
                desktop::pointer(&mut emu.m,playing);
                ran = true;
                match emu.run_frame(next_frame.checked_sub(presentation).unwrap_or(start).max(start)) {
                    Ok(()) => {}
                    Err(Fault::Stop) => {
                        if let Some(code) = emu.m.exit_code { eprintln!("program exited with code {code}"); break; }
                        if let Some((prog,args)) = emu.m.spawn_request.take() {
                            emu.m.desktop.release_cursor(&mut window);
                            crate::tgv::play_spawned(&mut window,&emu,&prog,&args);
                            pointer_last = Instant::now();
                            let held = gamepad::poll(emu.m.display.deadzone);
                            clear_input(&mut emu,&mut keys,held);
                            emu.cpu.set_cf(false); emu.cpu.r[x86::EAX] = 0;
                        }
                    }
                    Err(e) => { eprintln!("{}",emu.crash_report(e)); break; }
                }
            }
        }
        // The guest is paused this frame; keep its song playing in real time.
        if !ran { emu.m.audio.advance_music(f64::from(dt.min(0.1))); }
        let present_start = Instant::now();
        let size = if emu.m.display.help_open {
            display::present_help(&emu.m.display,&mut buf)
        } else if actions.open {
            actions.draw(&emu.m.display.art,&mut page_buf);
            if emu.m.display.options_original && !emu.m.display.options_in_game {
                emu.m.display.art.text(&mut page_buf,46,418,"GAME COMMANDS REQUIRE A RUNNING GAME",125);
            }
            if emu.m.desktop.controller_guide {emu.m.display.art.cursor(&mut page_buf,emu.m.display.ui_pointer.0,emu.m.display.ui_pointer.1);}
            display::present_menu(&emu.m.display,&page_buf,&mut buf)
        } else { display::render(&mut emu.m,&emu.cpu,&mut buf) };
        if let Err(e) = window.update_with_buffer(&buf,size.0,size.1) { eprintln!("presentation failed: {e}"); break; }
        presentation = present_start.elapsed();
        if ran && playing {
            world_work += start.elapsed();
            if emu.m.display.frame_serial != serial {
                emu.m.display.tune_resolution(world_work.as_secs_f64());
                world_work = Duration::ZERO;
            }
        } else { world_work = Duration::ZERO; }
        if !emu.m.display.options_open && window_mode != (emu.m.display.mode,emu.m.display.fullscreen) {
            emu.m.desktop.release_cursor(&mut window);
            match create_window(&emu.m.display,Some(window.get_position())) {
                Ok(new_window) => { window = new_window; clear_input(&mut emu,&mut keys,pad); }
                Err(e) => { eprintln!("cannot change window mode: {e}"); emu.m.display.fullscreen = window_mode.1; }
            }
            window_mode = (emu.m.display.mode,emu.m.display.fullscreen);
            pointer_last = Instant::now();
        }
        previous = pad; previous_physical = physical_pad;
        let vsync = emu.m.display.vsync && window.is_active();
        // Keep the compositor wait out of upload timing and dynamic-resolution work.
        let synced = sync_display(vsync);
        if vsync && !synced && !vsync_failed { eprintln!("V-Sync unavailable; falling back to 60 FPS"); }
        vsync_failed = vsync && !synced;
        if !synced { wait_until(next_frame); }
        let now = Instant::now();
        // V-Sync replaces the 60 Hz sleep, including on high-refresh displays.
        // The next deadline remains a CPU work budget, not a second frame cap.
        next_frame = if synced || now > next_frame + Duration::from_millis(1) { now + frame } else { next_frame + frame };
    }
    emu.m.desktop.release_cursor(&mut window);
}

/// minifb presents with GDI on Windows: submit its batch, then wait for DWM's
/// next composition/present. Works for windowed and borderless fullscreen.
pub(crate) fn sync_display(enabled: bool) -> bool {
    if !enabled { return false; }
    #[cfg(windows)] {
        #[link(name = "gdi32")]
        extern "system" { fn GdiFlush() -> i32; }
        #[link(name = "dwmapi")]
        extern "system" { fn DwmFlush() -> i32; }
        unsafe { GdiFlush() != 0 && DwmFlush() >= 0 }
    }
    #[cfg(not(windows))]
    { false }
}

/// Windows' default sleep quantum can miss a 16.67 ms frame deadline.
struct TimerResolution;
#[cfg(windows)]
#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
    fn timeEndPeriod(period: u32) -> u32;
}
impl TimerResolution {
    fn new() -> Self {
        #[cfg(windows)] unsafe { timeBeginPeriod(1); }
        Self
    }
}
impl Drop for TimerResolution {
    fn drop(&mut self) {
        #[cfg(windows)] unsafe { timeEndPeriod(1); }
    }
}
fn wait_until(deadline: Instant) {
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        if left > Duration::from_millis(1) { std::thread::sleep(left - Duration::from_millis(1)); }
        else { std::hint::spin_loop(); }
    }
}

pub fn guide_input(emu: &mut Emu, keys: &mut gamepad::Keys, pad: Pad, previous: Pad, edges: &[Key]) -> bool {
    if edges.contains(&Key::F1) {
        emu.m.display.help_keyboard = true; emu.m.display.help_open = true;
        clear_input(emu,keys,pad);
    }
    if !emu.m.display.help_open {return false;}
    emu.realtime_last = None;
    keys.desktop(&mut emu.m.kbd_queue,[false;256]);
    keys.suppress_controller(&mut emu.m.kbd_queue,pad);
    let edge = |button| pad.down(button) && !previous.down(button);
    if edge(gamepad::LEFT) || edge(gamepad::RIGHT) || edge(gamepad::LB) || edge(gamepad::RB) {
        emu.m.display.help_keyboard = !emu.m.display.help_keyboard;
    }
    if edge(gamepad::B) || edge(gamepad::A) || edge(gamepad::START) {
        display::settings_key(&mut emu.m.display,0x01); clear_input(emu,keys,pad);
    }
    true
}

fn pointer_buttons(emu: &Emu, keys: &gamepad::Keys, reserved: bool) -> u16 {
    let _ = emu;
    if reserved {0} else {keys.menu_mouse()}
}

fn options_tab(tab: usize, pad: Pad, previous: Pad) -> Option<usize> {
    let next = pad.rt > 0.12 && previous.rt <= 0.12;
    let back = pad.lt > 0.12 && previous.lt <= 0.12;
    let delta = i32::from(next) - i32::from(back);
    (delta != 0).then(|| (tab as i32 + delta).rem_euclid(display::MENU_TABS.len() as i32) as usize)
}

fn menu_exit(stage: &mut u8, options: bool, menu: bool) -> bool {
    let send = (*stage == 0 && options) || (*stage == 1 && !options && menu);
    if send { *stage += 1; }
    send
}

fn clear_input(emu: &mut Emu, keys: &mut gamepad::Keys, pad: Pad) {
    emu.m.desktop.suppress();
    emu.m.gamepad.game_buttons = 0;
    keys.clear(&mut emu.m.kbd_queue);
    keys.suppress_controller(&mut emu.m.kbd_queue,pad);
    emu.m.kbd_queue.clear();
    for sc in 0..128 { emu.cpu.wr8(crate::le::rt(0x207C54)+sc,0); }
    emu.cpu.wr8(crate::le::rt(0x207CD4),1);
    emu.m.mouse.buttons = 0;
    emu.m.mouse.mickey_x = 0; emu.m.mouse.mickey_y = 0;
    emu.realtime_last = None;
}

fn select_page(emu: &mut Emu, tab: usize, actions: &mut gamepad::Actions, keys: &mut gamepad::Keys, pad: Pad) {
    if emu.m.display.options_open { close_options(emu,keys,pad); }
    actions.open = tab == 2;
    emu.m.display.options_open = tab == 1;
    emu.m.display.pending_game_options = tab == 0 && !emu.m.display.options_original;
    emu.m.display.tab = tab;
    emu.m.display.row = 0;
    clear_input(emu,keys,pad);
}

fn close_options(emu: &mut Emu, keys: &mut gamepad::Keys, pad: Pad) {
    emu.m.display.options_open = false;
    emu.m.display.tab = 0;
    clear_input(emu,keys,pad);
    if let Err(e) = emu.m.display.save(&emu.m.vfs.overlay.join("NATIVE.CNF")) { eprintln!("cannot save native settings: {e}"); }
}

#[cfg(test)]
mod tests {
    #[test]
    fn disabled_vsync_does_not_wait_for_the_display() {
        assert!(!super::sync_display(false));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires a Windows desktop; checks compositor-synchronized presentation"]
    fn vsync_waits_for_compositor_presentation() {
        let mut d = crate::display::Display::new(std::path::Path::new(""));
        d.mode = 0;
        let pixels = vec![0x123456;640*480];
        for fullscreen in [false,true] {
            d.fullscreen = fullscreen;
            let mut window = super::create_window(&d,None).unwrap();
            let start = std::time::Instant::now();
            for _ in 0..16 {
                window.update_with_buffer(&pixels,640,480).unwrap();
                assert!(super::sync_display(true));
            }
            assert!(start.elapsed() > std::time::Duration::from_millis(16));
        }
    }

    #[test]
    fn triggers_cycle_all_options_tabs_once_and_bumpers_only_page_actions() {
        use crate::gamepad::{Actions, Pad, LB, RB};
        let neutral = Pad::default();
        let right = Pad { rt: 1.0, ..neutral };
        let left = Pad { lt: 1.0, ..neutral };
        for tab in 0..crate::display::MENU_TABS.len() {
            assert_eq!(super::options_tab(tab,right,neutral),Some((tab+1)%3));
            assert_eq!(super::options_tab(tab,left,neutral),Some((tab+2)%3));
            assert_eq!(super::options_tab(tab,right,right),None);
            assert_eq!(super::options_tab(tab,left,left),None);
            assert_eq!(super::options_tab(tab,neutral,right),None);
            assert_eq!(super::options_tab(tab,Pad { buttons: LB | RB, ..neutral },neutral),None);
            assert_eq!(super::options_tab(tab,Pad { rt: 1.0, lt: 1.0, ..neutral },neutral),None);
            assert_eq!(super::options_tab(tab,Pad { rt: 0.12, ..neutral },neutral),None);
        }
        let mut actions = Actions::default();
        actions.page(1); assert_eq!(actions.selected,8);
        actions.page(1); assert_eq!(actions.selected,16);
        actions.page(1); assert_eq!(actions.selected,0);
        actions.page(-1); assert_eq!(actions.selected,16);
        // Switching main tabs doesn't reset the remembered Actions subpage.
        assert_eq!(super::options_tab(2,right,neutral),Some(0));
        assert_eq!(super::options_tab(0,left,neutral),Some(2));
        assert_eq!(actions.selected,16);
    }
    #[test]
    #[ignore = "requires original game data; runs isolated title/gameplay menu transitions"]
    fn back_actions_trigger_tabs_open_real_game_options_without_exiting() {
        use crate::{display, gamepad, le::rt, machine::Machine, vfs::Vfs, Emu};
        let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        let game = project.join("iso/CYBERMAG");
        let root = std::env::temp_dir().join(format!("cybermage-back-tabs-{}",std::process::id()));
        fn advance(emu: &mut Emu) {
            gamepad::controller_input(&emu.m,&mut emu.cpu);
            match emu.run_for(0.05) {
                Ok(()) => {}
                Err(x86::Fault::Stop) if emu.m.spawn_request.take().is_some() => {
                    emu.cpu.set_cf(false); emu.cpu.r[x86::EAX] = 0;
                }
                Err(e) => panic!("{}",emu.crash_report(e)),
            }
        }
        for in_game in [false,true] {
            let run = root.join(if in_game {"game"} else {"title"});
            let mut cpu = x86::Cpu::new();
            let mut m = Machine::new(Vfs::new(game.clone(),run.clone()));
            crate::write_default_config(&run);
            std::fs::copy(project.join("run/CMDATA.CNF"),run.join("CMDATA.CNF")).unwrap();
            m.load_program(&mut cpu,&std::fs::read(game.join("CM_HI.EXE")).unwrap(),"").unwrap();
            crate::native::install(&mut m,&mut cpu);
            m.gamepad.connected = true;
            let mut emu = Emu { cpu, m, ips: 100_000_000.0, cfg_trace: false, watches: Vec::new(),
                profile: None, counter: None, profile_start: 0.0, scene_override: None, vehicle_anywhere: None, realtime_last: None };
            for frame in 0..280 {
                if in_game && frame == 180 { gamepad::tap(&mut emu.m.kbd_queue,0x1C,false); }
                advance(&mut emu);
            }
            display::prepare_menu(&mut emu.m,&emu.cpu);
            assert!(emu.m.display.art.ready());
            assert!(!emu.m.display.options_original);
            let camera = emu.cpu.read_bytes(rt(0x1819C4),24).to_vec();
            let mut actions = gamepad::Actions::default(); actions.open = true; actions.selected = 16;
            let mut keys = gamepad::Keys::default();
            emu.m.display.tab = 2; // Back opened Actions, not original Options
            let pad = gamepad::Pad { connected: true, rt: 1.0, ..gamepad::Pad::default() };
            let tab = super::options_tab(2,pad,gamepad::Pad::default()).unwrap();
            super::select_page(&mut emu,tab,&mut actions,&mut keys,pad);
            assert!(emu.m.display.pending_game_options);
            assert!(!gamepad::gameplay(&emu.m,&emu.cpu));
            for _ in 0..100 {
                advance(&mut emu);
                if emu.m.display.options_original { break; }
            }
            assert!(emu.m.display.options_original,"Game tab returned to the world instead of opening Options");
            assert!(!emu.m.display.pending_game_options);
            assert_eq!(emu.m.display.options_in_game,in_game);
            assert_eq!(emu.m.display.tab,0);
            assert_eq!(emu.cpu.read_bytes(rt(0x1819C4),24),camera);
            assert!(emu.m.exit_code.is_none());
            for _ in 0..10 { advance(&mut emu); } // allow the actual guest page to paint
            let mut pixels = Vec::new();
            let size = display::render(&mut emu.m,&emu.cpu,&mut pixels);
            crate::write_bmp(&project.join(format!("out/native-check/back-trigger-game-options-{in_game}.bmp")),size.0,size.1,&pixels);
            for tab in [1,2,0,2,1,0] {
                super::select_page(&mut emu,tab,&mut actions,&mut keys,pad);
                assert!(emu.m.display.options_original);
                assert!(!emu.m.display.pending_game_options);
                assert_eq!(emu.m.display.options_open,tab == 1);
                assert_eq!(actions.open,tab == 2);
                assert_eq!(actions.selected,16);
            }
            gamepad::tap(&mut emu.m.kbd_queue,0x01,false);
            for _ in 0..100 {
                advance(&mut emu);
                if !emu.m.display.options_original { break; }
            }
            assert!(!emu.m.display.options_original);
            assert!(emu.m.exit_code.is_none());
            // Also reproduce Actions -> Display -> Game with two separate LT presses.
            actions.open = true; emu.m.display.tab = 2;
            let left = gamepad::Pad { lt: 1.0, rt: 0.0, ..pad };
            for from in [2,1] {
                let tab = super::options_tab(from,left,gamepad::Pad::default()).unwrap();
                super::select_page(&mut emu,tab,&mut actions,&mut keys,left);
                assert_eq!(emu.m.display.options_open,tab == 1);
            }
            assert!(emu.m.display.pending_game_options);
            for _ in 0..100 {
                advance(&mut emu);
                if emu.m.display.options_original { break; }
            }
            assert!(emu.m.display.options_original);
            assert!(!emu.m.display.pending_game_options);
            assert!(emu.m.exit_code.is_none());
            assert_eq!(emu.cpu.read_bytes(rt(0x1819C4),24),camera);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "requires a Windows desktop; briefly opens fullscreen and windowed windows"]
    fn fullscreen_window_fills_monitor_without_changing_render_resolution() {
        #[link(name = "user32")]
        extern "system" {
            fn SendMessageW(hwnd: *mut std::ffi::c_void, msg: u32, wparam: usize, lparam: isize) -> isize;
        }
        let assert_icons = |window: &minifb::Window| {
            for kind in [0,1] {
                assert_ne!(unsafe { SendMessageW(window.get_window_handle(),0x007F,kind,0) },0);
            }
        };
        let mut d = crate::display::Display::new(std::path::Path::new("__window_test_no_config__"));
        d.mode = 0; d.fullscreen = true;
        let mut window = super::create_window(&d,None).unwrap();
        assert_icons(&window);
        let r = super::monitor_rect(&window).unwrap();
        assert_eq!(window.get_size(),((r.right-r.left) as usize,(r.bottom-r.top) as usize));
        assert_eq!(window.get_position(),(r.left as isize,r.top as isize));
        window.update_with_buffer(&vec![0x123456;640*480],640,480).unwrap();
        assert_eq!(d.size(),(640,480));
        drop(window);
        d.fullscreen = false;
        let window = super::create_window(&d,None).unwrap();
        assert_eq!(window.get_size(),(640,480));
        assert_icons(&window);
    }
    #[test]
    fn utility_actions_exit_each_menu_once_even_with_slow_presentation() {
        let mut stage = 0;
        assert!(super::menu_exit(&mut stage,true,true));
        assert!(!super::menu_exit(&mut stage,true,true));
        assert!(super::menu_exit(&mut stage,false,true));
        for _ in 0..1000 { assert!(!super::menu_exit(&mut stage,false,true)); }
        assert!(!super::menu_exit(&mut stage,false,false));
        stage = 0;
        assert!(super::menu_exit(&mut stage,true,true));
        for _ in 0..1000 { assert!(!super::menu_exit(&mut stage,false,false)); }
    }
}

fn start_audio(emu: &mut Emu) -> Option<cpal::Stream> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let dev = host.default_output_device()?;
    let cfg = dev.default_output_config().ok()?;
    let rate = cfg.sample_rate();
    let channels = cfg.channels() as usize;
    emu.m.audio.mixer.out_rate = rate as f64;
    emu.m.audio.output = true;
    let ring = emu.m.audio.ring.clone();
    let stream = dev.build_output_stream(cfg.config(),move |data: &mut [f32],_| {
        let frames = data.len()/channels;
        let mut r = ring.lock().unwrap();
        for f in 0..frames {
            let l = r.pop_front().unwrap_or(0.0);
            let rr = r.pop_front().unwrap_or(l);
            for c in 0..channels { data[f*channels+c] = if c&1 == 0 {l} else {rr}; }
        }
    },|e| eprintln!("audio error: {e}"),None).ok()?;
    stream.play().ok()?;
    Some(stream)
}
