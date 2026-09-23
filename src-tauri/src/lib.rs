use tauri::{AppHandle, Emitter};
use std::path::Path;

mod projects;
mod partitions;
mod boot;
mod customize;
mod build;
mod legacy_boot;
mod util;
mod fsconfig;
mod tools;
mod dtbo;
mod payload;

#[tauri::command]
async fn unpack_rom(app_handle: AppHandle, file_path: String, workspace_path: String) -> Result<(), String> {
    app_handle.emit("log-event", format!("[System] Unpacking ROM: {}", file_path)).map_err(|e| e.to_string())?;

    let path = Path::new(&file_path);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_string();

    // tar -xf autodetects compression, so .tar.gz/.tar.xz/.tgz all land here.
    let cmd = match ext.as_str() {
        "zip" | "jar" => {
            let mut c = util::cmd("unzip");
            c.arg("-o").arg(&file_path).arg("-d").arg(&workspace_path);
            c
        }
        "tar" | "md5" | "tgz" | "gz" | "xz" | "zst" | "bzip2" | "bz2" => {
            let mut c = util::cmd("tar");
            c.arg("-xf").arg(&file_path).arg("-C").arg(&workspace_path);
            c
        }
        "7z" => {
            let mut c = util::cmd("7z");
            c.arg("x").arg(&file_path).arg(format!("-o{}", workspace_path));
            c
        }
        "lz4" => {
            let mut c = util::cmd("lz4");
            c.arg("-d").arg(&file_path).arg(format!("{}/extracted.img", workspace_path));
            c
        }
        "br" => {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
            let mut c = util::cmd("brotli");
            c.arg("-d").arg(&file_path).arg("-o").arg(format!("{}/{}", workspace_path, stem));
            c
        }
        other => {
            app_handle.emit("log-event", format!("[Error] Unsupported file extension: {} - use the Partitions tab to check the file by signature", other)).map_err(|e| e.to_string())?;
            return Err("Unsupported format".into());
        }
    };

    std::fs::create_dir_all(&workspace_path).map_err(|e| e.to_string())?;
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || util::run_logged(&app, "Unpack", cmd))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            unpack_rom,
            projects::get_projects,
            projects::create_project,
            projects::delete_project,
            partitions::convert_sparse,
            partitions::unpack_super,
            partitions::decompress_brotli,
            partitions::extract_ext4,
            partitions::extract_erofs,
            partitions::extract_f2fs,
            partitions::convert_file_contexts,
            boot::unpack_boot,
            boot::repack_boot,
            boot::patch_magisk,
            boot::patch_vbmeta,
            customize::list_apps,
            customize::remove_app,
            customize::list_removed,
            customize::restore_app,
            customize::run_custom_script,
            customize::run_samsung_disarm,
            customize::run_deodex,
            build::build_image,
            build::build_super,
            build::build_tar,
            build::build_tar_md5,
            build::to_sparse,
            build::compress_lz4,
            legacy_boot::aik_unpack,
            legacy_boot::aik_repack,
            tools::check_tools,
            tools::probe_file,
            dtbo::dtbo_unpack,
            dtbo::dtbo_pack,
            payload::extract_payload
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
