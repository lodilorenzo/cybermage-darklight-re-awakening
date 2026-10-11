//! First-run import of user-owned CyberMage v1.01F data. No commercial bytes are embedded.
use std::{fs::{self, File}, io::{self, Read, Seek, SeekFrom}, path::{Path, PathBuf}};

const REQUIRED: &[&str] = &[
    "CM_HI.EXE", "ANMH.RSC", "FXO.RSC", "HIPAK.RSC", "PRP.RSC", "RAW.RSC", "SCENE.RSC",
    "CMNAME.MSG", "ITNAME.MSG", "STRINGS.MSG", "TEXT.MSG", "SPEECH.ENG", "CMLANG.CNF",
    "SOUNDSYS.CNF", "ECM.TGV", "CMENDE.TGV", "CREDENG.TGV",
    "PANEL00.DAT", "PANEL01.DAT", "PANEL02.DAT", "PANEL03.DAT", "PANEL04.DAT", "PANEL05.DAT",
    "PANEL06.DAT", "PANEL07.DAT", "PANEL08.DAT", "PANEL09.DAT", "PANEL10.DAT", "PANEL11.DAT",
];
const OPTIONAL: &[&str] = &[
    "CMNAME.FRN", "CMNAME.GER", "ITNAME.FRN", "ITNAME.GER", "STRINGS.FRN",
    "STRINGS.GER", "TEXT.FRN", "TEXT.GER", "SPEECH.FRN", "SPEECH.GER", "FXO.FSC", "FXO.GSC",
    "HIPAK.FSC", "HIPAK.GSC", "PANEL02.FDT", "PANEL02.GDT", "PANEL05.FDT", "PANEL05.GDT",
    "PANEL09.FDT", "PANEL09.GDT", "FCM.TGV", "GCM.TGV", "CMENDF.TGV", "CMENDG.TGV",
    "CREDFRE.TGV", "CREDGER.TGV",
];
fn allowed(name: &str) -> bool { REQUIRED.contains(&name) || OPTIONAL.contains(&name) }
fn invalid(message: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }
fn locate(dir: &Path, name: &str) -> io::Result<PathBuf> {
    fs::read_dir(dir)?.filter_map(Result::ok).find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|e| e.path()).ok_or_else(|| invalid(&format!("Missing required game file: {name}")))
}

pub fn validate(dir: &Path) -> io::Result<()> {
    for name in REQUIRED {
        let path = locate(dir, name)?;
        let md = fs::metadata(&path)?;
        if !md.is_file() || md.len() == 0 { return Err(invalid(&format!("Empty or invalid game file: {name}"))); }
    }
    let path = locate(dir,"CM_HI.EXE")?;
    if fs::metadata(&path)?.len() != 726898 { return Err(invalid("Unsupported executable size; CyberMage v1.01F is required.")); }
    let exe = fs::read(path)?;
    // Version fingerprint, not DRM or proof of ownership. All hooks target this release.
    let hash = exe.iter().fold(0xcbf29ce484222325u64, |h,b| (h ^ *b as u64).wrapping_mul(0x100000001b3));
    if exe.len() != 726898 || hash != 0x0A39EBE9A94993AC {
        return Err(invalid("Unsupported CM_HI.EXE. Supply the commercial CyberMage v1.01F English CD release."));
    }
    Ok(())
}

