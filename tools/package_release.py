"""Build an asset-free Windows x64 portable ZIP from an explicit allowlist.
Usage: python tools/package_release.py [--no-build]
Never reads iso/, run/, screenshots, reference art, or arbitrary repository files.
"""
from pathlib import Path
import hashlib
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import zipfile
from check_icon import check_icon
from export_source import export

PROJECT = Path(__file__).resolve().parents[1]
TARGET = "x86_64-pc-windows-msvc"
NAME = "cybermage-windows-x64"
EXE = PROJECT / "target" / TARGET / "release/cybermage.exe"


def imported_dlls(path):
    """PE32+ import table (stdlib only); ensure portable builds use system DLLs."""
    data = path.read_bytes()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    assert data[pe:pe+4] == b"PE\0\0"
    opt = pe + 24
    assert struct.unpack_from("<H", data, opt)[0] == 0x20B, "Expected Windows x64 binary"
    assert struct.unpack_from("<H", data, opt+68)[0] == 2, "Expected Windows GUI subsystem (no console); rebuild with --features windows-gui"
    sections = opt + struct.unpack_from("<H", data, pe+20)[0]
    def offset(rva):
        for i in range(struct.unpack_from("<H", data, pe+6)[0]):
            virtual_size, address, size, raw = struct.unpack_from("<4I", data, sections+i*40+8)
            if address <= rva < address+max(size, virtual_size):
                return raw+rva-address
        raise AssertionError(f"Unmapped PE RVA {rva:#x}")
    rva = struct.unpack_from("<I", data, opt+112+8)[0]
    imports = []
    pos = offset(rva)
    while any(data[pos:pos+20]):
        start = offset(struct.unpack_from("<I", data, pos+12)[0])
        imports.append(data[start:data.index(0, start)].decode("ascii").lower())
        pos += 20
    assert not struct.unpack_from("<I", data, opt+112+13*8)[0], "Review delayed DLL imports"
    return imports


def licenses(stage):
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", TARGET], cwd=PROJECT))
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    root = next(p["id"] for p in packages.values() if p["name"] == "cybermage")
    seen, pending = set(), [root]
    while pending:
        pid = pending.pop()
        if pid in seen:
            continue
        seen.add(pid)
        pending.extend(d["pkg"] for d in nodes[pid]["deps"])
    inventory = ["Dependencies included in the Windows x64 build; their own licenses apply.", ""]
    for pkg in sorted((packages[pid] for pid in seen), key=lambda p: (p["name"], p["version"])):
        inventory.append(f'{pkg["name"]} {pkg["version"]}: {pkg["license"] or "see bundled license"}')
        if pkg["id"] in metadata["workspace_members"]:
            continue
        source = Path(pkg["manifest_path"]).parent
        texts = [p for p in source.iterdir() if p.is_file() and
                 p.name.lower().startswith(("license", "licence", "copying", "copyright", "notice"))]
        if pkg.get("license_file"):
            texts.append(source / pkg["license_file"])
        if not texts:
            fallback = PROJECT / "third_party" / f'{pkg["name"]}-{pkg["version"]}'
            texts = list(fallback.glob("*")) if fallback.is_dir() else []
        assert texts, f'No license text found for {pkg["name"]}'
        dest = stage / "licenses" / f'{pkg["name"]}-{pkg["version"]}'
        dest.mkdir(parents=True)
        for p in set(texts):
            shutil.copyfile(p, dest / p.name)
    # Rust's standard library is linked into the executable too.
    sysroot = Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip())
    rust_docs = sysroot / "share/doc/rust"
    assert (rust_docs / "COPYRIGHT-library.html").is_file(), "Rust standard-library copyright text not found"
    dest = stage / "licenses/rust-standard-library"
    dest.mkdir()
    shutil.copyfile(rust_docs / "COPYRIGHT-library.html", dest / "COPYRIGHT-library.html")
    shutil.copytree(rust_docs / "licenses", dest / "licenses")
    dest = stage / "licenses/nuked-sc55-0.8.0"
    dest.mkdir()
    for name in ("LICENSE", "NOTICE"):
        shutil.copyfile(PROJECT / "third_party/nuked-sc55" / name, dest / name)
    inventory.extend(["", "Nuked-SC55 0.8.0: GPL-2.0-or-later (see licenses/nuked-sc55-0.8.0).",
                      "Combined executable: GPL-2.0-or-later; corresponding source in SOURCE.zip.",
                      "Rust standard library: MIT OR Apache-2.0 (see licenses/rust-standard-library).",
                      "Microsoft static C runtime: distributed under the installed MSVC toolchain redistribution terms.",
                      "Windows system DLLs/MIDI sound bank: not bundled."])
    (stage / "DEPENDENCIES.txt").write_text("\n".join(inventory)+"\n", encoding="utf-8")


