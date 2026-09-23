use tauri::{AppHandle, Emitter};
use std::path::{Path, PathBuf};
use std::fs;
use std::os::unix::fs::PermissionsExt;

use crate::util;

fn log(app_handle: &AppHandle, line: String) {
    let _ = app_handle.emit("log-event", line);
}

#[tauri::command]
pub async fn unpack_boot(app_handle: AppHandle, input: String, output_dir: String) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
        log(&app, format!("[System] Unpacking boot image: {} to {}", input, output_dir));
        let mut cmd = util::cmd("magiskboot");
        cmd.current_dir(&output_dir).arg("unpack").arg(&input);
        util::run_logged(&app, "magiskboot", cmd)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn repack_boot(app_handle: AppHandle, input_dir: String, output: String) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let orig_img = Path::new(&input_dir).join("boot.img");
        if !orig_img.exists() {
            let msg = format!("boot.img not found in {} - magiskboot repack requires the original image next to the unpacked files", input_dir);
            log(&app, format!("[Error] {}", msg));
            return Err(msg);
        }
        log(&app, format!("[System] Repacking boot image from: {} to {}", input_dir, output));
        let mut cmd = util::cmd("magiskboot");
        cmd.current_dir(&input_dir).arg("repack").arg(&orig_img).arg(&output);
        util::run_logged(&app, "magiskboot", cmd)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Locate a host magiskboot binary: explicit override, packaged copy, then PATH.
