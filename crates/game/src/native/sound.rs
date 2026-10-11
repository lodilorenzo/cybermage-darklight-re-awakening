//! Native replacement of the `Sounds` class (cmhmi40.cpp), which wrapped the
//! HMI SOS 4.0 digital + MIDI libraries.  Samples go to the native mixer,
//! songs to the native HMI player, and the game's 40 Hz system tick
//! (CM_ISR8) is driven by a native timer event.

use crate::audio::Voice;
use crate::le::rt;
use crate::machine::Machine;
use std::sync::Arc;
use x86::*;

const CM_ISR8: u32 = 0x067C06;
const SOUND_SAMPLE_SLOTS: usize = 16;
/// `SOSSampleID[16]` in the original data segment (sound number per slot)
const SOS_SAMPLE_ID: u32 = 0x2072E0;
/// `SOSSampleHandle[16]` (-1 = free)
const SOS_SAMPLE_HANDLE: u32 = 0x2072A0;
const SYS_TICK_HZ: f64 = 40.0;

pub fn install(m: &mut Machine, cpu: &mut Cpu) {
    m.hook(cpu, 0x06654C, "Sounds::BootSOS", boot_sos);
    m.hook(cpu, 0x066954, "Sounds::CMsosInitSystem", init_system);
    m.hook(cpu, 0x066B84, "Sounds::CMsosUnInitSystem", uninit_system);
    m.hook(cpu, 0x066C00, "Sounds::CMsosSetPatchBanks", ret0);
    m.hook(cpu, 0x066C3C, "Sounds::CMsosPlaySample", play_sample);
    m.hook(cpu, 0x066CE8, "Sounds::SampleStop", sample_stop);
    m.hook(cpu, 0x066D20, "Sounds::SampleDone", sample_done);
    m.hook(cpu, 0x066D4C, "Sounds::SetSampleVolume", set_sample_volume);
    m.hook(cpu, 0x066D8C, "Sounds::SetSamplePanVol", set_sample_panvol);
    m.hook(cpu, 0x066DE8, "Sounds::SampleMasterVolume", sample_master_volume);
    m.hook(cpu, 0x066DEC, "Sounds::CMsosPlaySong", play_song);
    m.hook(cpu, 0x066E3C, "Sounds::MidiMasterVolume", midi_master_volume);
    m.hook(cpu, 0x066E4C, "Sounds::SongPause", song_pause);
    m.hook(cpu, 0x066E5C, "Sounds::SongResume", song_resume);
    m.hook(cpu, 0x066E6C, "Sounds::SongDone", song_done);
    m.hook(cpu, 0x066E84, "Sounds::SongEnd", song_end);
}

fn start(m: &mut Machine, cpu: &mut Cpu, this: u32, no_sound: bool) {
    // `this->soundOn` is the first dword of the Sounds object
    cpu.wr32(this, if no_sound || !m.audio.enabled { 0 } else { 1 });
    for i in 0..SOUND_SAMPLE_SLOTS as u32 {
        cpu.wr32(rt(SOS_SAMPLE_HANDLE) + i * 4, 0xFFFF_FFFF);
        cpu.wr32(rt(SOS_SAMPLE_ID) + i * 4, 0);
    }
    if !m.events.iter().any(|e| e.active && e.addr == rt(CM_ISR8)) {
        m.add_timer_event(cpu, SYS_TICK_HZ, rt(CM_ISR8));
    }
}

/// long Sounds::BootSOS(long noSound, char *path)
fn boot_sos(m: &mut Machine, cpu: &mut Cpu) -> Result<(), Fault> {
    let this = cpu.r[EAX];
    let no_sound = cpu.r[EDX] == 1;
    start(m, cpu, this, no_sound);
    cpu.r[EAX] = 0;
    cpu.ret_near(0);
    Ok(())
}

/// long Sounds::CMsosInitSystem(uint, uint, uint, uint, uint, uint, uint)
fn init_system(m: &mut Machine, cpu: &mut Cpu) -> Result<(), Fault> {
    let this = cpu.r[EAX];
    let no_sound = cpu.r[EDX] == 1;
    start(m, cpu, this, no_sound);
    cpu.r[EAX] = 0;
    cpu.ret_near(0x10);
    Ok(())
}

fn uninit_system(m: &mut Machine, cpu: &mut Cpu) -> Result<(), Fault> {
    m.audio.mixer.voices.iter_mut().for_each(|v| *v = None);
    m.audio.stop_music();
    cpu.r[EAX] = 0;
    cpu.ret_near(0);
    Ok(())
}

fn ret0(_m: &mut Machine, cpu: &mut Cpu) -> Result<(), Fault> {
    cpu.r[EAX] = 0;
    cpu.ret_near(0);
    Ok(())
}

