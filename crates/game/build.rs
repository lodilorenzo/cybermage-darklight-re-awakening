use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=../../assets/cybermage.ico");
    let target = env::var("TARGET").unwrap();
    build_sc55(&target);
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

fn build_sc55(target: &str) {
    let backend = PathBuf::from("../../third_party/nuked-sc55/src/backend");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed={}", backend.display());
    println!("cargo:rerun-if-changed=src/sc55_bridge.cpp");
    fs::write(out.join("config.h"), "#pragma once\n#define NUKED_ENABLE_ASIO 0\n#define NUKED_ENABLE_DECODER2 1\n#define NUKED_VERSION \"0.8.0\"\n#define NUKED_SOURCE \"CyberMage embedded\"\n#define NUKED_VERSION_MAJOR 0\n#define NUKED_VERSION_MINOR 8\n#define NUKED_VERSION_PATCH 0\n#include <cstdio>\nvoid Cfg_WriteVersionInfo(FILE*);\n").unwrap();
    let mut build = cc::Build::new();
    build.cpp(true).opt_level(3).warnings(false).include(&backend).include(&out)
        .flag(if target.ends_with("msvc") { "/std:c++latest" } else { "-std=c++23" })
        .file("src/sc55_bridge.cpp");
    if target.ends_with("msvc") {
        // Let MSVC inline the firmware/PCM hot path across backend translation units.
        build.flag("/EHsc").flag("/GL");
        println!("cargo:rustc-link-arg=/LTCG");
    }
    for dir in [&backend, &backend.join("decoder2")] {
        let mut sources: Vec<_> = fs::read_dir(dir).unwrap().map(|p| p.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "cpp")).collect();
        sources.sort();
        build.files(sources);
    }
    build.compile("nuked_sc55");
    cc::Build::new().opt_level(3).warnings(false).include(backend.join("sha"))
        .file(backend.join("sha/sha224-256.c")).compile("nuked_sc55_sha");
}
