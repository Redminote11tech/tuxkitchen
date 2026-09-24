use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

// Android ramdisks are newc-format cpio archives ("070701"/"070702").
// A pure-Rust reader/writer means ramdisk editing needs no external tool
// and, more importantly, can carry untouched entries through VERBATIM:
// uid/gid/mode/timestamps live in the archive, and disk extraction as a
// normal user loses them - so a naive "repack from folder" would ship a
// ramdisk owned by uid 1000. Here only edited entries are rebuilt.

const NEWC_MAGIC: &[u8] = b"070701";
const CRC_MAGIC: &[u8] = b"070702";
const TRAILER: &str = "TRAILER!!!";

const S_IFMT: u32 = 0o170000;
const S_IFDIR: u32 = 0o040000;
const S_IFLNK: u32 = 0o120000;
const S_IFREG: u32 = 0o100000;

#[derive(Debug, Clone)]
pub struct CpioEntry {
    pub name: String,
    /// Full st_mode including the file-type bits.
    pub mode: u32,
    pub uid: u16,
    pub gid: u16,
    pub nlink: u16,
    pub mtime: u32,
    pub ino: u32,
    pub rdev: u32,
    /// File content, or the symlink target for S_IFLNK entries.
    pub data: Vec<u8>,
    pub magic_was_crc: bool,
}

fn hex_field(data: &[u8], at: usize) -> Result<u32, String> {
    let text = std::str::from_utf8(&data[at..at + 8]).map_err(|_| "cpio: header is not text".to_string())?;
    u32::from_str_radix(text, 16).map_err(|_| format!("cpio: bad header field {:?}", text))
}

pub fn read_entries(ramdisk: &Path) -> Result<Vec<CpioEntry>, String> {
    let data = fs::read(ramdisk)
        .map_err(|e| format!("cannot read {}: {}", ramdisk.display(), e))?;
    read_entries_from(&data)
}

pub fn read_entries_from(data: &[u8]) -> Result<Vec<CpioEntry>, String> {
    if data.len() < 6 || (data[0..6] != *NEWC_MAGIC && data[0..6] != *CRC_MAGIC) {
        return Err("not a newc cpio archive (ramdisk.cpio should come from unpacking a boot image)".into());
    }
    let mut entries = Vec::new();
    let mut pos = 0usize;
    loop {
        if pos + 6 > data.len() {
            break;
        }
        let magic_was_crc = data[pos..pos + 6] == CRC_MAGIC[..];
        if data[pos..pos + 6] != NEWC_MAGIC[..] && !magic_was_crc {
            break; // trailing padding or garbage: stop
        }
        if pos + 110 > data.len() {
            return Err("cpio: truncated header".into());
        }
        let ino = hex_field(data, pos + 6)?;
        let mode = hex_field(data, pos + 14)?;
        let uid = hex_field(data, pos + 22)?;
        let gid = hex_field(data, pos + 30)?;
        let nlink = hex_field(data, pos + 38)?;
        let mtime = hex_field(data, pos + 46)?;
        let filesize = hex_field(data, pos + 54)?;
        let _devmajor = hex_field(data, pos + 62)?;
        let _devminor = hex_field(data, pos + 70)?;
        let rdevmajor = hex_field(data, pos + 78)?;
        let rdevminor = hex_field(data, pos + 86)?;
        let namesize = hex_field(data, pos + 94)?;
        let _check = hex_field(data, pos + 102)?;
        pos += 110;

        let name_end = pos + namesize as usize;
        if name_end > data.len() {
            return Err("cpio: truncated name".into());
        }
        let name = String::from_utf8_lossy(&data[pos..name_end])
            .trim_end_matches('\0')
            .to_string();
        pos = align4(pos + namesize as usize);
        let data_end = pos + filesize as usize;
        if data_end > data.len() {
            return Err(format!("cpio: truncated data for {}", name));
        }
        if name == TRAILER {
            break;
        }
        entries.push(CpioEntry {
            name,
            mode,
            uid: uid as u16,
            gid: gid as u16,
            nlink: nlink as u16,
            mtime,
            ino,
            rdev: (rdevmajor << 16) | rdevminor,
            data: data[pos..data_end].to_vec(),
            magic_was_crc,
        });
        pos = align4(data_end);
    }
    if entries.is_empty() {
        return Err("cpio: no entries found".into());
    }
    Ok(entries)
}

