use tauri::{AppHandle, Emitter};
use std::path::{Path, PathBuf};
use std::fs;
use serde::{Serialize, Deserialize};

use crate::util;

fn log(app_handle: &AppHandle, line: String) {
    let _ = app_handle.emit("log-event", line);
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AppInfo {
    pub name: String,
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RemovedApp {
    /// Path inside the project backup.
    pub backup_path: String,
    /// Where the app will be restored to.
    pub original_path: String,
    pub name: String,
}

/// Directory size on disk (files only), for the app list.
fn tree_size(path: &Path) -> u64 {
    if path.is_file() {
        return path.metadata().map(|m| m.len()).unwrap_or(0);
    }
    fn walk(p: &Path, total: &mut u64) {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                let meta = match e.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if meta.is_dir() {
                    walk(&e.path(), total);
                } else {
                    *total += meta.len();
                }
            }
        }
    }
    let mut total = 0;
    walk(path, &mut total);
    total
}

#[tauri::command]
pub async fn list_apps(workspace_path: String) -> Result<Vec<AppInfo>, String> {
    let mut apps = Vec::new();
    let base_path = Path::new(&workspace_path);

    // Common directories where apps are stored in an extracted ROM
    let app_dirs = [
        "system/app",
        "system/priv-app",
        "system/product/app",
        "system/product/priv-app",
        "system/system_ext/app",
        "system/system_ext/priv-app",
        "vendor/app",
        "product/app",
        "product/priv-app",
    ];

    for dir in &app_dirs {
        let full_dir = base_path.join(dir);
        if full_dir.exists() && full_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&full_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = entry.file_name().to_string_lossy().to_string();
                    apps.push(AppInfo {
                        name,
                        path: path.to_string_lossy().to_string(),
                        size: tree_size(&path),
                    });
                }
            }
        }
    }

    apps.sort_by_key(|a| a.name.to_lowercase());
    Ok(apps)
}

fn backup_root(workspace_path: &str) -> PathBuf {
    Path::new(workspace_path).join(".tuxkitchen/removed")
}

/// Remove an app by moving it into the project backup, so removal stays
/// reversible. `app_path` must live inside the workspace.
#[tauri::command]
pub async fn remove_app(app_handle: AppHandle, app_path: String, workspace_path: String) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let target = Path::new(&app_path);
        let ws = Path::new(&workspace_path).canonicalize().map_err(|e| e.to_string())?;
        let target_abs = target.canonicalize().map_err(|e| format!("App path does not exist: {}", e))?;
        if !target_abs.starts_with(&ws) {
            return Err("Refusing to remove: app path is outside the project workspace".into());
        }
        if target_abs == ws {
            return Err("Refusing to remove: path is the workspace itself".into());
        }
        let rel = target_abs
            .strip_prefix(&ws)
            .map_err(|e| e.to_string())?
            .to_path_buf();
        let dest = backup_root(&workspace_path).join(&rel);
        if dest.exists() {
            return Err(format!("Backup already holds a copy of this path: {}", dest.display()));
        }
        fs::create_dir_all(dest.parent().unwrap_or(&dest)).map_err(|e| e.to_string())?;
        fs::rename(&target_abs, &dest).map_err(|e| e.to_string())?;
        log(&app, format!("[System] Moved {} to project backup (restorable)", app_path));
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_removed(workspace_path: String) -> Result<Vec<RemovedApp>, String> {
    let root = backup_root(&workspace_path);
    let mut out = Vec::new();
    if !root.is_dir() {
        return Ok(out);
    }
    fn walk(root: &Path, dir: &Path, ws: &str, out: &mut Vec<RemovedApp>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && !is_app_leaf(&p) {
                walk(root, &p, ws, out);
            } else {
                let rel = p.strip_prefix(root).unwrap_or(&p).to_path_buf();
                out.push(RemovedApp {
                    backup_path: p.to_string_lossy().to_string(),
                    original_path: Path::new(ws).join(&rel).to_string_lossy().to_string(),
                    name: rel
                        .iter()
                        .map(|s| s.to_string_lossy().to_string())
                        .collect::<Vec<_>>()
                        .join("/"),
                });
            }
        }
    }
    // A "removed app" is a directory directly under .../removed/<partition>/app-ish dir.
    // Simplest faithful model: walk until a directory that is not a plain
    // container (contains apk/odex/artifacts) and treat it as one entry.
    fn is_app_leaf(p: &Path) -> bool {
        let Ok(rd) = fs::read_dir(p) else { return false };
        rd.flatten().any(|e| {
            let n = e.file_name().to_string_lossy().to_lowercase();
            n.ends_with(".apk") || n.ends_with(".odex") || n.ends_with(".vdex")
        })
    }
    let ws = workspace_path.clone();
    walk(&root, &root, &ws, &mut out);
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

