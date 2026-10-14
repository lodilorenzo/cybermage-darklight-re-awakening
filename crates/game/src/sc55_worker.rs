//! One owner of the non-Send SC-55 handle. The game queues timestamped PCM/MIDI;
//! the worker publishes completed audio, never synthesizing in the device callback.
use crate::{audio::Audio, sc55::Sc55};
use std::{path::Path, sync::{Arc, Mutex, atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{self, SyncSender}}, thread::JoinHandle, collections::VecDeque};

#[cfg(windows)]
struct AudioPriority(*mut std::ffi::c_void);
#[cfg(windows)]
#[link(name = "avrt")]
extern "system" {
    fn AvSetMmThreadCharacteristicsW(task: *const u16, index: *mut u32) -> *mut std::ffi::c_void;
    fn AvSetMmThreadPriority(handle: *mut std::ffi::c_void, priority: i32) -> i32;
    fn AvRevertMmThreadCharacteristics(handle: *mut std::ffi::c_void) -> i32;
}
#[cfg(windows)]
impl AudioPriority {
    fn new() -> Self {
        let mut index = 0;
        let task: Vec<u16> = "Audio\0".encode_utf16().collect();
        let handle = unsafe { AvSetMmThreadCharacteristicsW(task.as_ptr(),&mut index) };
        if handle.is_null() { eprintln!("SC-55 audio scheduling unavailable: {}",std::io::Error::last_os_error()); }
        else { unsafe { AvSetMmThreadPriority(handle,1); } }
        Self(handle)
    }
}
#[cfg(windows)]
impl Drop for AudioPriority {
    fn drop(&mut self) { if !self.0.is_null() { unsafe { AvRevertMmThreadCharacteristics(self.0); } } }
}

struct Job {
    pcm: Vec<f32>,
    events: Vec<(usize, Vec<u8>)>, // host-frame offsets, in send order
    rate: f64,
}

pub struct Worker {
    sender: Option<SyncSender<Job>>,
    thread: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    queued: Arc<AtomicUsize>,
    ring: Arc<Mutex<VecDeque<f32>>>,
    events: Vec<(usize, Vec<u8>)>,
    frames: usize,
    pub dropped_frames: usize,
}

