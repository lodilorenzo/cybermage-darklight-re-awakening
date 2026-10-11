"""Paired CPU-only comparison of compose against a saved display.rs.
python tools/bench_compose.py out/performance-check/until-9/display-before.rs
Uses rustc and stdlib only; no game assets or writable game files.
"""
from pathlib import Path
import argparse
import subprocess
import tempfile


def function(text, name):
    start = text.index(f"fn {name}(")
    brace = text.index("{", start)
    depth = 1
    end = brace + 1
    while depth:
        depth += (text[end] == "{") - (text[end] == "}")
        end += 1
    return text[start:end]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    args = parser.parse_args()
    project = Path(__file__).resolve().parents[1]
    current = (project / "crates/game/src/display.rs").read_text()
    before = args.before.read_text()
    source = "const SIDE_W: usize = 534; const CLEAR: u32 = 0xFF00_0000;\nstruct SideArt { left: Vec<u32>, right: Vec<u32> }\n"
    source += function(current, "ui_rect") + "\n"

    source += "#[inline(never)]\n" + function(before, "compose").replace("fn compose(", "fn before(", 1) + "\n"
    source += "#[inline(never)]\n" + function(current, "compose").replace("fn compose(", "fn after(", 1) + "\n"
    source += r'''
#[cfg(windows)]
fn thread_time() -> u64 {
    #[link(name="kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> *mut std::ffi::c_void;
        fn GetThreadTimes(thread: *mut std::ffi::c_void, creation: *mut u64, exit: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
    }
    let (mut creation,mut exit,mut kernel,mut user) = (0,0,0,0);
    assert_ne!(unsafe {GetThreadTimes(GetCurrentThread(),&mut creation,&mut exit,&mut kernel,&mut user)},0);
    kernel+user
}
#[cfg(not(windows))]
fn thread_time() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64 / 100
}
fn main() {
    let legacy: Vec<u32> = (0..640*480).map(|i| i as u32 * 31).collect();
    let hud: Vec<bool> = (0..640*480).map(|i| i/640 > 425 || (i/640 > 340 && (250..390).contains(&(i%640)))).collect();
    let pal = std::array::from_fn(|i| i as u32 * 0x010101);
    for (w,h,rw,rh) in [(1920,1080,1280,720),(1920,1080,1072,604),(3840,2160,1280,720),(3840,2160,1216,684),(3840,2160,640,360),(5120,1440,1880,528)] {
        let world: Vec<u8> = (0..rw*rh).map(|i| i as u8).collect();
        let mut out = Vec::new();
        let mut times = [Vec::new(),Vec::new()];
        let mut cpu_times = [Vec::new(),Vec::new()];
        for repeat in 0..9 { for which in [repeat%2,1-repeat%2] {
            let f = if which == 0 { before } else { after };
            let cpu_start = thread_time();
            let start = std::time::Instant::now();
            for _ in 0..100 {
                f(&legacy,(640,480),(w,h),&world,(rw,rh),&hud,(0,0,w,h),true,&pal,None,&mut out);
                std::hint::black_box(&out);
            }
            times[which].push(start.elapsed().as_secs_f64()*10.0);
            cpu_times[which].push((thread_time()-cpu_start) as f64 / 1_000_000.0);
        } }
        before(&legacy,(640,480),(w,h),&world,(rw,rh),&hud,(0,0,w,h),true,&pal,None,&mut out);
        let expected = out.clone();
        after(&legacy,(640,480),(w,h),&world,(rw,rh),&hud,(0,0,w,h),true,&pal,None,&mut out);
        assert_eq!(out,expected);
        for t in times.iter_mut().chain(&mut cpu_times) {t.sort_by(f64::total_cmp);}
        println!("{w}x{h} world {rw}x{rh}: CPU before {:.3}, after {:.3} ms; wall before {:.3}, after {:.3} ms",cpu_times[0][4],cpu_times[1][4],times[0][4],times[1][4]);
    }
}
'''
    with tempfile.TemporaryDirectory(prefix="cybermage-compose-") as directory:
        path = Path(directory)
        (path / "bench.rs").write_text(source)
        exe = path / "bench.exe"
        subprocess.run(["rustc", "--edition=2021", "-O", str(path / "bench.rs"), "-o", str(exe)], check=True)
        subprocess.run([str(exe)], check=True)


if __name__ == "__main__":
    main()
