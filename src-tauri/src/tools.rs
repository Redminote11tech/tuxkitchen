use serde::Serialize;
use std::path::{Path, PathBuf};

/// Resolve a tool on PATH (no shell).
pub fn find_in_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

#[derive(Debug, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub purpose: String,
    pub installed: bool,
    pub hint: String,
}

const TOOLS: &[(&str, &str, &str)] = &[
    ("magiskboot", "boot unpack/repack, Magisk patching, Samsung disarm", "AUR: magiskboot / magiskboot-bin"),
    ("unzip", "firmware archive extraction", "pacman: unzip"),
    ("7z", "ext4 extraction, 7z archives", "pacman: 7zip"),
    ("tar", "tar/tar.md5 archives", "pacman: tar"),
    ("lz4", "Samsung .lz4 images", "pacman: lz4"),
    ("brotli", "dat.br and payload brotli blobs", "pacman: brotli"),
    ("simg2img", "sparse to raw images", "pacman: android-tools"),
    ("img2simg", "raw to sparse images", "pacman: android-tools"),
    ("lpmake", "super.img building", "pacman: android-tools"),
    ("lpdump", "super layout readout (group name, slots)", "pacman: android-tools"),
    ("e2fsdroid", "SELinux contexts + fs_config on ext4 build", "pacman: android-tools"),
    ("mke2fs", "ext4 image creation", "pacman: e2fsprogs"),
    ("e2fsck", "ext4 build verification", "pacman: e2fsprogs"),
    ("debugfs", "ext4 inspection and repair", "pacman: e2fsprogs"),
    ("mkfs.f2fs", "F2FS image creation", "pacman: f2fs-tools"),
    ("sload.f2fs", "F2FS population with contexts/fs_config", "pacman: f2fs-tools"),
    ("mkfs.erofs", "EROFS image creation", "pacman: erofs-utils"),
    ("fsck.erofs", "EROFS extraction and verification", "pacman: erofs-utils"),
    ("baksmali", "deodexing", "AUR: android-apktool or smali"),
    ("apktool", "APK decompile/recompile", "AUR: android-apktool (needs java)"),
    ("sefcontext_decompile", "file_contexts.bin decompilation", "not packaged on Arch - decompile manually or supply file_contexts.txt"),
    ("xz", "payload.bin xz blobs", "pacman: xz"),
    ("bzip2", "payload.bin bzip2 blobs", "pacman: bzip2"),
    ("zstd", "payload.bin zstd blobs", "pacman: zstd"),
    ("gzip", "kernel embedded-config (IKCONFIG) decompression", "pacman: gzip"),
];