fn pan_vol(vol: u32, pan: u32) -> (f32, f32) {
    let v = (vol.min(0x7FFF) as f32) / 32767.0;
    // SOS pan: 0x0000 = left, 0x8000 = centre, 0xFFFF = right
    let p = (pan & 0xFFFF) as f32 / 65535.0;
    let l = (1.0 - p).min(0.5) * 2.0;
    let r = p.min(0.5) * 2.0;
    (v * l, v * r)
}

/// long Sounds::CMsosPlaySample(long slot, long length, uchar *data,
///                              int channels, long bits, long vol, long loops, long pan)
fn play_sample(m: &mut Machine, cpu: &mut Cpu) -> Result<(), Fault> {
    let this = cpu.r[EAX];
    let slot = cpu.r[EDX] as usize;
    let len = cpu.r[EBX] as usize;
    let ptr = cpu.r[ECX];
    let _channels = cpu.stack_arg(0);
    let _bits = cpu.stack_arg(1);
    let vol = cpu.stack_arg(2) as u16 as i16 as i32;
    let loops = cpu.stack_arg(3) as i32;
    let pan = cpu.stack_arg(4);
    let result;
    if cpu.rd32(this) == 0 || slot >= SOUND_SAMPLE_SLOTS || len == 0 {
        result = 0xFFFF_FFFF;
    } else {
        let id = cpu.rd32(rt(SOS_SAMPLE_ID) + slot as u32 * 4);
        let data = Arc::new(cpu.read_bytes(ptr, len.min(16 << 20)).to_vec());
        let (vl, vr) = pan_vol(vol.max(0) as u32, pan);
        let mx = &mut m.audio.mixer;
        mx.voices[slot] = Some(Voice { data, pos: 0.0, rate: 11025.0, vol_l: vl, vol_r: vr, loops, id });
            cpu.wr32(rt(SOS_SAMPLE_HANDLE) + slot as u32 * 4, slot as u32);
        result = slot as u32;
    }
    cpu.r[EAX] = result;
    cpu.ret_near(0x14);
    Ok(())
}

fn slot_arg(cpu: &Cpu) -> usize {
    (cpu.r[EDX] as usize).min(SOUND_SAMPLE_SLOTS - 1)
}

fn sample_stop(m: &mut Machine, cpu: &mut Cpu) -> Result<(), Fault> {
    let slot = slot_arg(cpu);
    m.audio.mixer.voices[slot] = None;
    cpu.wr32(rt(SOS_SAMPLE_HANDLE) + slot as u32 * 4, 0xFFFF_FFFF);
    cpu.ret_near(0);
    Ok(())
}

fn sample_done(m: &mut Machine, cpu: &mut Cpu) -> Result<(), Fault> {
    let slot = slot_arg(cpu);
    let playing = m.audio.mixer.voices[slot].is_some();
    if !playing {
        cpu.wr32(rt(SOS_SAMPLE_HANDLE) + slot as u32 * 4, 0xFFFF_FFFF);
    }
    cpu.r[EAX] = if playing { 0 } else { 1 };
    cpu.ret_near(0);
    Ok(())
}

/// void Sounds::SetSampleVolume(int slot, long vol)
fn set_sample_volume(m: &mut Machine, cpu: &mut Cpu) -> Result<(), Fault> {
    let slot = slot_arg(cpu);
    let vol = (cpu.r[EBX] as i32).clamp(0, 0x7FFF) as f32 / 32767.0;
    if let Some(v) = &mut m.audio.mixer.voices[slot] {
        let cur = (v.vol_l.max(v.vol_r)).max(1e-6);
        v.vol_l = v.vol_l / cur * vol;
        v.vol_r = v.vol_r / cur * vol;
    }
    cpu.ret_near(0);
    Ok(())
}

/// void Sounds::SetSamplePanVol(int slot, long pan, long vol)
fn set_sample_panvol(m: &mut Machine, cpu: &mut Cpu) -> Result<(), Fault> {
    let slot = slot_arg(cpu);
    let pan = cpu.r[EBX];
    let vol = (cpu.r[ECX] as i32).clamp(0, 0x7FFF) as u32;
    let (l, r) = pan_vol(vol, pan);
    if let Some(v) = &mut m.audio.mixer.voices[slot] {
        v.vol_l = l;
        v.vol_r = r;
    }
    cpu.ret_near(0);
    Ok(())
}

fn sample_master_volume(_m: &mut Machine, cpu: &mut Cpu) -> Result<(), Fault> {
    cpu.ret_near(0);
    Ok(())
}

/// long Sounds::CMsosPlaySong(uchar *song)  (song pointer in EDX)
fn play_song(m: &mut Machine, cpu: &mut Cpu) -> Result<(), Fault> {
    let ptr = cpu.r[EDX];
    if std::env::var("CM_DUMP_SONGS").is_ok() {
        let len = 0xA000.min(crate::music::song_len(cpu, ptr));
        let name = format!("out/song_{:08X}.hmi", cpu.rd32(ptr.wrapping_add(0xE8)));
        std::fs::write(name, cpu.read_bytes(ptr, len as usize)).ok();
    }
    let player = crate::music::Player::from_guest(cpu, ptr);
    let ok = player.is_some();
    if std::env::var("CM_TRACE_SONG").is_ok() {
        eprintln!("[music] play song @{:08X} ok={} t={:.2}", ptr, ok, m.time);
    }
    m.audio.stop_music();
    m.audio.mixer.music = player;
    cpu.r[EAX] = if ok { 0 } else { 0xFFFF_FFFF };
    cpu.ret_near(0);
    Ok(())
}

