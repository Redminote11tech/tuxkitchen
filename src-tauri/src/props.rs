use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::Emitter;

#[derive(Debug, Serialize)]
pub struct PropFile {
    /// Path relative to the workspace root.
    pub rel: String,
    pub size: u64,
}

/// Prop files a ROM ships, in the order they usually matter.
const CANDIDATES: &[&str] = &[
    "build.prop",
    "system/build.prop",
    "vendor/build.prop",
    "product/build.prop",
    "system/system_ext/build.prop",
    "system_ext/build.prop",
    "system/product/build.prop",
    "vendor/etc/prop.default",
    "system/etc/prop.default",
    "default.prop",
];

#[tauri::command]
pub async fn list_prop_files(workspace_path: String) -> Result<Vec<PropFile>, String> {
    let ws = Path::new(&workspace_path).canonicalize().map_err(|e| e.to_string())?;
    let mut found = Vec::new();
    for cand in CANDIDATES {
        let p = ws.join(cand);
        if p.is_file() {
            let size = p.metadata().map(|m| m.len()).unwrap_or(0);
            found.push(PropFile { rel: cand.to_string(), size });
        }
    }
    Ok(found)
}

/// Refuse anything that is not a prop file inside the workspace: the editor
/// writes full file contents, so the target must be pinned down.
fn confine_prop_path(workspace_path: &str, rel: &str) -> Result<PathBuf, String> {
    let ws = Path::new(workspace_path).canonicalize().map_err(|e| e.to_string())?;
    let target = ws.join(rel.trim_start_matches('/'));
    let parent_ok = target
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.starts_with(&ws))
        .unwrap_or(false);
    if !parent_ok || !target.starts_with(&ws) {
        return Err("Refusing: prop path is outside the project workspace".into());
    }
    let name = target.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let is_prop = name == "build.prop"
        || name == "default.prop"
        || name == "prop.default"
        || name.ends_with(".prop");
    if !is_prop {
        return Err(format!("Refusing: {} is not a prop file", name));
    }
    Ok(target)
}

#[tauri::command]
pub async fn read_prop_file(workspace_path: String, rel: String) -> Result<String, String> {
    let target = confine_prop_path(&workspace_path, &rel)?;
    std::fs::read_to_string(&target).map_err(|e| format!("cannot read {}: {}", rel, e))
}

#[tauri::command]
pub async fn save_prop_file(
    app_handle: tauri::AppHandle,
    workspace_path: String,
    rel: String,
    content: String,
) -> Result<(), String> {
    let target = confine_prop_path(&workspace_path, &rel)?;
    if target.exists() {
        let backup = target.with_extension("prop.bak");
        std::fs::copy(&target, &backup).map_err(|e| e.to_string())?;
        let _ = app_handle.emit("log-event", format!("[Props] previous version saved to {}", backup.display()));
    }
    std::fs::write(&target, &content).map_err(|e| e.to_string())?;
    let _ = app_handle.emit("log-event", format!("[Props] wrote {} ({} lines)", rel, content.lines().count()));
    Ok(())
}

/// Parse "key=value" lines for the structured table; comments and blanks are
/// preserved verbatim on save, so a round trip never rewrites what it does
/// not understand. The frontend editor mirrors this exact rule; this copy
/// exists so the round-trip rule is pinned by tests.
#[allow(dead_code)]
pub fn parse_props(content: &str) -> Vec<(String, String)> {
    content
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#') && t.contains('=')
        })
        .map(|l| {
            let (k, v) = l.split_once('=').unwrap_or((l, ""));
            (k.trim().to_string(), v.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pairs_and_skips_comments() {
        let src = "# comment\nro.build.version.sdk=34\n\nro.kernel.qemu=0\nbroken line\n";
        let props = parse_props(src);
        assert_eq!(props.len(), 2);
        assert_eq!(props[0], ("ro.build.version.sdk".into(), "34".into()));
        assert_eq!(props[1], ("ro.kernel.qemu".into(), "0".into()));
    }

    #[test]
    fn values_may_contain_equals_and_spaces() {
        let props = parse_props("ro.build.fingerprint=Google/walleye/x: a=b\n");
        assert_eq!(props[0].1, "Google/walleye/x: a=b");
    }
}
