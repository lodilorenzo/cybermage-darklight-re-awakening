//! Window-owned presentation: Vulkan on Windows, existing minifb/GDI fallback.
use crate::display::Display;
use minifb::Window;
use std::{ops::{Deref, DerefMut}, time::Duration};

#[derive(Default)]
pub struct Timing {
    /// Resource/acquire/presentation waits, not CPU composition or GPU execution.
    pub wait: Duration,
    /// Previous submission's GPU transfer/blit time (not the CPU world renderer).
    pub gpu_ms: Option<f64>,
}

impl Timing {
    pub fn gpu_work(&self) -> Duration { Duration::from_secs_f64(self.gpu_ms.unwrap_or(0.0)/1000.0) }
}

// Field order matters: destroy the Vulkan surface before destroying its HWND.
pub struct HostWindow {
    #[cfg(windows)]
    pub(crate) gpu: Option<crate::vulkan::Renderer>,
    window: Window,
    pub timing: Timing,
    pub failure: bool,
}
impl Deref for HostWindow { type Target = Window; fn deref(&self) -> &Window { &self.window } }
impl DerefMut for HostWindow { fn deref_mut(&mut self) -> &mut Window { &mut self.window } }

impl HostWindow {
    pub fn new(window: Window, d: &Display) -> Self {
        let mut host = Self {
            #[cfg(windows)] gpu: None,
            window, timing: Timing::default(), failure: false,
        };
        if d.vulkan {
            #[cfg(windows)] {
                match crate::vulkan::Renderer::new(host.window.get_window_handle()) {
                    Ok(gpu) => host.gpu = Some(gpu),
                    Err(e) => { eprintln!("Vulkan unavailable; using software: {e}"); host.failure = true; }
                }
            }
            #[cfg(not(windows))] { eprintln!("Vulkan presentation currently requires Windows; using software"); host.failure = true; }
        }
        #[cfg(windows)] if let Some(gpu) = &mut host.gpu {
            // Driver first-use allocation/blit setup can take hundreds of milliseconds.
            // Warm it before starting/resuming the guest/audio realtime clock. Never put
            // this temporary buffer into minifb (GDI retains its pointer after the call).
            let size = d.size();
            let blank = vec![0;size.0*size.1];
            if let Err(e) = gpu.present(&blank,size,host.window.get_size(),d.vsync,false).and_then(|_| gpu.sync()) {
                eprintln!("Vulkan initialization failed; using software: {e}");
                host.gpu = None; host.failure = true;
            }
        }
        eprintln!("presentation backend: {}",host.backend());
        host
    }
    pub fn is_vulkan(&self) -> bool {
        #[cfg(windows)] { self.gpu.is_some() }
        #[cfg(not(windows))] { false }
    }
    pub fn backend(&self) -> &'static str { if self.is_vulkan() { "Vulkan" } else { "Software" } }
    pub fn present(&mut self, pixels: &[u32], w: usize, h: usize, vsync: bool) -> Result<(),minifb::Error> {
        self.timing = Timing::default();
        #[cfg(windows)] if let Some(gpu) = &mut self.gpu {
            // No GDI framebuffer is installed on a Vulkan HWND; update only pumps input.
            self.window.update();
            if !self.window.is_open() { return Ok(()); }
            match gpu.present(pixels,(w,h),self.window.get_size(),vsync,false) {
                Ok(timing) => { self.timing = timing; return Ok(()); }
                Err(e) => {
                    eprintln!("Vulkan presentation failed; using software: {e}");
                    self.gpu = None; self.failure = true;
                }
            }
        }
        self.window.update_with_buffer(pixels,w,h)
    }
    pub fn sync(&self, enabled: bool) -> bool {
        if !self.window.is_open() { return false; }
        #[cfg(windows)] if let Some(gpu) = &self.gpu {
            if !enabled || !gpu.paced() { return false; }
            return match gpu.sync() {
                Ok(()) => true,
                Err(e) => { eprintln!("Vulkan present wait failed: {e}"); false }
            };
        }
        crate::frontend::sync_display(enabled)
    }
}

/// Match minifb's aspect-preserving, centered output, including odd margins.
pub(crate) fn fit_rect((sw,sh): (usize,usize), (w,h): (usize,usize)) -> (usize,usize,usize,usize) {
    if sw == 0 || sh == 0 || w == 0 || h == 0 { return (0,0,0,0); }
    let (rw,rh) = if (w as u64)*(sh as u64) <= (h as u64)*(sw as u64) {
        (w, ((w as u64)*(sh as u64)/(sw as u64)) as usize)
    } else { (((h as u64)*(sw as u64)/(sh as u64)) as usize,h) };
    ((w-rw)/2,(h-rh)/2,rw,rh)
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn aspect_fit_and_empty_windows() {
        assert_eq!(fit_rect((640,480),(1920,1080)),(240,0,1440,1080));
        assert_eq!(fit_rect((1920,1080),(640,480)),(0,60,640,360));
        assert_eq!(fit_rect((640,480),(641,480)),(0,0,640,480));
        assert_eq!(fit_rect((640,480),(0,0)),(0,0,0,0));
        assert_eq!(fit_rect((0,480),(640,480)),(0,0,0,0));
    }
}
