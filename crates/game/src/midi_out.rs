//! General MIDI output through the Windows MIDI mapper (winmm).

#[cfg(windows)]
mod sys {
    #[link(name = "winmm")]
    extern "system" {
        pub fn midiOutOpen(phmo: *mut isize, device: u32, callback: usize, instance: usize, flags: u32) -> u32;
        pub fn midiOutShortMsg(hmo: isize, msg: u32) -> u32;
        pub fn midiOutReset(hmo: isize) -> u32;
        pub fn midiOutClose(hmo: isize) -> u32;
    }
}

pub struct MidiOut {
    #[allow(dead_code)]
    handle: isize,
}

impl MidiOut {
    /// Open the default MIDI output device (MIDI mapper).
    pub fn open() -> Option<MidiOut> {
        #[cfg(windows)]
        unsafe {
            let mut h: isize = 0;
            if sys::midiOutOpen(&mut h, 0xFFFF_FFFF, 0, 0, 0) == 0 {
                return Some(MidiOut { handle: h });
            }
        }
        None
    }

    pub fn send(&mut self, msg: &[u8]) {
        #[cfg(windows)]
        unsafe {
            let mut w = 0u32;
            for (i, b) in msg.iter().take(3).enumerate() {
                w |= (*b as u32) << (i * 8);
            }
            sys::midiOutShortMsg(self.handle, w);
        }
        let _ = msg;
    }

    pub fn reset(&mut self) {
        #[cfg(windows)]
        unsafe {
            sys::midiOutReset(self.handle);
        }
    }
}

impl Drop for MidiOut {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            sys::midiOutReset(self.handle);
            sys::midiOutClose(self.handle);
        }
    }
}
