use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=../../assets/cybermage.ico");
    let target = env::var("TARGET").unwrap();
    if !target.contains("windows") { return; }
    let icon = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../assets/cybermage.ico").canonicalize().unwrap();
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let rc = out.join("cybermage.rc");
    let resource = out.join(if target.ends_with("msvc") { "cybermage.res" } else { "cybermage.o" });
    let icon = icon.to_str().unwrap().replace('\\', "/").trim_start_matches("//?/").to_owned();
    fs::write(&rc, format!("1 ICON \"{icon}\"\n")).unwrap();
    let status = if target.ends_with("msvc") {
        cc::windows_registry::find(&target, "rc.exe").expect("Install the Windows SDK resource compiler")
            .arg("/nologo").arg("/fo").arg(&resource).arg(&rc).status()
    } else {
        Command::new(env::var("WINDRES").unwrap_or_else(|_| "windres".into()))
            .arg(&rc).args(["-O", "coff", "-o"]).arg(&resource).status()
    }.expect("Run the Windows resource compiler");
    assert!(status.success(), "Windows icon resource compilation failed");
    println!("cargo:rustc-link-arg={}", resource.display());
}
