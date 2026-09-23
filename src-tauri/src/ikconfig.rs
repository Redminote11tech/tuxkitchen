use serde::Serialize;
use std::path::Path;
use std::process::Command;


// Kernels built with CONFIG_IKCONFIG embed their .config as a gzip stream
// between the "IKCFG_ST" / "IKCFG_ED" markers. Reading it out of an unpacked
// kernel answers the question every filesystem conversion depends on: can
// this kernel actually mount EROFS / F2FS, and with which compression?

const IKCFG_ST: &[u8; 8] = b"IKCFG_ST";
const IKCFG_ED: &[u8; 8] = b"IKCFG_ED";

#[derive(Debug, Serialize)]
pub struct KernelFsSupport {
    /// Whether an embedded config was found and decompressed.
    pub config_found: bool,
    pub ext4: Option<bool>,
    pub erofs: Option<bool>,
    pub erofs_zip: Option<bool>,
    pub f2fs: Option<bool>,
    pub f2fs_compression: Option<bool>,
    /// Compression algorithms confirmed supported, e.g. "EROFS LZ4".
    pub algorithms: Vec<String>,
    pub total_entries: usize,
}

fn cfg(map: &[(String, String)], key: &str) -> Option<bool> {
    map.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v == "y" || v == "m")
}

fn supported(map: &[(String, String)], key: &str, label: &str, algos: &mut Vec<String>) -> Option<bool> {
    let on = cfg(map, key);
    if on == Some(true) {
        algos.push(label.to_string());
    }
    on
}

/// Decompress the embedded config with the system gzip and parse it.
fn parse_config(gzip_blob: &[u8]) -> Option<Vec<(String, String)>> {
    let tmp = std::env::temp_dir().join(format!("tk-ikcfg-{}.gz", std::process::id()));
    std::fs::write(&tmp, gzip_blob).ok()?;
    let out = Command::new("gzip")
        .arg("-dc")
        .arg(&tmp)
        .output()
        .ok()?;
    let _ = std::fs::remove_file(&tmp);
    // gzip warns (nonzero) about trailing garbage after the stream - that is
    // expected, since we cut the blob at IKCFG_ED. The stdout is the config.
    if out.stdout.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("CONFIG_") {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            entries.push((k.trim().to_string(), v.trim().to_string()));
        } else if let Some(k) = line.strip_suffix(" is not set") {
            entries.push((k.trim().to_string(), "n".to_string()));
        }
    }
    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

/// Extract and summarise the kernel's embedded configuration, if present.
pub fn read(kernel_path: &Path) -> KernelFsSupport {
    let mut none = KernelFsSupport {
        config_found: false,
        ext4: None,
        erofs: None,
        erofs_zip: None,
        f2fs: None,
        f2fs_compression: None,
        algorithms: Vec::new(),
        total_entries: 0,
    };
    let Ok(data) = std::fs::read(kernel_path) else { return none };
    let Some(st) = data.windows(8).position(|w| w == IKCFG_ST) else { return none };
    let blob_start = st + 8;
    if blob_start + 2 > data.len() || data[blob_start..blob_start + 2] != [0x1f, 0x8b] {
        return none;
    }
    let Some(ed) = data[blob_start..].windows(8).position(|w| w == IKCFG_ED) else { return none };
    let Some(map) = parse_config(&data[blob_start..blob_start + ed]) else { return none };

    none.config_found = true;
    none.total_entries = map.len();
    let mut algos = Vec::new();
    none.ext4 = supported(&map, "CONFIG_EXT4_FS", "ext4", &mut algos);
    none.erofs = supported(&map, "CONFIG_EROFS_FS", "EROFS", &mut algos);
    none.erofs_zip = supported(&map, "CONFIG_EROFS_FS_ZIP", "EROFS compression", &mut algos);
    none.f2fs = supported(&map, "CONFIG_F2FS_FS", "F2FS", &mut algos);
    none.f2fs_compression = supported(&map, "CONFIG_F2FS_FS_COMPRESSION", "F2FS compression", &mut algos);
    // Per-algorithm details, only meaningful when the filesystem (and its
    // compression layer) are actually built in.
    if none.erofs_zip == Some(true) {
        for (key, label) in [
            ("CONFIG_EROFS_FS_ZIP_LZ4", "EROFS LZ4"),
            ("CONFIG_EROFS_FS_ZIP_DEFLATE", "EROFS deflate"),
            ("CONFIG_EROFS_FS_ZIP_ZSTD", "EROFS zstd"),
        ] {
            if cfg(&map, key) == Some(true) {
                algos.push(label.to_string());
            }
        }
    }
    if none.f2fs == Some(true) && none.f2fs_compression == Some(true) {
        for (key, label) in [
            ("CONFIG_F2FS_FS_LZ4", "F2FS LZ4"),
            ("CONFIG_F2FS_FS_LZO", "F2FS LZO"),
            ("CONFIG_F2FS_FS_ZSTD", "F2FS zstd"),
        ] {
            if cfg(&map, key) == Some(true) {
                algos.push(label.to_string());
            }
        }
    }
    none.algorithms = algos;
    none
}

