use tauri::{AppHandle, Emitter};
use std::path::Path;
use std::fs;

use crate::util;

/// AIK's own scripts do the unpack/repack; the shared runner drains both
/// output streams concurrently so a chatty script cannot deadlock on a full
/// stderr pipe, and surfaces every line in the console.
#[tauri::command]
pub async fn aik_unpack(app_handle: AppHandle, aik_path: String, boot_image: String) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = app.emit("log-event", format!("[Legacy] Unpacking {} using AIK at {}", boot_image, aik_path));

        let aik_dir = Path::new(&aik_path);
        let unpack_script = aik_dir.join("unpackimg.sh");

        if !unpack_script.exists() {
            return Err(format!("unpackimg.sh not found in {}", aik_path));
        }

        let mut cmd = util::cmd("bash");
        cmd.current_dir(aik_dir);
        cmd.arg("unpackimg.sh").arg(&boot_image);

        util::run_logged(&app, "AIK-Unpack", cmd)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn aik_repack(app_handle: AppHandle, aik_path: String, output_image: String) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = app.emit("log-event", format!("[Legacy] Repacking boot image using AIK at {}", aik_path));

        let aik_dir = Path::new(&aik_path);
        let repack_script = aik_dir.join("repackimg.sh");

        if !repack_script.exists() {
            return Err(format!("repackimg.sh not found in {}", aik_path));
        }

        let mut cmd = util::cmd("bash");
        cmd.current_dir(aik_dir);
        cmd.arg("repackimg.sh");

        // First run the repack script
        util::run_logged(&app, "AIK-Repack", cmd)?;

        // Then copy the generated image-new.img to the output destination
        let generated_img = aik_dir.join("image-new.img");
        if generated_img.exists() {
            fs::copy(&generated_img, &output_image).map_err(|e| format!("Failed to copy output image: {}", e))?;
            let _ = app.emit("log-event", format!("[System] Copied AIK output to {}", output_image));
            Ok(())
        } else {
            Err("AIK completed but image-new.img was not generated.".into())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