fn align4(n: usize) -> usize {
    n.div_ceil(4) * 4
}

/// Read a directory tree back into entries, mirroring the original archive's
/// naming style ("./x" vs "x").
fn entries_from_dir(
    dir: &Path,
    prefix: &str,
    next_ino: &mut u32,
    now: u32,
) -> Result<Vec<CpioEntry>, String> {
    let mut out = Vec::new();
    fn walk(
        dir: &Path,
        rel: &str,
        prefix: &str,
        out: &mut Vec<CpioEntry>,
        next_ino: &mut u32,
        now: u32,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 32 {
            return Err("ramdisk tree too deep".into());
        }
        let rd = fs::read_dir(dir).map_err(|e| format!("cannot read {}: {}", dir.display(), e))?;
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let rel_child = if rel.is_empty() { name.clone() } else { format!("{}/{}", rel, name) };
            let full = format!("{}{}", prefix, rel_child);
            let meta = entry.metadata().map_err(|e| e.to_string())?;
            let ftype = meta.file_type();
            let path = entry.path();
            if ftype.is_dir() {
                out.push(CpioEntry {
                    name: full.clone(),
                    mode: S_IFDIR | 0o755,
                    uid: 0,
                    gid: 0,
                    nlink: 2,
                    mtime: now,
                    ino: *next_ino,
                    rdev: 0,
                    data: Vec::new(),
                    magic_was_crc: false,
                });
                *next_ino += 1;
                walk(&path, &rel_child, prefix, out, next_ino, now, depth + 1)?;
            } else if ftype.is_symlink() {
                let target = fs::read_link(&path)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .into_owned();
                out.push(CpioEntry {
                    name: full.clone(),
                    mode: S_IFLNK | 0o777,
                    uid: 0,
                    gid: 0,
                    nlink: 1,
                    mtime: now,
                    ino: *next_ino,
                    rdev: 0,
                    data: target.into_bytes(),
                    magic_was_crc: false,
                });
                *next_ino += 1;
            } else if ftype.is_file() {
                let perms = meta.permissions().mode();
                out.push(CpioEntry {
                    name: full.clone(),
                    mode: S_IFREG | (perms & 0o7777),
                    uid: 0,
                    gid: 0,
                    nlink: 1,
                    mtime: now,
                    ino: *next_ino,
                    rdev: 0,
                    data: fs::read(&path).map_err(|e| e.to_string())?,
                    magic_was_crc: false,
                });
                *next_ino += 1;
            }
            // char/block devices and fifos are skipped: a non-root
            // extraction could not have created them anyway.
        }
        Ok(())
    }
    walk(dir, "", prefix, &mut out, next_ino, now, 0)?;
    Ok(out)
}

/// Rebuild the ramdisk from an edited folder, carrying every entry that
/// matches the original archive through with its original header and data.
/// Changed and new entries get fresh data with a sane header; deleted paths
/// disappear. Returns the number of entries written.
pub fn repack(original: &Path, dir: &Path, out: &Path) -> Result<usize, String> {
    let original_entries = read_entries(original)?;
    let prefix = if original_entries.iter().any(|e| e.name.starts_with("./")) {
        "./"
    } else {
        ""
    };
    let next_ino = original_entries.iter().map(|e| e.ino).max().unwrap_or(0) + 1;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);

    let disk_entries = entries_from_dir(dir, prefix, &mut { next_ino }, now)?;

    // Index the original archive by name for metadata preservation.
    let originals: std::collections::HashMap<&str, &CpioEntry> = original_entries
        .iter()
        .map(|e| (e.name.trim_start_matches("./"), e))
        .collect();

    let mut result: Vec<CpioEntry> = Vec::new();
    let mut reused = 0usize;
    // Original order first: keep every original entry that still exists on
    // disk, carrying headers verbatim; only the data may change.
    for entry in &original_entries {
        let bare = entry.name.trim_start_matches("./");
        let disk_path = dir.join(bare);
        let meta = match fs::symlink_metadata(&disk_path) {
            Ok(m) => m,
            Err(_) => continue, // deleted by the user
        };
        let kind = entry.mode & S_IFMT;
        let still_same_kind = match kind {
            S_IFDIR => meta.is_dir(),
            S_IFLNK => meta.file_type().is_symlink(),
            S_IFREG => meta.is_file(),
            _ => false,
        };
        if !still_same_kind {
            continue; // type changed: it will be re-added from disk below
        }
        let mut updated = entry.clone();
        match kind {
            S_IFLNK => {
                let target = fs::read_link(&disk_path)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .into_owned();
                updated.data = target.into_bytes();
                updated.filesize_set();
            }
            S_IFREG => {
                let content = fs::read(&disk_path).map_err(|e| e.to_string())?;
                updated.data = content;
                updated.filesize_set();
            }
            _ => {}
        }
        reused += 1;
        result.push(updated);
    }

    // Then anything new on disk that the original archive did not have.
    for entry in disk_entries {
        let bare = entry.name.trim_start_matches("./");
        if !originals.contains_key(bare) {
            result.push(entry);
        }
    }

    write_entries(&result, out)?;
    Ok(reused + result.iter().filter(|e| !originals.contains_key(e.name.trim_start_matches("./"))).count())
}

