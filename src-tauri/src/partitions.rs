use tauri::{AppHandle, Emitter};
use std::path::Path;

use crate::util;

fn log(app_handle: &AppHandle, line: String) {
    let _ = app_handle.emit("log-event", line);
}

async fn run(
    app_handle: &AppHandle,
    tag: &str,
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
) -> Result<(), String> {
    let mut cmd = util::cmd(program);
    for a in args {
        cmd.arg(a);
    }
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    tauri::async_runtime::spawn_blocking({
        let app_handle = app_handle.clone();
        let tag = tag.to_string();
        move || util::run_logged(&app_handle, &tag, cmd)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn convert_sparse(app_handle: AppHandle, input: String, output: String) -> Result<(), String> {
    log(&app_handle, format!("[System] Converting sparse image: {} to {}", input, output));
    run(&app_handle, "simg2img", "simg2img", &[input, output], None).await
}

#[tauri::command]
pub async fn unpack_super(app_handle: AppHandle, input: String, output_dir: String) -> Result<(), String> {
    log(&app_handle, format!("[System] Unpacking super image: {} to {}", input, output_dir));
    std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
    run(&app_handle, "lpunpack", "lpunpack", &[input, output_dir], None).await
}

#[tauri::command]
pub async fn decompress_brotli(app_handle: AppHandle, input: String, output: String) -> Result<(), String> {
    log(&app_handle, format!("[System] Decompressing brotli: {} to {}", input, output));
    run(&app_handle, "brotli", "brotli", &["-d".into(), input, "-o".into(), output], None).await
}

#[tauri::command]
pub async fn extract_ext4(app_handle: AppHandle, input: String, output_dir: String) -> Result<(), String> {
    log(&app_handle, format!("[System] Extracting ext4 image: {} to {}", input, output_dir));
    std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
    run(&app_handle, "7z", "7z", &["x".into(), input, format!("-o{}", output_dir)], None).await
}

#[tauri::command]
pub async fn extract_erofs(app_handle: AppHandle, input: String, output_dir: String) -> Result<(), String> {
    log(&app_handle, format!("[System] Extracting erofs image: {} to {}", input, output_dir));
    std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
    run(&app_handle, "fsck.erofs", "fsck.erofs", &[format!("--extract={}", output_dir), input], None).await
}

#[tauri::command]
pub async fn extract_f2fs(_app_handle: AppHandle, _input: String, _output_dir: String) -> Result<(), String> {
    // No root-free F2FS reader exists in the toolchain: 7z has no F2FS
    // support and dump.f2fs cannot walk the whole tree. Building F2FS works
    // (mkfs.f2fs + sload.f2fs); extraction would need a mount with root.
    Err("F2FS extraction is not supported without root (the toolchain has no F2FS reader). Building F2FS images works - see the Build tab.".into())
}

#[tauri::command]
pub async fn convert_file_contexts(app_handle: AppHandle, input: String, output: String) -> Result<(), String> {
    log(&app_handle, format!("[System] Converting file_contexts.bin: {} to {}", input, output));
    run(&app_handle, "sefcontext_decompile", "sefcontext_decompile", &["-o".into(), output, input], None).await
}