impl Worker {
    pub fn open(path: &Path, ring: Arc<Mutex<VecDeque<f32>>>) -> Result<Self,String> {
        let path = path.to_path_buf();
        let (sender,receiver) = mpsc::sync_channel::<Job>(512);
        let (ready,started) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let queued = Arc::new(AtomicUsize::new(0));
        let (exit,bad,pending,output) = (stop.clone(),failed.clone(),queued.clone(),ring.clone());
        let thread = std::thread::Builder::new().name("SC-55 audio".into()).spawn(move || {
            // Construct and destroy the FFI handle on this thread; no unsafe Send impl.
            let mut synth = match Sc55::open(&path) {
                Ok(s) => { if ready.send(Ok(())).is_err() { return; } s }
                Err(e) => { let _ = ready.send(Err(e)); return; }
            };
            #[cfg(windows)]
            let _priority = AudioPriority::new();
            while let Ok(mut job) = receiver.recv() {
                if exit.load(Ordering::Acquire) { break; }
                let frames = job.pcm.len()/2;
                let mut events = job.events.into_iter().peekable();
                for (i,chunk) in job.pcm.chunks_mut(128).enumerate() {
                    if exit.load(Ordering::Acquire) { return; }
                    while events.peek().is_some_and(|(frame,_)| *frame <= i*64) {
                        synth.send(&events.next().unwrap().1);
                    }
                    if !synth.mix(chunk,job.rate) {
                        bad.store(true,Ordering::Release);
                        return;
                    }
                    for s in chunk { *s = s.clamp(-1.0,1.0); }
                }
                // Transfer queued -> ready frames under the same lock used by submit.
                let mut ring = output.lock().unwrap();
                Audio::publish(&mut ring,&job.pcm,job.rate);
                pending.fetch_sub(frames,Ordering::Release);
            }
        }).map_err(|e| format!("Cannot start SC-55 audio worker: {e}"))?;
        let worker = Self { sender: Some(sender),thread: Some(thread),stop,failed,queued,ring,
            events: Vec::new(),frames: 0,dropped_frames: 0 };
        started.recv().map_err(|_| "SC-55 audio worker failed during startup".to_string())??;
        Ok(worker)
    }
    pub fn send(&mut self, bytes: &[u8]) {
        self.events.push((self.frames,bytes.to_vec()));
    }
    pub fn advance(&mut self, frames: usize) -> bool {
        self.frames += frames;
        !self.failed.load(Ordering::Acquire)
    }
    pub fn submit(&mut self, pcm: &[f32], rate: f64) -> bool {
        assert!(pcm.len()%2 == 0 && rate.is_finite() && rate>0.0);
        let frames = pcm.len()/2;
        assert_eq!(frames,self.frames);
        self.frames = 0;
        if self.failed.load(Ordering::Acquire) { return false; }
        {
            let mut ring = self.ring.lock().unwrap();
            // Bound queued + ready audio to the existing 150 ms. As before, trim
            // stale *completed* PCM, never MIDI or unrendered synthesis time.
            let pending = self.queued.load(Ordering::Acquire);
            let max = (rate*0.15) as usize;
            // ponytail: >150 ms pending synthesis disables music rather than blocking
            // gameplay; add timestamped resynchronization if long-stall recovery is needed.
            if pending + frames > max {
                eprintln!("SC-55 queue budget: queued={pending} new={frames} frames");
                return false;
            }
            let excess = (ring.len()/2).saturating_sub(max-pending-frames);
            ring.drain(..excess*2);
            self.dropped_frames += excess;
            self.queued.fetch_add(frames,Ordering::Release);
        }
        let job = Job {pcm: pcm.to_vec(),events: std::mem::take(&mut self.events),rate};
        if self.sender.as_ref().unwrap().try_send(job).is_err() {
            self.queued.fetch_sub(frames,Ordering::Release);
            return false;
        }
        true
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true,Ordering::Release);
        self.sender.take(); // wake an idle receiver
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires user ROMs: CM_SC55_ROMS=path; validates asynchronous PCM, overload and shutdown"]
    fn worker_matches_synchronous_pcm_and_stops_without_stale_writes() {
        let path = std::env::var_os("CM_SC55_ROMS").expect("Set CM_SC55_ROMS");
        let path = Path::new(&path);
        let ring = Arc::new(Mutex::new(VecDeque::new()));
        let mut worker = Worker::open(path,ring.clone()).unwrap();
        let mut sync = Sc55::open(path).unwrap();
        for rate in [44100.0,48000.0] {
            let mut pcm: Vec<f32> = (0..2048).map(|i| (i%17) as f32/100.0).collect();
            let clean = pcm.clone();
            for (i,chunk) in pcm.chunks_mut(128).enumerate() {
                let events: &[&[u8]] = match i {
                    0 => &[&[0xC0,0],&[0xB0,7,100],&[0x90,60,100]],
                    8 => &[&[0x80,60,0],&[0x90,67,100]],
                    12 => &[&[0xB0,120,0]],
                    _ => &[],
                };
                for e in events { sync.send(e); worker.send(e); }
                assert!(sync.mix(chunk,rate));
                assert!(worker.advance(chunk.len()/2));
                for s in chunk { *s = s.clamp(-1.0,1.0); }
            }
            assert!(worker.submit(&clean,rate));
            let deadline = std::time::Instant::now()+std::time::Duration::from_secs(5);
            while ring.lock().unwrap().len() != pcm.len() {
                assert!(std::time::Instant::now()<deadline,"worker failed to publish audio");
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert_eq!(ring.lock().unwrap().drain(..).collect::<Vec<_>>(),pcm);
        }
        // Full output trims stale PCM; excess new work rejects without losing note-offs.
        ring.lock().unwrap().resize(48000*2,0.0);
        assert!(worker.advance(64));
        assert!(worker.submit(&[0.0;128],48000.0)); // stale ready PCM is bounded
        assert!(worker.dropped_frames>0);
        assert!(worker.advance(48000));
        assert!(!worker.submit(&vec![0.0;96000],48000.0)); // excessive new work is rejected
        drop(worker);
        ring.lock().unwrap().clear();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(ring.lock().unwrap().is_empty());
        // Opening a bad replacement must leave the old output untouched.
        ring.lock().unwrap().push_back(0.5);
        assert!(Worker::open(&path.join("missing"),ring.clone()).is_err());
        assert_eq!(ring.lock().unwrap().iter().copied().collect::<Vec<_>>(),[0.5]);
        let mut audio = Audio::new();
        audio.output = true;
        audio.mixer.voices[0] = Some(crate::audio::Voice {data: Arc::new(vec![255;4096]),pos: 0.0,
            rate: 11025.0,vol_l: 1.0,vol_r: 1.0,loops: 0,id: 1});
        audio.set_music_output(true,path).unwrap();
        assert!(audio.is_sc55());
        let lead = audio.ring.lock().unwrap().len();
        audio.ring.lock().unwrap().clear();
        audio.ring.lock().unwrap().push_back(0.25);
        audio.prime_output();
        audio.prime_output();
        {
            let ring = audio.ring.lock().unwrap();
            assert_eq!(ring.len(),lead,"resume must not grow the scheduling lead");
            assert_eq!(*ring.back().unwrap(),0.25,"resume lost completed PCM");
            assert!(ring.iter().take(lead-1).all(|&p| p==0.0));
        }
        audio.advance_music(0.05);
        assert_eq!(audio.mixer.voices[0].as_ref().unwrap().pos,0.0);
        let deadline = std::time::Instant::now()+std::time::Duration::from_secs(5);
        while audio.ring.lock().unwrap().len()<=lead {
            assert!(std::time::Instant::now()<deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        audio.advance(0.02);
        assert!(audio.mixer.voices[0].as_ref().unwrap().pos>0.0);
        assert!(audio.set_music_output(true,&path.join("missing")).is_err());
        assert!(audio.is_sc55(),"failed replacement lost the old synth");
        #[cfg(windows)] {
            audio.set_music_output(false,path).unwrap();
            assert!(!audio.is_sc55());
            assert!(audio.ring.lock().unwrap().is_empty());
            std::thread::sleep(std::time::Duration::from_millis(20));
            assert!(audio.ring.lock().unwrap().is_empty(),"old worker wrote after switching");
            audio.set_music_output(true,path).unwrap();
            assert!(audio.is_sc55());
        }
    }
}