#[tauri::command]
pub async fn restore_app(app_handle: AppHandle, backup_path: String) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let src = Path::new(&backup_path);
        if !src.exists() {
            return Err(format!("Backup path does not exist: {}", backup_path));
        }
        // original path = backup root stripped: <ws>/.tuxkitchen/removed/<rel>
        let rel = src
            .components()
            .collect::<Vec<_>>();
        // find the "removed" component and rejoin everything after it
        let idx = rel.iter().position(|c| c.as_os_str() == "removed")
            .ok_or("backup path is not inside the removed-apps folder")?;
        let after: PathBuf = rel[idx + 1..].iter().collect();
        let ws_root: PathBuf = rel[..idx].iter().collect();
        let dest = ws_root.join(after);
        if dest.exists() {
            return Err(format!("Restore target already exists: {}", dest.display()));
        }
        fs::create_dir_all(dest.parent().unwrap_or(&dest)).map_err(|e| e.to_string())?;
        fs::rename(src, &dest).map_err(|e| e.to_string())?;
        log(&app, format!("[System] Restored {}", dest.display()));
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn run_custom_script(app_handle: AppHandle, script_path: String, workspace_path: String) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if !Path::new(&script_path).exists() {
            return Err(format!("Script not found: {}", script_path));
        }
        log(&app, format!("[System] Running custom script: {}", script_path));
        let mut cmd = util::cmd("bash");
        cmd.arg(&script_path).current_dir(&workspace_path);
        util::run_logged(&app, "custom_script", cmd)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn run_samsung_disarm(app_handle: AppHandle, workspace_path: String) -> Result<(), String> {
    // Real disarm, ported from Magisk boot_patch.sh: hexpatch the unpacked
    // kernel to drop RKP/defex/PROCA enforcement and clear vbmeta verity flags.
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let ws = Path::new(&workspace_path);
        if !ws.is_dir() {
            return Err(format!("Workspace not found: {}", workspace_path));
        }
        log(&app, "[Disarm] Scanning workspace for boot/vbmeta images".into());
        let images: Vec<std::path::PathBuf> = std::fs::read_dir(ws)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().and_then(|e| e.to_str()) == Some("img")
                    && p.file_name().is_some_and(|n| {
                        let n = n.to_string_lossy();
                        // vendor_boot holds the ramdisk on modern Samsungs;
                        // vbmeta_system/vbmeta_vendor carry verity flags too.
                        n.starts_with("boot") || n.starts_with("vbmeta") || n.starts_with("vendor_boot")
                    })
            })
            .collect();
        if images.is_empty() {
            return Err("No boot*/vendor_boot*/vbmeta* images found in workspace - unpack a ROM first".into());
        }

        // Kernel enforcement patches (label, hex find, hex replace) from
        // Magisk boot_patch.sh v30.7 - applied only where patterns exist.
        const KERNEL_PATCHES: [(&str, &str, &str); 3] = [
            ("RKP", "49010054011440B93FA00F71E9000054010840B93FA00F7189000054001840B91FA00F7188010054", "A1020054011440B93FA00F7140020054010840B93FA00F71E0010054001840B91FA00F7181010054"),
            ("defex", "821B8012", "E2FF8F12"),
            ("PROCA", "70726F63615F636F6E66696700", "70726F63615F6D616769736B00"),
        ];

        let mut patched_any = false;
        for img in images {
            let name = img.file_name().unwrap_or_default().to_string_lossy().to_string();
            if name.starts_with("vbmeta") {
                let data = fs::read(&img).map_err(|e| e.to_string())?;
                let mut data = data;
                match crate::boot::patch_vbmeta_image(&mut data) {
                    Ok(()) => {
                        fs::write(&img, &data).map_err(|e| e.to_string())?;
                        log(&app, format!("[Disarm] Cleared verity flags in {}", name));
                        patched_any = true;
                    }
                    Err(e) => log(&app, format!("[Disarm] Skipping {}: {}", name, e)),
                }
                continue;
            }

            let work = ws.join(format!(".disarm-{}", name));
            if work.exists() {
                fs::remove_dir_all(&work).map_err(|e| e.to_string())?;
            }
            fs::create_dir_all(&work).map_err(|e| e.to_string())?;
            let img_copy = work.join("boot.img");
            fs::copy(&img, &img_copy).map_err(|e| e.to_string())?;

            let mut cmd = util::cmd("magiskboot");
            cmd.current_dir(&work).arg("unpack").arg("boot.img");
            util::run_logged(&app, "magiskboot", cmd)?;

            let kernel = work.join("kernel");
            if kernel.exists() {
                for (label, find, repl) in KERNEL_PATCHES {
                    let mut hp = util::cmd("magiskboot");
                    hp.current_dir(&work).arg("hexpatch").arg("kernel").arg(find).arg(repl);
                    match util::run_logged(&app, "magiskboot", hp) {
                        Ok(()) => log(&app, format!("[Disarm] {} kernel patch applied to {}", label, name)),
                        Err(_) => log(&app, format!("[Disarm] {} pattern not present in {} kernel", label, name)),
                    }
                }
            }

            let mut repack = util::cmd("magiskboot");
            repack.current_dir(&work).arg("repack").arg("boot.img");
            util::run_logged(&app, "magiskboot", repack)?;

            let new_img = work.join("new-boot.img");
            if new_img.exists() {
                let dest = ws.join(format!("{}_disarmed.img", name.trim_end_matches(".img")));
                fs::copy(&new_img, &dest).map_err(|e| e.to_string())?;
                log(&app, format!("[Disarm] Wrote {}", dest.display()));
                patched_any = true;
            }
            fs::remove_dir_all(&work).map_err(|e| e.to_string())?;
        }

        if patched_any {
            Ok(())
        } else {
            Err("No images could be patched (see logs)".into())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Recursively collect .odex files (modern ROMs nest them in AppName/oat/<isa>/).
fn collect_odexes(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 6 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_odexes(&p, depth + 1, out);
        } else if p.extension().and_then(|x| x.to_str()) == Some("odex") {
            out.push(p);
        }
    }
}