#[derive(Debug)]
struct Entry { name: String, offset: u64, len: u64, directory: bool }
struct Iso { file: File, size: u64 }
impl Iso {
    fn extent(&mut self, offset: u64, len: u64) -> io::Result<Vec<u8>> {
        if len > 8*1024*1024 || offset.checked_add(len).is_none_or(|end| end > self.size) {
            return Err(invalid("Invalid ISO directory extent"));
        }
        let mut data = vec![0; len as usize];
        self.file.seek(SeekFrom::Start(offset))?; self.file.read_exact(&mut data)?;
        Ok(data)
    }
    fn entry(&self, record: &[u8]) -> io::Result<Entry> {
        if record.len() < 34 || record[0] as usize != record.len() || record[1] != 0 || record[25] & 0x80 != 0 {
            return Err(invalid("Unsupported or malformed ISO directory record"));
        }
        let sector = u32::from_le_bytes(record[2..6].try_into().unwrap()) as u64;
        let len = u32::from_le_bytes(record[10..14].try_into().unwrap()) as u64;
        let count = record[32] as usize;
        if 33+count > record.len() || sector*2048+len > self.size { return Err(invalid("ISO extent outside image")); }
        let name = std::str::from_utf8(&record[33..33+count]).map_err(|_| invalid("Non-ASCII ISO name"))?
            .split(';').next().unwrap().to_ascii_uppercase();
        Ok(Entry { name, offset: sector*2048, len, directory: record[25] & 2 != 0 })
    }
    fn directory(&mut self, entry: &Entry) -> io::Result<Vec<Entry>> {
        let data = self.extent(entry.offset, entry.len)?;
        let mut entries = Vec::new(); let mut pos = 0;
        while pos < data.len() {
            let len = data[pos] as usize;
            if len == 0 { pos = (pos/2048+1)*2048; continue; }
            if pos+len > data.len() || pos%2048+len > 2048 { return Err(invalid("Truncated ISO directory")); }
            let record = &data[pos..pos+len];
            let entry = self.entry(record)?;
            if !(record[32] == 1 && record[33] <= 1) { entries.push(entry); }
            pos += len;
        }
        Ok(entries)
    }
    fn game_files(&mut self) -> io::Result<Vec<Entry>> {
        for sector in 16..80 {
            let pvd = self.extent(sector*2048, 2048)?;
            if &pvd[1..6] != b"CD001" || pvd[6] != 1 { return Err(invalid("Not an ISO-9660 image; select an extracted CYBERMAG folder instead")); }
            if pvd[0] == 255 { break; }
            if pvd[0] != 1 { continue; }
            if u16::from_le_bytes(pvd[128..130].try_into().unwrap()) != 2048 { return Err(invalid("Unsupported ISO sector size")); }
            let root = self.entry(&pvd[156..156+pvd[156] as usize])?;
            let entries = self.directory(&root)?;
            let game = entries.into_iter().find(|e| e.directory && e.name == "CYBERMAG")
                .ok_or_else(|| invalid("ISO has no CYBERMAG directory"))?;
            return self.directory(&game);
        }
        Err(invalid("No ISO-9660 primary volume descriptor"))
    }
}

pub fn import(source: &Path, root: &Path) -> io::Result<()> {
    let target = root.join("iso/CYBERMAG");
    if target.exists() { return Err(invalid("Asset directory already exists. Import into a fresh package; existing data will not be overwritten.")); }
    fs::create_dir_all(root)?;
    let stage = root.join(format!(".asset-import-{}", std::process::id()));
    fs::create_dir(&stage)?;
    let result = (|| {
        if source.extension().is_some_and(|e| e.eq_ignore_ascii_case("iso")) {
            let file = File::open(source)?; let size = file.metadata()?.len();
            let mut iso = Iso { file, size };
            for entry in iso.game_files()?.into_iter().filter(|e| !e.directory && allowed(&e.name)) {
                if entry.len > 512*1024*1024 { return Err(invalid("Game asset exceeds import size limit")); }
                let mut out = File::create(stage.join(&entry.name))?;
                iso.file.seek(SeekFrom::Start(entry.offset))?;
                if io::copy(&mut (&mut iso.file).take(entry.len), &mut out)? != entry.len { return Err(invalid("Truncated ISO file")); }
            }
        } else {
            let mut dir = if source.is_file() { source.parent().unwrap_or(source) } else { source }.to_path_buf();
            if locate(&dir,"CM_HI.EXE").is_err() { dir = locate(&dir,"CYBERMAG")?; }
            validate(&dir)?;
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().to_ascii_uppercase();
                if entry.file_type()?.is_file() && allowed(&name) { fs::copy(entry.path(),stage.join(name))?; }
            }
        }
        validate(&stage)?;
        fs::create_dir_all(target.parent().unwrap())?;
        fs::rename(&stage,&target)?; // publish only a complete, validated import
        Ok(())
    })();
    if stage.exists() { fs::remove_dir_all(&stage).ok(); }
    result
}

/// Setup is deliberately local/offline: no game downloads, CD keys, or ownership claims.
pub fn ensure(root: &Path, interactive: bool) -> io::Result<bool> {
    if validate(&root.join("iso/CYBERMAG")).is_ok() { return Ok(true); }
    if !interactive { return Err(invalid("Game assets unavailable. Use --import-assets PATH --root DIRECTORY with your CyberMage v1.01F ISO or game folder.")); }
    #[cfg(windows)] loop {
        message("Commercial CyberMage v1.01F assets are required and are not included. Select your own ISO, or CM_HI.EXE inside an installed/extracted CYBERMAG folder. Setup copies only game assets, not saves. Cancel exits without starting the game.");
        let Some(source) = pick_source()? else { return Ok(false); };
        match import(&source,root) { Ok(()) => return Ok(true), Err(e) => message(&format!("Import failed: {e}")) }
    }
    #[cfg(not(windows))]
    Err(invalid("Use --import-assets PATH --root DIRECTORY with your own ISO or CYBERMAG folder."))
}