#[tauri::command]
pub async fn check_tools() -> Vec<ToolInfo> {
    TOOLS
        .iter()
        .map(|(name, purpose, hint)| ToolInfo {
            name: name.to_string(),
            purpose: purpose.to_string(),
            installed: find_in_path(name).is_some(),
            hint: hint.to_string(),
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct ProbeResult {
    /// Machine-readable kind, used by the UI to route actions.
    pub kind: String,
    /// Human-readable description with a suggested next step.
    pub detail: String,
}

fn u32_be(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}
fn u32_le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// Identify a file by header signature, not by extension. Firmware breaks
/// extension conventions constantly; the probes below cover the formats the
/// kitchen actually acts on.
pub fn probe(path: &Path) -> Result<ProbeResult, String> {
    use std::io::Read;
    let mut head = [0u8; 4096];
    let mut f = std::fs::File::open(path)
        .map_err(|e| format!("cannot open {}: {}", path.display(), e))?;
    let n = f.read(&mut head).map_err(|e| e.to_string())?;
    let h = &head[..n];

    let kind = if n >= 8 && &h[0..8] == b"ANDROID!" {
        "boot"
    } else if n >= 8 && &h[0..8] == b"VNDRBOOT" {
        "vendor_boot"
    } else if n >= 4 && (&h[0..4] == b"AVB0" || (n >= 64 && &h[n - 64..n - 60] == b"AVBf")) {
        "vbmeta"
    } else if n >= 4 && &h[0..4] == b"CrAU" {
        "payload"
    } else if n >= 4 && u32_le(&h[0..4]) == 0xed26ff3a {
        "sparse"
    } else if n >= 1084 && u32_le(&h[1080..1084]) == 0xef53 {
        "ext"
    } else if n >= 1030 && &h[1024..1030] == b"EROFS\0" {
        "erofs"
    } else if n >= 1028 && u32_le(&h[1024..1028]) == 0xf2f52010 {
        "f2fs"
    } else if n >= 4 && u32_be(&h[0..4]) == 0xd7b7ab1e {
        "dtbo"
    } else if n >= 258 && &h[257..262] == b"ustar" {
        "tar"
    } else if n >= 4 && &h[0..4] == b"PK\x03\x04" {
        "zip"
    } else if n >= 2 && &h[0..2] == b"\x1f\x8b" {
        "gzip"
    } else if n >= 6 && &h[0..6] == b"\xfd7zXZ\x00" {
        "xz"
    } else if n >= 4 && &h[0..4] == b"\x28\xb5\x2f\xfd" {
        "zstd"
    } else if n >= 3 && &h[0..3] == b"BZh" {
        "bzip2"
    } else if n >= 4 && &h[0..4] == b"\x04\x22\x4d\x18" {
        "lz4"
    } else if n >= 4 && &h[0..4] == b"\x02\x21\x4c\x18" {
        "lz4-legacy"
    } else {
        "unknown"
    };

    let detail = match kind {
        "boot" => "Android boot image - unpack/repack or patch with Magisk in the Magisk tab",
        "vendor_boot" => "vendor_boot image - same boot tools apply",
        "vbmeta" => "vbmeta - disable verity flags in the Magisk tab",
        "payload" => "OTA payload.bin - extract partitions in the Partitions tab",
        "sparse" => "Android sparse image - convert to raw first",
        "ext" => "ext4 filesystem image - extract with 7z or build from a directory",
        "erofs" => "EROFS image - extract with fsck.erofs or build from a directory",
        "f2fs" => "F2FS image - build supported; extraction needs a mount",
        "dtbo" => "device tree overlay table - unpack/pack into individual dtb blobs",
        "tar" => "tar archive (Odin or firmware) - unpack in the Unpacker",
        "zip" => "zip archive (firmware or OTA) - unpack in the Unpacker; may contain payload.bin",
        "gzip" | "xz" | "zstd" | "bzip2" | "lz4" | "lz4-legacy" => "compressed stream - decompress before use",
        _ => "unrecognized signature",
    };
    Ok(ProbeResult { kind: kind.to_string(), detail: detail.to_string() })
}

#[tauri::command]
pub fn probe_file(path: String) -> Result<ProbeResult, String> {
    probe(Path::new(&path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_by_signature_not_extension() {
        let tmp = std::env::temp_dir().join(format!("tk-probe-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();

        let cases: Vec<(&[u8], &str)> = vec![
            (b"ANDROID!\x00\x01", "boot"),
            (b"CrAU\x00\x00\x00\x00", "payload"),
            (&[0x3a, 0xff, 0x26, 0xed, 0, 0, 0, 0], "sparse"),
        ];
        for (i, (bytes, expect)) in cases.iter().enumerate() {
            // Deliberately wrong extension: detection must not care.
            let p = tmp.join(format!("file{}.dat", i));
            std::fs::write(&p, bytes).unwrap();
            assert_eq!(probe(&p).unwrap().kind, *expect);
        }

        // ext superblock magic lives at offset 1080 regardless of extension.
        let mut ext = vec![0u8; 2048];
        ext[1080..1084].copy_from_slice(&0xef53u32.to_le_bytes());
        let p = tmp.join("vendor.img");
        std::fs::write(&p, &ext).unwrap();
        assert_eq!(probe(&p).unwrap().kind, "ext");

        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
