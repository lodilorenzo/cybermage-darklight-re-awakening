//! Embedded Nuked-SC55. User supplies hash-verified Roland ROMs; no ROMs are bundled.
use std::{ffi::{c_char, c_void, CStr, CString}, path::Path, ptr::NonNull};

extern "C" {
    fn cm_sc55_open(directory: *const c_char, error: *mut c_char, size: usize) -> *mut c_void;
    fn cm_sc55_close(handle: *mut c_void);
    fn cm_sc55_send(handle: *mut c_void, data: *const u8, size: usize);
    fn cm_sc55_mix(handle: *mut c_void, stereo: *mut f32, frames: usize, rate: f64) -> bool;
}

pub struct Sc55(NonNull<c_void>);

impl Sc55 {
    pub fn open(directory: &Path) -> Result<Self, String> {
        let path = directory.to_str().ok_or("SC-55 ROM path must be valid UTF-8")?;
        let path = CString::new(path).map_err(|_| "Invalid SC-55 ROM path")?;
        let mut error = [0 as c_char; 256];
        let handle = unsafe { cm_sc55_open(path.as_ptr(), error.as_mut_ptr(), error.len()) };
        NonNull::new(handle).map(Self).ok_or_else(|| {
            let message = unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy();
            if message.is_empty() { "Cannot initialize SC-55".into() } else { message.into_owned() }
        })
    }
    pub fn send(&mut self, data: &[u8]) {
        unsafe { cm_sc55_send(self.0.as_ptr(), data.as_ptr(), data.len()); }
    }
    pub fn mix(&mut self, stereo: &mut [f32], rate: f64) -> bool {
        assert!(stereo.len() % 2 == 0 && rate.is_finite() && rate > 0.0);
        unsafe { cm_sc55_mix(self.0.as_ptr(), stereo.as_mut_ptr(), stereo.len()/2, rate) }
    }
}
impl Drop for Sc55 {
    fn drop(&mut self) { unsafe { cm_sc55_close(self.0.as_ptr()); } }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_or_unrecognized_roms_are_rejected() {
        let dir = std::env::temp_dir().join(format!("cybermage-sc55-音-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(Sc55::open(&dir).is_err());
        std::fs::write(dir.join("sc55_rom1.bin"), [0; 32]).unwrap();
        assert!(Sc55::open(&dir).is_err());
        assert!(Sc55::open(&dir.join("missing")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    #[ignore = "requires user-supplied ROMs: CM_SC55_ROMS=path cargo test sc55_renders -- --ignored"]
    fn sc55_renders_audible_notes_at_host_rates() {
        let path = std::env::var_os("CM_SC55_ROMS").expect("Set CM_SC55_ROMS to your ROM directory");
        let mut synth = Sc55::open(Path::new(&path)).unwrap();
        for rate in [44100.0, 48000.0] {
            synth.send(&[0xC0, 0]);
            synth.send(&[0xB0, 7, 100]);
            synth.send(&[0x90, 60, 100]);
            let mut samples = vec![0.0; rate as usize]; // half a second, stereo
            assert!(synth.mix(&mut samples, rate));
            assert!(samples.iter().all(|s| s.is_finite()));
            assert!(samples.iter().any(|s| s.abs() > 0.001), "SC-55 produced silence");
            synth.send(&[0x80, 60, 0]);
            synth.send(&[0xB0, 120, 0]);
        }
        let mut audio = crate::audio::Audio::new();
        audio.output = true;
        audio.midi = Some(crate::midi_out::MidiOut::Sc55(synth));
        audio.mixer.voices[0] = Some(crate::audio::Voice {
            data: std::sync::Arc::new(vec![255; 1024]), pos: 0.0, rate: 11025.0,
            vol_l: 1.0, vol_r: 1.0, loops: 0, id: 1,
        });
        audio.midi.as_mut().unwrap().send(&[0x90, 60, 100]);
        for _ in 0..6 { audio.advance_music(1.0 / 60.0); }
        assert_eq!(audio.mixer.voices[0].as_ref().unwrap().pos, 0.0);
        assert!(audio.ring.lock().unwrap().iter().any(|s| s.abs() > 0.001));
        audio.advance(1.0 / 60.0);
        assert!(audio.mixer.voices[0].as_ref().unwrap().pos > 0.0);
    }
}
