//! MIDI output: Windows MIDI mapper or embedded Nuked-SC55.

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

pub enum MidiOut {
    #[allow(dead_code)]
    Windows(isize),
    Sc55(crate::sc55::Sc55),
    Sc55Worker(crate::sc55_worker::Worker),
}

impl MidiOut {
    /// Open the default Windows MIDI output device (MIDI mapper).
    pub fn open() -> Option<Self> {
        #[cfg(windows)]
        unsafe {
            let mut h: isize = 0;
            if sys::midiOutOpen(&mut h, 0xFFFF_FFFF, 0, 0, 0) == 0 {
                return Some(Self::Windows(h));
            }
        }
        None
    }
    pub fn is_sc55(&self) -> bool { matches!(self, Self::Sc55(_) | Self::Sc55Worker(_)) }
    pub fn send(&mut self, msg: &[u8]) {
        match self {
            Self::Sc55(s) => s.send(msg),
            Self::Sc55Worker(w) => w.send(msg),
            Self::Windows(_handle) => {
                #[cfg(windows)]
                unsafe {
                    let mut w = 0u32;
                    for (i, b) in msg.iter().take(3).enumerate() { w |= (*b as u32) << (i * 8); }
                    sys::midiOutShortMsg(*_handle, w);
                }
            }
        }
    }
    pub fn mix(&mut self, stereo: &mut [f32], rate: f64) -> bool {
        match self {
            Self::Sc55(s) => s.mix(stereo, rate),
            Self::Sc55Worker(w) => w.advance(stereo.len()/2),
            Self::Windows(_) => true,
        }
    }
}

impl Drop for MidiOut {
    fn drop(&mut self) {
        if let Self::Windows(_handle) = self {
            #[cfg(windows)]
            unsafe {
                sys::midiOutReset(*_handle);
                sys::midiOutClose(*_handle);
            }
        }
    }
}
