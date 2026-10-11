"""Paired native span benchmarks (rustc + stdlib, Windows CPU timing).
python tools/bench_columns.py out/performance-check/until-9/wall-before-unroll.rs
python tools/bench_columns.py --flat out/performance-check/until-9/image1-before-flat-normalize.rs
"""
from pathlib import Path
import argparse
import subprocess
import tempfile
from bench_compose import function


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("--flat", action="store_true", help="benchmark horizontal floor/ceiling spans")
    args = parser.parse_args()
    project = Path(__file__).resolve().parents[1]
    filename = "crates/game/src/native/image1.rs" if args.flat else "crates/game/src/native/image1/wall.rs"
    name = "flat_span" if args.flat else "scale_column_m"
    current = (project / filename).read_text()
    source = r'''
const MEM_SIZE: usize = 64 << 20;
struct Cpu {mem:Vec<u8>,wlog_on:bool}
impl Cpu {
    fn rd8(&self,a:u32)->u8 {self.mem[a as usize & (MEM_SIZE-1)]}
    fn wr8(&mut self,a:u32,v:u8) {self.mem[a as usize & (MEM_SIZE-1)]=v;}
}
thread_local! {static NATIVE_TARGET:std::cell::Cell<(u32,i32)>=const{std::cell::Cell::new((1280,480))};}
fn render_pitch()->u32 {NATIVE_TARGET.with(|s|s.get().0)}
'''
    source += function((project / "tools/bench_compose.py").read_text(), "thread_time") + "\n"
    if args.flat:
        source += "#[inline]\n" + function(current, "flat_texel") + "\n"
    for variant, text in [("before", args.before.read_text()), ("after", current)]:
        source += "#[inline(never)]\n" + function(text, name).replace(f"fn {name}(", f"fn {variant}(", 1) + "\n"
    source += (r'''
fn raster(c: &mut Cpu,f:fn(&mut Cpu,u32,usize,u32,u32,i32,i32,i32,i32,bool,u32),mask:bool,rot:u32) {
    for y in 0..720u32 {f(c,0x300000+y*1280,1280,0x100000,0x200000,i32::MAX.wrapping_sub((y as i32)*217171),(y as i32)*2456424,317923,-91289,mask,rot);}
}
fn main() {
    let mut c=Cpu {mem:vec![0;MEM_SIZE+16],wlog_on:false};
    for i in 0..16384 {c.mem[0x100000+i]=((i*17)>>2) as u8;}
    for i in 0..256 {c.mem[0x200000+i]=(i*3) as u8;}
    for rot in 0..4 {for mask in [false,true] {
        let mut wall=[Vec::new(),Vec::new()]; let mut cpu=[Vec::new(),Vec::new()];
        for repeat in 0..9 {for which in [repeat%2,1-repeat%2] {
            let f=if which==0 {before} else {after};
            let start=std::time::Instant::now(); let ticks=thread_time();
            for _ in 0..100 {raster(&mut c,f,mask,rot); std::hint::black_box(&c.mem);}
            wall[which].push(start.elapsed().as_secs_f64()*10.0);
            cpu[which].push((thread_time()-ticks) as f64/1_000_000.0);
        }}
        c.mem[0x300000..0x300000+1280*720].fill(123); raster(&mut c,before,mask,rot);
        let expected=c.mem[0x300000..0x300000+1280*720].to_vec();
        c.mem[0x300000..0x300000+1280*720].fill(123); raster(&mut c,after,mask,rot);
        assert_eq!(c.mem[0x300000..0x300000+1280*720],expected);
        for t in wall.iter_mut().chain(&mut cpu) {t.sort_by(f64::total_cmp);}
        println!("rotation {rot} mask {mask}: CPU {:.3}->{:.3}, wall {:.3}->{:.3} ms",cpu[0][4],cpu[1][4],wall[0][4],wall[1][4]);
    }}
}
''' if args.flat else r'''
fn main() {
    let mut c=Cpu {mem:vec![0;MEM_SIZE+16],wlog_on:false};
    for i in 0..0x20000 {c.mem[0x100000+i]=(i*13) as u8;}
    for i in 0..256 {c.mem[0x200000+i]=(i*17) as u8;}
    for step in [0x2000,0x8000,0x18000,0x50000] {for mask in [false,true] {
        let mut wall=[Vec::new(),Vec::new()]; let mut cpu=[Vec::new(),Vec::new()];
        for repeat in 0..9 {for which in [repeat%2,1-repeat%2] {
            let f=if which==0 {before} else {after};
            let start=std::time::Instant::now(); let ticks=thread_time();
            for _ in 0..50 {for x in 0..1280 {
                f(&mut c,0x200000,720,step,0x8000,0x100000+(x%128)*256,0x300000+719*1280+x,mask);
            } std::hint::black_box(&c.mem);}
            wall[which].push(start.elapsed().as_secs_f64()*20.0);
            cpu[which].push((thread_time()-ticks) as f64 / 500_000.0);
        }}
        c.mem[0x300000..0x300000+1280*720].fill(123);
        for x in 0..1280 {before(&mut c,0x200000,720,step,0x8000,0x100000+(x%128)*256,0x300000+719*1280+x,mask);}
        let expected=c.mem[0x300000..0x300000+1280*720].to_vec();
        c.mem[0x300000..0x300000+1280*720].fill(123);
        for x in 0..1280 {after(&mut c,0x200000,720,step,0x8000,0x100000+(x%128)*256,0x300000+719*1280+x,mask);}
        assert_eq!(c.mem[0x300000..0x300000+1280*720],expected);
        for t in wall.iter_mut().chain(&mut cpu) {t.sort_by(f64::total_cmp);}
        println!("step {step:05X} mask {mask}: CPU {:.3}->{:.3}, wall {:.3}->{:.3} ms",cpu[0][4],cpu[1][4],wall[0][4],wall[1][4]);
    }}
}
''')
    with tempfile.TemporaryDirectory(prefix="cybermage-columns-") as directory:
        path = Path(directory)
        (path / "bench.rs").write_text(source)
        exe = path / "bench.exe"
        subprocess.run(["rustc", "--edition=2021", "-O", str(path / "bench.rs"), "-o", str(exe)], check=True)
        subprocess.run([str(exe)], check=True)


if __name__ == "__main__":
    main()