fn fmt_bool(v: Option<bool>) -> String {
    match v {
        Some(true) => "supported".to_string(),
        Some(false) => "NOT in kernel".to_string(),
        None => "not mentioned in config".to_string(),
    }
}

/// Build-time companion: find the workspace's unpacked kernel, read its
/// config, and log what the chosen filesystem format can expect. Warnings,
/// never blockers - the config is only present when the vendor enabled it.
pub fn log_build_warnings(app: &tauri::AppHandle, workspace: &Path, format: &str) {
    use tauri::Emitter;
    let candidates = ["kernel", "boot/kernel", "boot-unpack/kernel"];
    let Some(kernel) = candidates.iter().map(|c| workspace.join(c)).find(|p| p.is_file()) else {
        let _ = app.emit("log-event", "[Kernel] no unpacked kernel in workspace - unpack boot.img in the Magisk tab to get filesystem support checks".to_string());
        return;
    };
    let support = read(&kernel);
    if !support.config_found {
        let _ = app.emit("log-event", format!(
            "[Kernel] {} carries no embedded config (CONFIG_IKCONFIG disabled) - cannot verify {} support",
            kernel.display(), format
        ));
        return;
    }
    let _ = app.emit("log-event", format!(
        "[Kernel] embedded config read from {} ({} entries)",
        kernel.display(),
        support.total_entries
    ));
    let checks: &[(&str, Option<bool>)] = match format {
        "erofs" => &[
            ("EROFS filesystem", support.erofs),
            ("EROFS compression", support.erofs_zip),
        ],
        "f2fs" => &[
            ("F2FS filesystem", support.f2fs),
            ("F2FS compression", support.f2fs_compression),
        ],
        "ext4" => &[("ext4 filesystem", support.ext4)],
        _ => &[],
    };
    for (name, state) in checks {
        let _ = app.emit("log-event", format!("[Kernel] {}: {}", name, fmt_bool(*state)));
        if *state == Some(false) {
            let _ = app.emit("log-event", format!(
                "[Kernel] WARNING: kernel lacks {} - the built {} image may not mount on this device",
                name.to_lowercase(), format
            ));
        }
    }
    if !support.algorithms.is_empty() {
        let _ = app.emit("log-event", format!("[Kernel] compression support: {}", support.algorithms.join(", ")));
    }
}

#[tauri::command]
pub async fn read_kernel_config(kernel_path: String) -> Result<KernelFsSupport, String> {
    let path = Path::new(&kernel_path);
    if !path.is_file() {
        return Err(format!("kernel not found: {} - unpack boot.img first", kernel_path));
    }
    Ok(read(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_kernel(config_text: &str) -> Vec<u8> {
        // Compress the config with the system gzip, exactly like the runtime
        // path consumes it.
        let raw = std::env::temp_dir().join(format!("tk-ikcfg-src-{}", std::process::id()));
        std::fs::write(&raw, config_text).unwrap();
        let out = Command::new("gzip").arg("-c").arg(&raw).output().unwrap();
        std::fs::remove_file(&raw).unwrap();
        assert!(out.status.success());
        let mut kernel = vec![0u8; 1024]; // kernel header noise
        kernel.extend_from_slice(b"\x11arch stuff\x22");
        kernel.extend_from_slice(IKCFG_ST);
        kernel.extend_from_slice(&out.stdout);
        kernel.extend_from_slice(IKCFG_ED);
        kernel.extend_from_slice(&[0u8; 512]);
        kernel
    }

    #[test]
    fn extracts_and_summarises_support() {
        let config = "\
CONFIG_LINUX_VERSION=6
CONFIG_EXT4_FS=y
CONFIG_EROFS_FS=y
CONFIG_EROFS_FS_ZIP=y
CONFIG_EROFS_FS_ZIP_LZ4=y
CONFIG_EROFS_FS_ZIP_ZSTD=n
CONFIG_F2FS_FS is not set
CONFIG_F2FS_FS_COMPRESSION is not set
";
        let kernel = make_kernel(config);
        let tmp = std::env::temp_dir().join(format!("tk-ikcfg-kernel-{}", std::process::id()));
        std::fs::write(&tmp, &kernel).unwrap();
        let s = read(&tmp);
        std::fs::remove_file(&tmp).unwrap();

        assert!(s.config_found);
        assert!(s.total_entries >= 8);
        assert_eq!(s.ext4, Some(true));
        assert_eq!(s.erofs, Some(true));
        assert_eq!(s.erofs_zip, Some(true));
        assert_eq!(s.f2fs, Some(false));
        assert!(s.algorithms.contains(&"EROFS LZ4".to_string()));
        assert!(!s.algorithms.contains(&"EROFS zstd".to_string()));
        assert!(!s.algorithms.iter().any(|a| a.starts_with("F2FS ")));
    }

    #[test]
    fn rejects_kernel_without_config() {
        let tmp = std::env::temp_dir().join(format!("tk-ikcfg-none-{}", std::process::id()));
        std::fs::write(&tmp, b"bare kernel without ikconfig".repeat(10)).unwrap();
        let s = read(&tmp);
        std::fs::remove_file(&tmp).unwrap();
        assert!(!s.config_found);
        assert_eq!(s.erofs, None);
    }
}