#[tauri::command]
pub async fn run_deodex(app_handle: AppHandle, workspace_path: String) -> Result<(), String> {
    // Best-effort deodex: baksmali each .odex (searched recursively, covering
    // the oat/<isa> layout), reinsert the resulting classes.dex into the APK,
    // and remove the odex plus the stale precompiled artefacts that would
    // otherwise keep the deodexed code from loading.
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let ws = Path::new(&workspace_path);
        if !ws.is_dir() {
            return Err(format!("Workspace not found: {}", workspace_path));
        }
        let have_baksmali = util::capture("which", &["baksmali"], None)
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !have_baksmali {
            return Err("baksmali not found on PATH. Install smali/baksmali (AUR: android-apktool) to enable deodexing".into());
        }

        let mut odexes: Vec<std::path::PathBuf> = Vec::new();
        for sub in [
            "system/app",
            "system/priv-app",
            "system/product/app",
            "system/product/priv-app",
            "system/system_ext/app",
            "system/system_ext/priv-app",
            "product/app",
            "product/priv-app",
            "vendor/app",
        ] {
            let dir = ws.join(sub);
            if dir.is_dir() {
                collect_odexes(&dir, 0, &mut odexes);
            }
        }
        if odexes.is_empty() {
            log(&app, "[Deodex] No .odex files found - nothing to do".into());
            return Ok(());
        }
        log(&app, format!("[Deodex] Found {} odex file(s)", odexes.len()));

        let mut converted = 0usize;
        for odex in odexes {
            let dir = odex.parent().unwrap_or(Path::new(".")).to_path_buf();
            let stem = odex.file_stem().unwrap_or_default().to_string_lossy().to_string();
            // The APK usually sits in the app folder above oat/<isa>/.
            let apk = if dir.file_name().is_some_and(|n| n == "oat") {
                dir.parent().unwrap_or(Path::new(".")).join(format!("{}.apk", stem))
            } else {
                dir.join(format!("{}.apk", stem))
            };
            if !apk.exists() {
                log(&app, format!("[Deodex] Skipping {} (no matching APK)", stem));
                continue;
            }

            let out_dex_dir = dir.join(format!(".deodex-{}", stem));
            let _ = fs::remove_dir_all(&out_dex_dir);

            let mut b = util::cmd("baksmali");
            b.arg("d").arg(&odex).arg("-o").arg(&out_dex_dir);
            if util::run_logged(&app, "baksmali", b).is_err() {
                let _ = fs::remove_dir_all(&out_dex_dir);
                log(&app, format!("[Deodex] Failed to disassemble {}", stem));
                continue;
            }
            let dex_file = out_dex_dir.join("classes.dex");
            if !dex_file.exists() {
                let _ = fs::remove_dir_all(&out_dex_dir);
                log(&app, format!("[Deodex] No classes.dex produced for {}", stem));
                continue;
            }

            let mut zip = util::cmd("zip");
            zip.current_dir(&out_dex_dir).arg("-j").arg(&apk).arg("classes.dex");
            if util::run_logged(&app, "zip", zip).is_err() {
                let _ = fs::remove_dir_all(&out_dex_dir);
                log(&app, format!("[Deodex] Failed to repackage {}", stem));
                continue;
            }

            let _ = fs::remove_file(&odex);
            // Stale precompiled artefacts stop deodexed code from loading.
            for artifact in [
                dir.join(format!("{}.vdex", stem)),
                dir.join(format!("{}.art", stem)),
                apk.with_extension("vdex"),
                apk.with_extension("art"),
            ] {
                if artifact.exists() {
                    let _ = fs::remove_file(&artifact);
                    log(&app, format!("[Deodex] Removed stale artifact {}", artifact.display()));
                }
            }
            // Drop the now-empty oat container if we were inside one.
            if dir.file_name().is_some_and(|n| n == "oat") {
                let _ = fs::remove_dir(&dir);
                let _ = fs::remove_dir(dir.parent().unwrap_or(Path::new(".")));
            }
            let _ = fs::remove_dir_all(&out_dex_dir);
            converted += 1;
            log(&app, format!("[Deodex] Converted {}", stem));
        }

        if converted == 0 {
            Err("No apps could be deodexed (see logs)".into())
        } else {
            log(&app, format!("[Deodex] Deodexed {} app(s)", converted));
            Ok(())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