impl CpioEntry {
    fn filesize_set(&mut self) {
        // filesize lives in the serialized header only; data.len() is the
        // source of truth when writing.
    }
}

fn write_entry(buf: &mut Vec<u8>, e: &CpioEntry) {
    let push_hex = |buf: &mut Vec<u8>, v: u32| {
        buf.extend_from_slice(format!("{:08x}", v).as_bytes());
    };
    buf.extend_from_slice(if e.magic_was_crc { CRC_MAGIC } else { NEWC_MAGIC });
    push_hex(buf, e.ino);
    push_hex(buf, e.mode);
    push_hex(buf, e.uid as u32);
    push_hex(buf, e.gid as u32);
    push_hex(buf, e.nlink as u32);
    push_hex(buf, e.mtime);
    push_hex(buf, e.data.len() as u32);
    push_hex(buf, 0); // devmajor
    push_hex(buf, 0); // devminor
    push_hex(buf, (e.rdev >> 16) & 0xffff);
    push_hex(buf, e.rdev & 0xffff);
    push_hex(buf, (e.name.len() + 1) as u32);
    push_hex(buf, 0); // check
    buf.extend_from_slice(e.name.as_bytes());
    buf.push(0);
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
    buf.extend_from_slice(&e.data);
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

pub fn write_entries(entries: &[CpioEntry], out: &Path) -> Result<(), String> {
    let mut buf: Vec<u8> = Vec::new();
    for e in entries {
        write_entry(&mut buf, e);
    }
    // trailer, padded to a 512-byte boundary like the classic tool
    let trailer = CpioEntry {
        name: TRAILER.into(),
        mode: 0,
        uid: 0,
        gid: 0,
        nlink: 1,
        mtime: 0,
        ino: 0,
        rdev: 0,
        data: Vec::new(),
        magic_was_crc: false,
    };
    write_entry(&mut buf, &trailer);
    while !buf.len().is_multiple_of(512) {
        buf.push(0);
    }
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut f = fs::File::create(out).map_err(|e| format!("cannot create {}: {}", out.display(), e))?;
    f.write_all(&buf).map_err(|e| e.to_string())?;
    Ok(())
}

/// Extract entries into a folder. Char/block devices and fifos are skipped
/// (cannot exist without root) and reported in the returned warnings.
pub fn extract(entries: &[CpioEntry], out_dir: &Path) -> Result<Vec<String>, String> {
    let mut warnings = Vec::new();
    fs::create_dir_all(out_dir).map_err(|e| format!("cannot create {}: {}", out_dir.display(), e))?;
    // Dirs and files may appear in any order in the archive; create parents
    // on demand instead of trusting order.
    for e in entries {
        let bare = e.name.trim_start_matches("./");
        // The archive root ("." or "./") is the output folder itself:
        // create_dir_all on a path ending in "." would ENOENT anyway.
        if bare.is_empty() || bare == "." {
            continue;
        }
        let dest = out_dir.join(bare);
        match e.mode & S_IFMT {
            S_IFDIR => {
                fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            }
            S_IFLNK => {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                let target = String::from_utf8_lossy(&e.data).into_owned();
                let _ = fs::remove_file(&dest);
                std::os::unix::fs::symlink(&target, &dest)
                    .map_err(|e| format!("symlink {}: {}", dest.display(), e))?;
            }
            S_IFREG => {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                fs::write(&dest, &e.data).map_err(|e| e.to_string())?;
                fs::set_permissions(&dest, fs::Permissions::from_mode(e.mode & 0o7777))
                    .map_err(|e| e.to_string())?;
            }
            _ => warnings.push(format!("{} (special file, skipped)", e.name)),
        }
    }
    Ok(warnings)
}

#[tauri::command]
pub async fn ramdisk_extract(
    app_handle: tauri::AppHandle,
    ramdisk_cpio: String,
    out_dir: String,
) -> Result<usize, String> {
    use tauri::Emitter;
    tauri::async_runtime::spawn_blocking(move || {
        let entries = read_entries(Path::new(&ramdisk_cpio))?;
        let n = entries.len();
        let warnings = extract(&entries, Path::new(&out_dir))?;
        let _ = app_handle.emit(
            "log-event",
            format!("[Ramdisk] extracted {} entries to {}", n, out_dir),
        );
        for w in &warnings {
            let _ = app_handle.emit("log-event", format!("[Ramdisk] skipped {}", w));
        }
        Ok(n)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn ramdisk_repack(
    app_handle: tauri::AppHandle,
    ramdisk_dir: String,
    original_cpio: String,
    out_cpio: String,
) -> Result<usize, String> {
    use tauri::Emitter;
    tauri::async_runtime::spawn_blocking(move || {
        let original = Path::new(&original_cpio);
        let out = Path::new(&out_cpio);
        // Writing over the original keeps repack_boot working unchanged;
        // the pristine archive is backed up once for future restores.
        if out == original {
            let backup = original.with_extension("orig.cpio");
            if !backup.exists() {
                fs::copy(original, &backup).map_err(|e| e.to_string())?;
                let _ = app_handle.emit(
                    "log-event",
                    format!("[Ramdisk] pristine copy kept at {}", backup.display()),
                );
            }
        }
        let n = repack(original, Path::new(&ramdisk_dir), out)?;
        let _ = app_handle.emit(
            "log-event",
            format!(
                "[Ramdisk] rebuilt {} ({} entries, metadata carried from the original archive)",
                out_cpio, n
            ),
        );
        Ok(n)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use crate::util;
    use std::path::PathBuf;
    use super::*;

    fn make_fixture(dir: &Path) -> PathBuf {
        let src = dir.join("src");
        fs::create_dir_all(src.join("init.d")).unwrap();
        fs::create_dir_all(src.join("system/bin")).unwrap();
        fs::write(src.join("init.rc"), "#!/system/bin/sh\n").unwrap();
        fs::set_permissions(src.join("init.rc"), fs::Permissions::from_mode(0o750)).unwrap();
        fs::write(src.join("init.test.rc"), "service t /system/bin/t\n").unwrap();
        fs::write(src.join("default.prop"), b"ro.test=1\n").unwrap();
        fs::write(src.join("system/bin/tool"), b"binary").unwrap();
        fs::set_permissions(src.join("system/bin/tool"), fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink("/system/bin/tool", src.join("system/bin/tool-link")).unwrap();
        let cpio_path = dir.join("ramdisk.cpio");
        let out = fs::File::create(&cpio_path).unwrap();
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("cd {} && find . | cpio -o -H newc --owner=0:0", src.display()))
            .stdout(std::process::Stdio::from(out))
            .status()
            .unwrap();
        assert!(status.success(), "host cpio failed");
        cpio_path
    }

    #[test]
    fn reads_what_host_cpio_wrote() {
        if util::find_tool("cpio").is_none() {
            eprintln!("skipping: cpio missing");
            return;
        }
        let dir = std::env::temp_dir().join(format!("tk-cpio-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let fixture = make_fixture(&dir);

        let entries = read_entries(&fixture).unwrap();
        let find = |n: &str| entries.iter().find(|e| e.name.trim_start_matches("./") == n).cloned();
        let init_rc = find("init.rc").unwrap();
        assert_eq!(init_rc.mode, S_IFREG | 0o750, "mode must come from the archive");
        assert_eq!(init_rc.uid, 0);
        let link = find("system/bin/tool-link").unwrap();
        assert_eq!(link.mode & S_IFMT, S_IFLNK);
        assert_eq!(String::from_utf8_lossy(&link.data), "/system/bin/tool");
        let prop = find("default.prop").unwrap();
        assert_eq!(prop.data.as_slice(), b"ro.test=1\n");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn extract_writes_modes_and_links() {
        if util::find_tool("cpio").is_none() {
            eprintln!("skipping: cpio missing");
            return;
        }
        let dir = std::env::temp_dir().join(format!("tk-cpio-x-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let fixture = make_fixture(&dir);
        let entries = read_entries(&fixture).unwrap();
        let out = dir.join("out");
        let warnings = extract(&entries, &out).unwrap();
        assert!(warnings.is_empty());
        let mode = fs::metadata(out.join("init.rc")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o750);
        let target = fs::read_link(out.join("system/bin/tool-link")).unwrap();
        assert_eq!(target.to_string_lossy(), "/system/bin/tool");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn repack_edits_and_preserves_metadata() {
        if util::find_tool("cpio").is_none() {
            eprintln!("skipping: cpio missing");
            return;
        }
        let dir = std::env::temp_dir().join(format!("tk-cpio-r-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let fixture = make_fixture(&dir);

        // Extract, edit, add, delete.
        let tree = dir.join("ramdisk");
        let entries = read_entries(&fixture).unwrap();
        extract(&entries, &tree).unwrap();
        fs::write(tree.join("default.prop"), b"ro.test=2\n").unwrap();
        fs::write(tree.join("new_file.txt"), b"added").unwrap();
        fs::remove_file(tree.join("init.test.rc")).unwrap();
        fs::set_permissions(tree.join("init.rc"), fs::Permissions::from_mode(0o750)).unwrap();

        let rebuilt = dir.join("ramdisk-new.cpio");
        let count = repack(&fixture, &tree, &rebuilt).unwrap();
        assert!(count >= 6);

        // (a) Untouched entries carry their original headers byte for byte.
        let orig = read_entries(&fixture).unwrap();
        let new = read_entries(&rebuilt).unwrap();
        let orig_rc = orig.iter().find(|e| e.name.trim_start_matches("./") == "init.rc").unwrap();
        let new_rc = new.iter().find(|e| e.name.trim_start_matches("./") == "init.rc").unwrap();
        assert_eq!(orig_rc.mode, new_rc.mode, "original metadata must be carried");
        assert_eq!(orig_rc.uid, new_rc.uid);
        assert_eq!(orig_rc.mtime, new_rc.mtime);

        // (b) The host cpio can read our archive and shows the edits.
        let listing = std::process::Command::new("cpio")
            .arg("-it")
            .stdin(fs::File::open(&rebuilt).unwrap())
            .output()
            .unwrap();
        let list = String::from_utf8_lossy(&listing.stdout);
        assert!(list.contains("new_file.txt"), "added file must appear: {}", list);
        assert!(!list.contains("init.test.rc"), "deleted file must be gone: {}", list);

        // (c) Host cpio extraction sees the edited content and kept modes.
        let out = dir.join("verify");
        fs::create_dir_all(&out).unwrap();
        let status = std::process::Command::new("cpio")
            .args(["-id", "--no-absolute-filenames"])
            .stdin(fs::File::open(&rebuilt).unwrap())
            .current_dir(&out)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(fs::read(out.join("default.prop")).unwrap().as_slice(), b"ro.test=2\n");
        assert_eq!(fs::read(out.join("new_file.txt")).unwrap().as_slice(), b"added");
        let mode = fs::metadata(out.join("init.rc")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o750, "exec bits survive the rebuild");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn repack_without_changes_is_a_noop() {
        if util::find_tool("cpio").is_none() {
            eprintln!("skipping: cpio missing");
            return;
        }
        let dir = std::env::temp_dir().join(format!("tk-cpio-n-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let fixture = make_fixture(&dir);
        let entries = read_entries(&fixture).unwrap();
        let tree = dir.join("ramdisk");
        extract(&entries, &tree).unwrap();

        let rebuilt = dir.join("ramdisk-new.cpio");
        repack(&fixture, &tree, &rebuilt).unwrap();

        let a = read_entries(&fixture).unwrap();
        let b = read_entries(&rebuilt).unwrap();
        assert_eq!(a.len(), b.len());
        for (ea, eb) in a.iter().zip(b.iter()) {
            assert_eq!(ea.name, eb.name);
            assert_eq!(ea.mode, eb.mode);
            assert_eq!(ea.data, eb.data);
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_non_cpio_input() {
        let dir = std::env::temp_dir().join(format!("tk-cpio-bad-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("not.cpio");
        fs::write(&p, b"this is not a cpio").unwrap();
        assert!(read_entries(&p).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