fn resolve_magiskboot() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("TUXKITCHEN_MAGISKBOOT") {
        if Path::new(&p).exists() {
            return Some(PathBuf::from(p));
        }
    }
    let bundled = Path::new("/usr/lib/tuxkitchen/magiskboot");
    if bundled.exists() {
        return Some(bundled.to_path_buf());
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("magiskboot");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Extract the Magisk patcher payloads from the official APK into `stage`.
/// Image parsing uses the host magiskboot (arch-independent output); the
/// ramdisk payload binaries are the arm64 builds from the APK, which is
/// what phones execute after flash.
fn extract_magisk_payload(app: &AppHandle, apk: &str, stage: &Path) -> Result<(), String> {
    let raw = stage.join("raw");
    fs::create_dir_all(&raw).map_err(|e| e.to_string())?;
    log(app, "[Magisk] Extracting patcher payload from APK".into());

    let mut cmd = util::cmd("unzip");
    cmd.arg("-o").arg(apk)
        .arg("assets/boot_patch.sh")
        .arg("assets/util_functions.sh")
        .arg("assets/stub.apk")
        .arg("lib/arm64-v8a/*")
        .arg("-d").arg(&raw);
    util::run_logged(app, "unzip", cmd)?;

    let lib = raw.join("lib/arm64-v8a");
    let moves = [
        (raw.join("assets/boot_patch.sh"), stage.join("boot_patch.sh")),
        (raw.join("assets/util_functions.sh"), stage.join("util_functions.sh")),
        (raw.join("assets/stub.apk"), stage.join("stub.apk")),
        (lib.join("libmagisk.so"), stage.join("magisk")),
        (lib.join("libmagiskinit.so"), stage.join("magiskinit")),
        (lib.join("libinit-ld.so"), stage.join("init-ld")),
    ];
    for (from, to) in moves {
        if !from.exists() {
            return Err(format!("Magisk APK is missing expected payload: {}", from.display()));
        }
        fs::rename(&from, &to).map_err(|e| e.to_string())?;
    }

    let host_boot = resolve_magiskboot()
        .ok_or_else(|| "magiskboot not found. Install it (AUR: magiskboot / magiskboot-bin) or set TUXKITCHEN_MAGISKBOOT".to_string())?;
    fs::copy(&host_boot, stage.join("magiskboot")).map_err(|e| e.to_string())?;

    for name in ["magiskboot", "magisk", "magiskinit", "boot_patch.sh"] {
        let p = stage.join(name);
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }

    // Wrapper defines the Android-script shims boot_patch.sh expects (ui_print,
    // abort, grep_prop) and runs it in SOURCEDMODE so host-incompatible
    // api_level_arch_detect is skipped. The arm64 payload is data, not code.
    let wrapper = r#"#!/bin/sh
ui_print() { echo "$1"; }
abort() { echo "$1"; exit 1; }
grep_prop() { sed -n "s/^$1=//p" "$2" | head -n 1; }
SOURCEDMODE=1 KEEPVERITY=false KEEPFORCEENCRYPT=false PATCHVBMETAFLAG=false RECOVERYMODE=false LEGACYSAR=false . ./boot_patch.sh "$1"
"#;
    fs::write(stage.join("run_patch.sh"), wrapper).map_err(|e| e.to_string())?;
    fs::set_permissions(stage.join("run_patch.sh"), fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn patch_magisk(app_handle: AppHandle, boot_image: String, magisk_apk: String, output_dir: String) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if !Path::new(&boot_image).exists() {
            return Err(format!("Boot image not found: {}", boot_image));
        }
        if !Path::new(&magisk_apk).exists() {
            return Err(format!("Magisk APK not found: {}", magisk_apk));
        }
        let stage = Path::new(&output_dir).join(".tuxkitchen-magisk");
        if stage.exists() {
            fs::remove_dir_all(&stage).map_err(|e| e.to_string())?;
        }
        fs::create_dir_all(&stage).map_err(|e| e.to_string())?;

        log(&app, format!("[System] Patching boot image {} with Magisk {}", boot_image, magisk_apk));
        extract_magisk_payload(&app, &magisk_apk, &stage)?;

        // The patcher works in its own directory; give it a copy of the image.
        let img = stage.join("boot.img");
        fs::copy(&boot_image, &img).map_err(|e| e.to_string())?;

        let mut cmd = util::cmd("sh");
        cmd.current_dir(&stage).arg("./run_patch.sh").arg("./boot.img");
        util::run_logged(&app, "magisk_patch", cmd)?;

        let patched = stage.join("new-boot.img");
        if !patched.exists() {
            return Err("Magisk patcher did not produce new-boot.img".into());
        }
        let dest = Path::new(&output_dir).join("magisk_patched.img");
        fs::copy(&patched, &dest).map_err(|e| e.to_string())?;
        log(&app, format!("[System] Magisk-patched image written to {}", dest.display()));
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}


const AVB_MAGIC: &[u8; 4] = b"AVB0";
const AVB_FOOTER_MAGIC: &[u8; 4] = b"AVBf";
pub const AVB_VBMETA_IMAGE_FLAGS_HASHTREE_DISABLED: u32 = 0x0000_0001;
pub const AVB_VBMETA_IMAGE_FLAGS_VERIFICATION_DISABLED: u32 = 0x0000_0002;
/// Magisk PATCHVBMETAFLAG disables both verity enforcement bits.
pub const AVB_VBMETA_FLAGS_DISABLE_ALL: u32 =
    AVB_VBMETA_IMAGE_FLAGS_HASHTREE_DISABLED | AVB_VBMETA_IMAGE_FLAGS_VERIFICATION_DISABLED;

/// AVB 2.0 header layout (libavb/avb_vbmeta_image.h): rollback_index @112
/// (8 bytes), flags @120 (4 bytes, big endian). Edits invalidate the
/// authentication block by design; unlocked bootloaders accept invalidated
/// vbmeta, which is the point of this patch.
pub fn patch_vbmeta_flags(header: &mut [u8], flags: u32) -> Result<(), String> {
    if header.len() < 124 {
        return Err(format!("vbmeta header too small: {} bytes", header.len()));
    }
    if &header[0..4] != AVB_MAGIC {
        return Err("Not a vbmeta image (missing AVB0 magic)".into());
    }
    header[120..124].copy_from_slice(&flags.to_be_bytes());
    Ok(())
}

/// Find the vbmeta header offset: at 0, or via the AVB footer at EOF.
fn locate_vbmeta_offset(data: &[u8]) -> Result<usize, String> {
    if data.len() >= 4 && &data[0..4] == AVB_MAGIC {
        return Ok(0);
    }
    if data.len() >= 64 && &data[data.len() - 64..data.len() - 60] == AVB_FOOTER_MAGIC {
        let footer = &data[data.len() - 64..];
        let offset = u64::from_be_bytes(footer[20..28].try_into().unwrap()) as usize;
        if offset + 4 <= data.len() && &data[offset..offset + 4] == AVB_MAGIC {
            return Ok(offset);
        }
        return Err("Footer found but vbmeta offset is out of range".into());
    }
    Err("Not a vbmeta image (no AVB0 header or AVBf footer found)".into())
}
/// Patch a whole vbmeta image (header at 0 or via AVBf footer) in place.
pub fn patch_vbmeta_image(data: &mut [u8]) -> Result<(), String> {
    let offset = locate_vbmeta_offset(data)?;
    let header_end = offset + 256;
    if header_end > data.len() {
        return Err("vbmeta header extends past end of image".into());
    }
    patch_vbmeta_flags(&mut data[offset..header_end], AVB_VBMETA_FLAGS_DISABLE_ALL)
}

#[tauri::command]
pub async fn patch_vbmeta(app_handle: AppHandle, vbmeta_image: String, output_dir: String) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        log(&app, format!("[System] Patching vbmeta {} (disable verity + verification)", vbmeta_image));
        let data = fs::read(&vbmeta_image).map_err(|e| format!("Failed to read {}: {}", vbmeta_image, e))?;
        let mut data = data;
        let offset = locate_vbmeta_offset(&data)?;
        let header_end = offset + 256;
        patch_vbmeta_flags(&mut data[offset..header_end], AVB_VBMETA_FLAGS_DISABLE_ALL)?;

        fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
        let src_name = Path::new(&vbmeta_image)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("vbmeta.img");
        let dest = Path::new(&output_dir).join(format!("vbmeta_patched_{}", src_name));
        fs::write(&dest, &data).map_err(|e| e.to_string())?;
        log(&app, format!("[System] VBMeta flags set to {} (HASHTREE_DISABLED | VERIFICATION_DISABLED), written to {}", AVB_VBMETA_FLAGS_DISABLE_ALL, dest.display()));
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_header() -> Vec<u8> {
        let mut h = vec![0u8; 256];
        h[0..4].copy_from_slice(b"AVB0");
        // version 1.0
        h[4..8].copy_from_slice(&1u32.to_be_bytes());
        h[8..12].copy_from_slice(&0u32.to_be_bytes());
        h
    }

    #[test]
    fn vbmeta_flags_are_written_be() {
        let mut h = synthetic_header();
        assert!(h[120..124] == [0, 0, 0, 0]);
        patch_vbmeta_flags(&mut h, AVB_VBMETA_IMAGE_FLAGS_HASHTREE_DISABLED).unwrap();
        assert_eq!(&h[120..124], &1u32.to_be_bytes());
    }

    #[test]
    fn vbmeta_rejects_bad_magic() {
        let mut h = synthetic_header();
        h[0..4].copy_from_slice(b"XXXX");
        assert!(patch_vbmeta_flags(&mut h, 2).is_err());
        let mut short = vec![0u8; 100];
        assert!(patch_vbmeta_flags(&mut short, 2).is_err());
    }

    #[test]
    fn locate_handles_plain_and_footer_images() {
        let mut img = synthetic_header();
        assert_eq!(locate_vbmeta_offset(&img).unwrap(), 0);
        // trailing padding + footer pointing back at header
        img.extend_from_slice(&vec![0u8; 512]);
        let mut footer = vec![0u8; 64];
        footer[0..4].copy_from_slice(b"AVBf");
        footer[20..28].copy_from_slice(&0u64.to_be_bytes()); // vbmeta_offset
        footer[28..36].copy_from_slice(&256u64.to_be_bytes()); // vbmeta_size
        img.extend_from_slice(&footer);
        assert_eq!(locate_vbmeta_offset(&img).unwrap(), 0);
        assert!(locate_vbmeta_offset(&vec![0u8; 128]).is_err());
    }
}