fn midi_master_volume(m: &mut Machine, cpu: &mut Cpu) -> Result<(), Fault> {
    // SOS MIDI uses seven-bit volume, unlike the 15-bit sample mixer.
    m.audio.mixer.music_vol = (cpu.r[EDX] & 0x7F) as f32 / 127.0;
    if std::env::var("CM_TRACE_SONG").is_ok() {
        eprintln!("[music] master volume {}/127", cpu.r[EDX] & 0x7F);
    }
    cpu.ret_near(0);
    Ok(())
}

fn song_pause(m: &mut Machine, cpu: &mut Cpu) -> Result<(), Fault> {
    if let Some(p) = &mut m.audio.mixer.music {
        p.paused = true;
        let midi = &mut m.audio.midi;
        p.all_notes_off(&mut |msg| crate::audio::Audio::midi_send(midi, msg));
    }
    cpu.ret_near(0);
    Ok(())
}

fn song_resume(m: &mut Machine, cpu: &mut Cpu) -> Result<(), Fault> {
    if let Some(p) = &mut m.audio.mixer.music {
        p.paused = false;
    }
    cpu.ret_near(0);
    Ok(())
}

fn song_done(m: &mut Machine, cpu: &mut Cpu) -> Result<(), Fault> {
    let done = m.audio.mixer.music.as_ref().map(|p| p.done).unwrap_or(true);
    cpu.r[EAX] = done as u32;
    cpu.ret_near(0);
    Ok(())
}

fn song_end(m: &mut Machine, cpu: &mut Cpu) -> Result<(), Fault> {
    m.audio.stop_music();
    cpu.ret_near(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midi_volume_uses_seven_bits_and_emits_audible_notes() {
        let path = std::env::temp_dir().join(format!("cybermage-music-{}", std::process::id()));
        let mut m = Machine::new(crate::vfs::Vfs::new(path.join("iso"), path.join("run")));
        let mut c = Cpu::new();
        c.r[ESP] = 0x30000;
        for volume in [0, 75, 127, 255] {
            c.r[EDX] = volume;
            c.push32(0x123456);
            midi_master_volume(&mut m, &mut c).unwrap();
            assert_eq!(m.audio.mixer.music_vol, (volume & 127) as f32 / 127.0);
            assert_eq!(c.eip, 0x123456);
            assert_eq!(c.r[ESP], 0x30000);
        }
        let mut player = crate::music::Player::new(test_song()).unwrap();
        let mut messages = Vec::new();
        player.advance(1.0 / 120.0, 75.0 / 127.0, &mut |msg| messages.push(msg.to_vec()));
        assert!(messages.contains(&vec![0xB3, 7, 75]));
        assert!(messages.contains(&vec![0x93, 60, 100]));
        assert!(!player.done);
        std::fs::remove_dir_all(path).unwrap();
    }

    /// One note (key 60, two ticks long) on channel 3, then end of track.
    fn test_song() -> Vec<u8> {
        let mut song = vec![0; 0x300];
        song[..18].copy_from_slice(b"HMI-MIDISONG061595");
        song[0xD4] = 120;
        song[0xE4] = 1;
        song[0xE8..0xEC].copy_from_slice(&0x100u32.to_le_bytes());
        song[0x100..0x104].copy_from_slice(&0x120u32.to_le_bytes());
        song[0x177..0x17B].copy_from_slice(&0x90u32.to_le_bytes());
        song[0x19B] = 3;
        song[0x1B0..0x1BD].copy_from_slice(&[0, 0xB3, 7, 127, 0, 0x93, 60, 100, 2, 10, 0xFF, 0x2F, 0]);
        song
    }

    #[test]
    fn paused_guest_music_keeps_playing_and_releases_its_notes() {
        // Host pages stop the guest; the frontend then calls advance_music each frame.
        let mut audio = crate::audio::Audio::new();
        let mut player = crate::music::Player::new(test_song()).unwrap();
        let mut messages = Vec::new();
        player.advance(1.0 / 120.0, 1.0, &mut |msg| messages.push(msg.to_vec()));
        assert!(messages.contains(&vec![0x93, 60, 100])); // the note is sounding
        audio.mixer.music = Some(player);
        for _ in 0..30 { audio.advance_music(1.0 / 60.0); }
        // The note-off and the end of the song were reached while "paused".
        assert!(audio.mixer.music.as_ref().unwrap().done);
        assert!(audio.ring.lock().unwrap().is_empty()); // sound effects stay paused
    }
}
