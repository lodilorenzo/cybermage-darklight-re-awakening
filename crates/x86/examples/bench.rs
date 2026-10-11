use x86::*;
struct NullBus;
impl Bus for NullBus {
    fn port_in(&mut self, _: &mut Cpu, _: u16, _: u8) -> u32 { 0 }
    fn port_out(&mut self, _: &mut Cpu, _: u16, _: u8, _: u32) {}
    fn interrupt(&mut self, _: &mut Cpu, _: u8) -> Result<(), Fault> { Ok(()) }
    fn hle(&mut self, _: &mut Cpu, _: u16) -> Result<(), Fault> { Err(Fault::Stop) }
}
fn run(name: &str, body: &[u8]) {
    let mut cpu = Cpu::new();
    let mut code = vec![0xB9, 0x00, 0xE1, 0xF5, 0x05];
    code.extend_from_slice(body);
    code.push(0x49);
    let back = -((body.len() + 3) as i32) as i8 as u8;
    code.extend_from_slice(&[0x75, back, 0x0F, 0xFF, 0, 0]);
    cpu.set_code_range(0x1000, 0x1000);
    cpu.write_bytes(0x1000, &code);
    cpu.eip = 0x1000;
    cpu.r[ESI] = 0x10000;
    cpu.r[EDI] = 0x20000;
    cpu.r[ESP] = 0x8000;
    let t = std::time::Instant::now();
    let mut bus = NullBus;
    loop {
        if cpu.run(&mut bus, 1 << 40).is_err() { break; }
    }
    let el = t.elapsed().as_secs_f64();
    println!("{:20} {:.1} MIPS ({:.2} ns/instr)", name, cpu.icount as f64 / el / 1e6, el * 1e9 / cpu.icount as f64);
}
fn main() {
    run("dec/jnz", &[]);
    run("inc ebx", &[0x43]);
    run("add eax,ebx", &[0x01, 0xD8]);
    run("mov [edi],eax", &[0x89, 0x07]);
    run("add eax,[esi+ebx*4]", &[0x03, 0x04, 0x9E]);
    run("and ebx,imm32", &[0x81, 0xE3, 0xFF, 0, 0, 0]);
    let mut cpu = Cpu::new();
    // mov ecx,N ; L: add eax,[esi+ebx*4] ; mov [edi],eax ; inc ebx ; and ebx,255 ; dec ecx ; jnz L ; 0F FF
    let code: Vec<u8> = vec![
        0xB9, 0x00, 0xE1, 0xF5, 0x05, // mov ecx, 100_000_000
        0x03, 0x04, 0x9E,             // add eax,[esi+ebx*4]
        0x89, 0x07,                   // mov [edi],eax
        0x43,                         // inc ebx
        0x81, 0xE3, 0xFF, 0, 0, 0,    // and ebx,255
        0x49,                         // dec ecx
        0x75, 0xF2,                   // jnz L
        0x0F, 0xFF, 0, 0,
    ];
    cpu.set_code_range(0x1000, 0x1000);
    cpu.write_bytes(0x1000, &code);
    cpu.eip = 0x1000;
    cpu.r[ESI] = 0x10000;
    cpu.r[EDI] = 0x20000;
    cpu.r[ESP] = 0x8000;
    let t = std::time::Instant::now();
    let mut bus = NullBus;
    loop {
        if cpu.step(&mut bus).is_err() { break; }
    }
    let el = t.elapsed().as_secs_f64();
    println!("{} instr in {:.2}s = {:.1} MIPS", cpu.icount, el, cpu.icount as f64 / el / 1e6);
}