def main():
    if "--no-build" not in sys.argv:
        env = dict(os.environ)
        env["RUSTFLAGS"] = "-C target-feature=+crt-static"
        env["CARGO_PROFILE_RELEASE_DEBUG"] = "0"
        subprocess.run(["cargo", "build", "--locked", "--release", "--target", TARGET, "--features", "windows-gui"], cwd=PROJECT, env=env, check=True)
    assert EXE.is_file(), "Build the portable target first"
    imports = imported_dlls(EXE)
    check_icon(EXE)
    assert not any("vcruntime" in n or n.startswith("msvcp") for n in imports), f"External VC++ runtime: {imports}"
    system = {"kernel32.dll", "user32.dll", "gdi32.dll", "advapi32.dll", "ole32.dll", "oleaut32.dll",
              "winmm.dll", "avrt.dll", "comdlg32.dll", "shell32.dll", "xinput9_1_0.dll", "ntdll.dll",
              "bcrypt.dll", "bcryptprimitives.dll", "combase.dll", "ws2_32.dll", "rpcrt4.dll", "dwmapi.dll", "shcore.dll", "mmdevapi.dll"}
    assert all(n in system or n.startswith("api-ms-win-") for n in imports), f"Review DLL imports: {imports}"
    dist = PROJECT / "dist"
    dist.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="package-", dir=dist) as temp:
        stage = Path(temp) / NAME
        stage.mkdir()
        for source, target in [
            (EXE, "cybermage.exe"), (PROJECT / "docs/RELEASE_README.md", "README.md"),
            (PROJECT / "docs/CONTROLS.md", "CONTROLS.md"), (PROJECT / "LICENSE", "LICENSE"),
            (PROJECT / "THIRD_PARTY_NOTICE.md", "THIRD_PARTY_NOTICE.md"), (PROJECT / "COPYING", "COPYING"),
            (PROJECT / "assets/controller-help.png", "guides/controller-help.png"),
            (PROJECT / "assets/keyboard-help.png", "guides/keyboard-help.png"),
        ]:
            out = stage / target
            out.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, out)
        licenses(stage)
        # Ship corresponding source for the GPL-linked executable, including Cargo dependencies.
        source = export(PROJECT, Path(temp) / "source", include_site=False)
        config = subprocess.check_output(["cargo", "vendor", "--locked", "vendor"],
                                         cwd=source, text=True)
        (source / ".cargo").mkdir()
        (source / ".cargo/config.toml").write_text(config, encoding="utf-8")
        # Cargo source mtimes may be Unix epoch; ZIP's earliest date is 1980.
        with zipfile.ZipFile(stage / "SOURCE.zip", "w", zipfile.ZIP_DEFLATED, strict_timestamps=False) as z:
            for p in sorted(source.rglob("*")):
                if p.is_file():
                    z.write(p, p.relative_to(source).as_posix())
        files = sorted(p for p in stage.rglob("*") if p.is_file())
        (stage / "SHA256SUMS").write_text("".join(
            f"{hashlib.sha256(p.read_bytes()).hexdigest()} *{p.relative_to(stage).as_posix()}\n" for p in files), encoding="ascii")
        archive = dist / f"{NAME}.zip"
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
            for p in sorted(stage.rglob("*")):
                if p.is_file():
                    z.write(p, f"{NAME}/{p.relative_to(stage).as_posix()}")
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        archive.with_suffix(".zip.sha256").write_text(f"{digest} *{archive.name}\n", encoding="ascii")
    print(f"Created {archive} ({archive.stat().st_size:,} bytes)")
    print("Windows GUI executable: no console/log window on launch.")
    print(f"System DLLs only: {', '.join(imports)}")
    print("No commercial game files, saves, personal configuration, screenshots, or installer data included.")
    print("The embedded Darklight logo icon retains its original commercial-artwork rights.")


if __name__ == "__main__":
    main()
