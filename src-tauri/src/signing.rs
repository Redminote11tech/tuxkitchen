use std::path::{Path, PathBuf};

use crate::util;

// APK signing via uber-apk-signer (patrickfav, Apache-2.0): a single jar
// that applies v1+v2+v3 schemes with an auto-generated debug keystore.
// apktool output is unsigned and will not install without this.
// Location: TUXKITCHEN_UAPKSIGNER_JAR, ~/.local/share/tuxkitchen/, or
// /usr/lib/tuxkitchen/. Needs java on PATH.

const HINT: &str = "download uber-apk-signer (https://github.com/patrickfav/uber-apk-signer/releases) \
and place the jar in ~/.local/share/tuxkitchen/ or set TUXKITCHEN_UAPKSIGNER_JAR";

pub fn find_jar() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("TUXKITCHEN_UAPKSIGNER_JAR") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let p = Path::new(&home).join(".local/share/tuxkitchen");
        if let Ok(rd) = std::fs::read_dir(&p) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with("uber-apk-signer") && name.ends_with(".jar") {
                    return Some(e.path());
                }
            }
        }
    }
    let p = Path::new("/usr/lib/tuxkitchen/uber-apk-signer.jar");
    if p.is_file() {
        return Some(p.to_path_buf());
    }
    None
}

#[tauri::command]
pub async fn sign_apk(app_handle: tauri::AppHandle, apk_path: String) -> Result<(), String> {
    use tauri::Emitter;
    tauri::async_runtime::spawn_blocking(move || {
        if !Path::new(&apk_path).is_file() {
            return Err(format!("APK not found: {}", apk_path));
        }
        if util::find_tool("java").is_none() {
            return Err("java not found on PATH (pacman: jdk-openjdk)".into());
        }
        let jar = find_jar().ok_or_else(|| HINT.to_string())?;

        let _ = app_handle.emit(
            "log-event",
            format!("[Sign] signing {} with {}", apk_path, jar.display()),
        );
        let mut cmd = util::cmd("java");
        cmd.arg("-jar")
            .arg(&jar)
            .arg("-a")
            .arg(&apk_path)
            .arg("--overwrite")
            .arg("--allowResign");
        util::run_logged(&app_handle, "apksigner", cmd)?;
        let _ = app_handle.emit(
            "log-event",
            format!("[Sign] {} is signed (v1+v2+v3, debug key)", apk_path),
        );
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
