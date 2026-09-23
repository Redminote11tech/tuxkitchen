use serde::Serialize;
use std::path::{Path, PathBuf};
use std::fs;

use crate::util;

fn log(app: &tauri::AppHandle, line: String) {
    use tauri::Emitter;
    let _ = app.emit("log-event", line);
}

#[derive(Debug, Serialize)]
pub struct DirEntryInfo {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

fn confine(workspace_path: &str, rel: &str) -> Result<PathBuf, String> {
    let ws = Path::new(workspace_path).canonicalize().map_err(|e| e.to_string())?;
    let target = ws.join(rel.trim_start_matches('/'));
    let resolved = if target.exists() {
        target.canonicalize().map_err(|e| e.to_string())?
    } else {
        // Not-yet-existing paths (rename targets): resolve lexically.
        let parent = target
            .parent()
            .ok_or("invalid path")?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !parent.starts_with(&ws) {
            return Err("Refusing: path is outside the project workspace".into());
        }
        parent.join(target.file_name().ok_or("invalid path")?)
    };
    if !resolved.starts_with(&ws) {
        return Err("Refusing: path is outside the project workspace".into());
    }
    if resolved == ws {
        return Err("Refusing: that is the workspace itself".into());
    }
    Ok(resolved)
}

#[tauri::command]
pub async fn list_dir(workspace_path: String, rel: String) -> Result<Vec<DirEntryInfo>, String> {
    let dir = confine(&workspace_path, &rel)?;
    if !dir.is_dir() {
        return Err(format!("not a directory: {}", rel));
    }
    let mut out: Vec<DirEntryInfo> = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(meta) = entry.metadata() else { continue };
        let is_dir = meta.is_dir();
        let size = if is_dir { 0 } else { meta.len() };
        out.push(DirEntryInfo { name, is_dir, size });
    }
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

#[tauri::command]
pub async fn rename_path(workspace_path: String, rel: String, new_name: String) -> Result<(), String> {
    let target = confine(&workspace_path, &rel)?;
    if new_name.is_empty() || new_name.contains('/') || new_name.contains('\\') || new_name.starts_with('.') {
        return Err(format!("invalid name: {:?}", new_name));
    }
    let dest = target.with_file_name(&new_name);
    if dest.exists() {
        return Err(format!("{} already exists", new_name));
    }
    fs::rename(&target, &dest).map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete by moving into the project backup, exactly like app removal, so
/// everything is restorable from the Debloat screen.
#[tauri::command]
pub async fn delete_path(
    app_handle: tauri::AppHandle,
    workspace_path: String,
    rel: String,
) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let target = confine(&workspace_path, &rel)?;
        let ws = Path::new(&workspace_path).canonicalize().map_err(|e| e.to_string())?;
        let rel_path = target.strip_prefix(&ws).map_err(|e| e.to_string())?;
        let dest = ws.join(".tuxkitchen/removed").join(rel_path);
        if dest.exists() {
            return Err(format!("backup already holds {}", rel_path.display()));
        }
        fs::create_dir_all(dest.parent().unwrap_or(&dest)).map_err(|e| e.to_string())?;
        fs::rename(&target, &dest).map_err(|e| e.to_string())?;
        log(&app, format!("[Files] moved {} to project backup (restorable)", rel_path.display()));
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn apktool_decompile(
    app_handle: tauri::AppHandle,
    apk_path: String,
    out_dir: String,
) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if util::find_tool("apktool").is_none() {
            return Err("apktool not found on PATH (AUR: android-apktool; it needs java)".into());
        }
        let mut cmd = util::cmd("apktool");
        cmd.arg("d").arg("-f").arg("-o").arg(&out_dir).arg(&apk_path);
        log(&app, format!("[APK] decompiling {} with apktool", apk_path));
        util::run_logged(&app, "apktool", cmd)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn apktool_recompile(
    app_handle: tauri::AppHandle,
    apk_dir: String,
) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if util::find_tool("apktool").is_none() {
            return Err("apktool not found on PATH (AUR: android-apktool; it needs java)".into());
        }
        if !Path::new(&apk_dir).join("apktool.yml").is_file() {
            return Err(format!("no apktool.yml in {} - not an apktool project", apk_dir));
        }
        let mut cmd = util::cmd("apktool");
        cmd.arg("b").arg(&apk_dir);
        log(&app, format!("[APK] recompiling {}", apk_dir));
        util::run_logged(&app, "apktool", cmd)
    })
    .await
    .map_err(|e| e.to_string())?
}