#[cfg(windows)]
fn message(text: &str) {
    #[link(name="user32")]
    extern "system" { fn MessageBoxW(hwnd: isize, text: *const u16, title: *const u16, flags: u32) -> i32; }
    let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = format!("{} - Game Asset Setup", crate::rebrand::NAME).encode_utf16().chain(Some(0)).collect();
    unsafe { MessageBoxW(0,text.as_ptr(),title.as_ptr(),0x40); }
}

#[cfg(windows)]
fn pick_source() -> io::Result<Option<PathBuf>> {
    use std::os::windows::ffi::OsStringExt;
    #[repr(C)]
    struct OpenFileName {
        size: u32, owner: isize, instance: isize, filter: *const u16, custom_filter: *mut u16,
        max_custom: u32, index: u32, file: *mut u16, max_file: u32, file_title: *mut u16,
        max_title: u32, initial_dir: *const u16, title: *const u16, flags: u32,
        file_offset: u16, extension_offset: u16, default_extension: *const u16,
        custom_data: isize, hook: usize, template: *const u16, reserved: *mut std::ffi::c_void,
        reserved2: u32, flags_ex: u32,
    }
    #[link(name="comdlg32")]
    extern "system" { fn GetOpenFileNameW(info: *mut OpenFileName) -> i32; fn CommDlgExtendedError() -> u32; }
    let filter: Vec<u16> = "CyberMage ISO or executable\0*.iso;CM_HI.EXE\0ISO images\0*.iso\0Game executable\0CM_HI.EXE\0\0".encode_utf16().collect();
    let title: Vec<u16> = "Select your CyberMage v1.01F ISO or CYBERMAG\\CM_HI.EXE".encode_utf16().chain(Some(0)).collect();
    let mut file = vec![0u16;32768];
    let mut info: OpenFileName = unsafe { std::mem::zeroed() };
    info.size = std::mem::size_of::<OpenFileName>() as u32;
    info.filter = filter.as_ptr(); info.index = 1; info.file = file.as_mut_ptr(); info.max_file = file.len() as u32;
    info.title = title.as_ptr(); info.flags = 0x00080000 | 0x00001000 | 0x00000800 | 0x00000008;
    if unsafe { GetOpenFileNameW(&mut info) } == 0 {
        let error = unsafe { CommDlgExtendedError() };
        return if error == 0 { Ok(None) } else { Err(invalid(&format!("File picker error {error:#x}"))) };
    }
    let len = file.iter().position(|&c| c == 0).unwrap_or(file.len());
    Ok(Some(PathBuf::from(std::ffi::OsString::from_wide(&file[..len]))))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn iso_records_reject_truncation_traversal_and_bad_extents() {
        let dir = std::env::temp_dir().join(format!("cybermage-iso-{}",std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut iso = Iso { file: File::create(dir.join("test.iso")).unwrap(), size: 65536 };
        let mut record = vec![0;48]; record[0]=48; record[32]=9;
        record[2..6].copy_from_slice(&1u32.to_le_bytes()); record[10..14].copy_from_slice(&8u32.to_le_bytes());
        record[33..42].copy_from_slice(b"RAW.RSC;1");
        let entry = iso.entry(&record).unwrap();
        assert_eq!((entry.name.as_str(),entry.offset,entry.len),("RAW.RSC",2048,8));
        for len in 0..34 { assert!(iso.entry(&record[..len]).is_err()); }
        record[32]=255; assert!(iso.entry(&record).is_err()); record[32]=9;
        record[2..6].copy_from_slice(&u32::MAX.to_le_bytes()); assert!(iso.entry(&record).is_err());
        assert!(iso.extent(65535,2).is_err());
        for name in ["../SAVE000.DAT","SAVE000.DAT","CMDATA.CNF","NATIVE.CNF","C:/evil.exe","INSTALL.EXE"] { assert!(!allowed(name)); }
        assert!(import(&dir.join("missing.iso"),&dir.join("dest")).is_err());
        assert!(!dir.join("dest/iso/CYBERMAG").exists());
        assert_eq!(fs::read_dir(dir.join("dest")).unwrap().count(),0);
        fs::remove_dir_all(dir).unwrap();
    }
}
