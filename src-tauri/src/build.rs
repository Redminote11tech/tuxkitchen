use tauri::{AppHandle, Emitter};
use std::path::Path;

use crate::fsconfig;
use crate::util;

fn log(app_handle: &AppHandle, line: String) {
    let _ = app_handle.emit("log-event", line);
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Count every directory entry (files, dirs, symlinks) for inode sizing.
fn count_entries(dir: &str) -> u64 {
    fn walk(p: &Path, total: &mut u64) {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                *total += 1;
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    walk(&e.path(), total);
                }
            }
        }
    }
    let mut total = 0;
    walk(Path::new(dir), &mut total);
    total
}

fn dir_size_mb(dir: &str) -> u64 {
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
    walk(Path::new(dir), &mut total);
    total.div_ceil(1_048_576)
}

/// Decompile a compiled file_contexts.bin when sefcontext_decompile exists.
fn decompile_contexts(app: &AppHandle, bin: &Path, out: &Path) -> Option<std::path::PathBuf> {
    let status = util::capture("sefcontext_decompile", &["-o", &out.to_string_lossy(), &bin.to_string_lossy()], None)
        .ok()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if status && out.exists() {
        Some(out.to_path_buf())
    } else {
        log(app, "[Meta] file_contexts.bin found but sefcontext_decompile is unavailable - image will not be SELinux-labelled".into());
        None
    }
}

/// Report paths that no file_contexts rule covers (the metadata coverage
/// check): an unlabelled path ships with the "unlabeled" context and is the
/// classic silent bootloop.
fn report_coverage(app: &AppHandle, contexts: &Path, tree: &Path, mount_point: &str) {
    match fsconfig::ContextRules::load(contexts) {
        Ok(rules) => {
            let (list, total) = rules.uncovered(tree, mount_point, 10);
            if total > 0 {
                log(app, format!("[Meta] WARNING: {} path(s) match no file_contexts rule and would ship unlabelled:", total));
                for p in &list {
                    log(app, format!("[Meta]   {}", p));
                }
                if total > list.len() {
                    log(app, format!("[Meta]   ... and {} more", total - list.len()));
                }
            } else {
                log(app, "[Meta] file_contexts coverage: all paths labelled".into());
            }
            let skipped = rules.skipped();
            if skipped > 0 {
                log(app, format!("[Meta] {} file_contexts rule(s) could not be parsed", skipped));
            }
        }
        Err(e) => log(app, format!("[Meta] coverage check skipped: {}", e)),
    }
}

/// Apply Android ownership/SELinux metadata to a built ext4 image with
/// e2fsdroid. Returns whether the tool ran successfully; on hosts where it
/// is broken (Arch's android-tools currently fails on any image) the caller
/// falls back to the debugfs batch.
fn e2fsdroid(app: &AppHandle, image: &Path, fs_config: &Path, contexts: Option<&Path>, mount_point: &str, sparse: bool) -> Result<bool, String> {
    if util::find_tool("e2fsdroid").is_none() {
        log(app, "[Meta] e2fsdroid not found - falling back to debugfs for ownership and labels".into());
        return Ok(false);
    }
    let mut cmd = util::cmd("e2fsdroid");
    cmd.arg("-T").arg("0").arg("-C").arg(fs_config).arg("-a").arg(mount_point);
    if sparse {
        cmd.arg("-s");
    }
    if let Some(ctx) = contexts {
        cmd.arg("-S").arg(ctx);
    }
    cmd.arg(image);
    match util::run_logged(app, "e2fsdroid", cmd) {
        Ok(()) => {
            log(app, format!(
                "[Meta] e2fsdroid applied: root:root ownership from fs_config{}{}",
                if contexts.is_some() { ", SELinux contexts" } else { "" },
                if sparse { ", sparse output" } else { "" }
            ));
            Ok(true)
        }
        Err(_) => {
            log(app, "[Meta] e2fsdroid failed - falling back to debugfs for ownership and labels".into());
            Ok(false)
        }
    }
}

/// Direct metadata application through debugfs: root:root ownership and
/// SELinux labels written into the image without any android-tools.
fn apply_metadata_debugfs(app: &AppHandle, image: &Path, tree: &Path, mount_point: &str, contexts: Option<&Path>, stage: &Path, part: &str) -> Result<(), String> {
    let rules = match contexts {
        Some(p) => Some(fsconfig::ContextRules::load(p)?),
        None => None,
    };
    let script = stage.join(format!("debugfs_meta_{}.txt", part));
    let (n, skipped) = fsconfig::write_debugfs_script(tree, mount_point, rules.as_ref(), &script)?;
    let mut cmd = util::cmd("debugfs");
    cmd.arg("-w").arg("-f").arg(&script).arg(image);
    util::run_logged(app, "debugfs", cmd)?;
    log(app, format!(
        "[Meta] debugfs applied {} metadata command(s): root:root ownership{}",
        n,
        if rules.is_some() { ", SELinux contexts (unlabelled paths inherit nearest parent)" } else { ", no file_contexts available" }
    ));
    if skipped > 0 {
        log(app, format!("[Meta] {} path(s) with spaces or quotes could not be addressed by debugfs - relabel those manually", skipped));
    }
    Ok(())
}

#[tauri::command]
pub async fn build_image(
    app_handle: AppHandle,
    input_dir: String,
    output_img: String,
    format: String,
    sparse: Option<bool>,
    verify: Option<bool>,
    erofs_algo: Option<String>,
) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let tree = Path::new(&input_dir);
        if !tree.is_dir() {
            return Err(format!("Input directory not found: {}", input_dir));
        }
        let part = tree
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("partition")
            .to_string();
        let mount_point = format!("/{}", part.trim_end_matches("_new"));
        log(&app, format!("[System] Building {} image from {} to {}", format, input_dir, output_img));

        // Metadata inputs shared by ext4 and f2fs.
        let workspace = tree.parent().unwrap_or(tree).to_path_buf();
        let stage = workspace.join(".tuxkitchen");
        std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
        let fs_config = stage.join(format!("fs_config_{}", part));
        let entries = fsconfig::write_fs_config(tree, &mount_point, &fs_config)?;
        log(&app, format!("[Meta] fs_config written: {} entries, ownership root:root", entries));

        let (ctx_text, ctx_bin) = fsconfig::find_file_contexts(&workspace, tree);
        let contexts = match (ctx_text, ctx_bin) {
            (Some(t), _) => {
                log(&app, format!("[Meta] using file_contexts: {}", t.display()));
                Some(t)
            }
            (None, Some(b)) => decompile_contexts(&app, &b, &stage.join(format!("file_contexts_{}.txt", part))),
            (None, None) => {
                log(&app, "[Meta] no file_contexts found - image will be built without SELinux labels".into());
                None
            }
        };
        if let Some(ctx) = &contexts {
            report_coverage(&app, ctx, tree, &mount_point);
        }

        let want_sparse = sparse.unwrap_or(false);
        let want_verify = verify.unwrap_or(true);
        let out = Path::new(&output_img);
        let out_mb = (dir_size_mb(input_dir.trim()) * 125 / 100).max(64);
        let inodes = (count_entries(input_dir.trim()) * 115 / 100).max(1024);

        let status = match format.as_str() {
            "ext4" => {
                // mke2fs populates from the directory, e2fsdroid adds the
                // Android metadata layer (ownership, modes, SELinux, sparse).
                let mut cmd = util::cmd("sh");
                cmd.arg("-c").arg(format!(
                    "truncate -s {size}M {out} && mkfs.ext4 -F -b 4096 -I 256 -N {inodes} \
                     -O ^has_journal,^resize_inode,^metadata_csum -d {dir} {out}",
                    size = out_mb, out = shell_quote(&output_img), dir = shell_quote(input_dir.trim()),
                    inodes = inodes
                ));
                util::run_logged(&app, "mke2fs", cmd)?;
                let applied = e2fsdroid(&app, out, &fs_config, contexts.as_deref(), &mount_point, want_sparse)?;
                if !applied {
                    apply_metadata_debugfs(&app, out, tree, &mount_point, contexts.as_deref(), &stage, &part)?;
                    if want_sparse {
                        log(&app, "[Meta] sparse output needs e2fsdroid; producing a raw image instead".into());
                    }
                }
                if want_verify && util::find_tool("e2fsck").is_some() {
                    let mut cmd = util::cmd("e2fsck");
                    cmd.arg("-fn").arg(out);
                    util::run_logged(&app, "e2fsck", cmd)?;
                }
                Ok(())
            }
            "f2fs" => {
                // sload.f2fs supports the same fs_config/file_contexts inputs.
                let out_mb = out_mb.max(16);
                let mut cmd = util::cmd("sh");
                cmd.arg("-c").arg(format!(
                    "truncate -s {size}M {out} && mkfs.f2fs -f {out}",
                    size = out_mb, out = shell_quote(&output_img)
                ));
                util::run_logged(&app, "mkfs.f2fs", cmd)?;

                if util::find_tool("sload.f2fs").is_none() {
                    return Err("sload.f2fs not found (pacman: f2fs-tools) - cannot populate F2FS image".into());
                }
                let mut cmd = util::cmd("sload.f2fs");
                cmd.arg("-f").arg(input_dir.trim()).arg("-C").arg(&fs_config).arg("-t").arg(&mount_point);
                if let Some(ctx) = &contexts {
                    cmd.arg("-s").arg(ctx);
                }
                cmd.arg(out);
                match util::run_logged(&app, "sload.f2fs", cmd) {
                    Ok(()) => log(&app, "[Meta] sload.f2fs applied fs_config ownership and SELinux contexts".into()),
                    Err(_) => {
                        // Retry without the metadata layer rather than leaving
                        // a half-populated image behind.
                        log(&app, "[Meta] sload.f2fs rejected fs_config/contexts - rebuilding F2FS without metadata".into());
                        let mut cmd = util::cmd("sh");
                        cmd.arg("-c").arg(format!(
                            "truncate -s {size}M {out} && mkfs.f2fs -f {out}",
                            size = out_mb, out = shell_quote(&output_img)
                        ));
                        util::run_logged(&app, "mkfs.f2fs", cmd)?;
                        let mut cmd = util::cmd("sload.f2fs");
                        cmd.arg("-f").arg(input_dir.trim()).arg(out);
                        util::run_logged(&app, "sload.f2fs", cmd)?;
                        log(&app, "[Meta] F2FS built WITHOUT ownership/SELinux metadata".into());
                    }
                }
                Ok(())
            }
            "erofs" => {
                let mut cmd = util::cmd("mkfs.erofs");
                match erofs_algo.as_deref() {
                    Some(a) if a == "lz4" || a == "lz4hc" => {
                        cmd.arg("-z").arg(a);
                        log(&app, format!("[System] EROFS compression: {}", a));
                    }
                    _ => log(&app, "[System] EROFS compression: none".into()),
                }
                cmd.arg(out).arg(input_dir.trim());
                util::run_logged(&app, "mkfs.erofs", cmd)?;
                if want_verify && util::find_tool("fsck.erofs").is_some() {
                    let mut cmd = util::cmd("fsck.erofs");
                    cmd.arg(out);
                    util::run_logged(&app, "fsck.erofs", cmd)?;
                }
                Ok(())
            }
            other => Err(format!("Unsupported format: {}", other)),
        };
        status
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---- super.img ----

/// Read group name and metadata slot count from `lpdump` output of the stock
/// super image, so the rebuilt super matches what the firmware declared.
fn parse_lpdump(output: &str) -> (Option<String>, Option<u32>) {
    let mut group = None;
    let mut slots = None;
    for line in output.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Group ") {
            let name = rest.split([':', ' ']).next().unwrap_or("");
            if !name.is_empty() && group.is_none() {
                group = Some(name.to_string());
            }
        }
        if let Some(rest) = line.strip_prefix("Metadata slot count:") {
            if let Ok(n) = rest.trim().parse::<u32>() {
                slots = Some(n);
            }
        }
    }
    (group, slots)
}

#[tauri::command]
pub async fn build_super(app_handle: AppHandle, workspace_path: String, output_img: String) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let ws = Path::new(&workspace_path);
        if !ws.is_dir() {
            return Err(format!("Workspace not found: {}", workspace_path));
        }
        // Partitions: look for *_new.img files produced by Build Partition Image,
        // falling back to stock extracted images (system.img etc).
        let mut groups: Vec<(String, std::path::PathBuf)> = Vec::new();
        for e in std::fs::read_dir(ws).map_err(|e| e.to_string())? {
            let e = e.map_err(|e| e.to_string())?;
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(base) = name.strip_suffix("_new.img") {
                if base != "super" {
                    groups.push((base.to_string(), e.path()));
                }
            }
        }
        if groups.is_empty() {
            for part in ["system", "vendor", "product", "odm"] {
                let p = ws.join(format!("{}.img", part));
                if p.exists() {
                    groups.push((part.to_string(), p));
                }
            }
        }
        if groups.is_empty() {
            return Err("No partition images found in workspace (need *_new.img or system/vendor/product/odm .img)".into());
        }
        groups.sort();

        let out = ws.join("super_new.img");
        if out.exists() {
            std::fs::remove_file(&out).map_err(|e| e.to_string())?;
        }

        let part_size = |p: &std::path::PathBuf| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        let total: u64 = groups.iter().map(|(_, p)| part_size(p)).sum();
        if total == 0 {
            return Err("Partition images exist but are empty".into());
        }

        // Prefer the layout the stock super declares; qti is the common default.
        let (mut group_name, mut slots) = (None, None);
        let stock_super = ws.join("super.img");
        if stock_super.exists() {
            if let Some(lpdump) = util::find_tool("lpdump") {
                let _ = lpdump; // used only for the existence check above
                match util::capture("lpdump", &[&stock_super.to_string_lossy()], None) {
                    Ok(o) if o.status.success() => {
                        let text = String::from_utf8_lossy(&o.stdout).into_owned();
                        let (g, s) = parse_lpdump(&text);
                        group_name = g;
                        slots = s;
                    }
                    _ => log(&app, "[Super] lpdump could not read super.img - using defaults".into()),
                }
            } else {
                log(&app, "[Super] lpdump not found - group name defaults to qti_dynamic_partitions".into());
            }
        }
        let group_name = group_name.unwrap_or_else(|| "qti_dynamic_partitions".to_string());
        let slots = slots.unwrap_or(2);

        log(&app, format!("[Super] group: {}, metadata slots: {}", group_name, slots));
        for (name, path) in &groups {
            log(&app, format!("[Super]   {}: {} bytes", name, part_size(path)));
        }
        log(&app, format!("[Super] partition total: {} bytes", total));

        let mut args: Vec<String> = vec![
            "--metadata-size".into(), "65536".into(),
            "--metadata-slots".into(), slots.to_string(),
            "--device-size".into(), "auto".into(),
            "--group".into(), format!("{}:{}", group_name, total),
        ];
        for (name, path) in &groups {
            args.push("--partition".into());
            args.push(format!("{}:readonly:{}:{}", name, part_size(path), group_name));
            args.push("--image".into());
            args.push(format!("{}={}", name, path.display()));
        }
        args.push("--output".into());
        args.push(output_img.clone());

        log(&app, format!("[System] Building super.img with lpmake from {} partition(s)", groups.len()));
        let mut cmd = util::cmd("lpmake");
        for a in &args {
            cmd.arg(a);
        }
        util::run_logged(&app, "lpmake", cmd)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---- packaging ----

#[tauri::command]
pub async fn build_tar(app_handle: AppHandle, input_dir: String, output_tar: String) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        log(&app, format!("[System] Creating Odin Flashable TAR: {}", output_tar));
        let mut cmd = util::cmd("tar");
        cmd.arg("-cvf").arg(&output_tar).arg("-C").arg(&input_dir).arg(".");
        util::run_logged(&app, "tar", cmd)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Odin .tar.md5: plain tar with the 32-hex-char MD5 of the archive appended
/// verbatim (no newline) - that is what Odin verifies against.
fn append_md5(path: &Path) -> Result<String, String> {
    let out = util::capture("md5sum", &[&path.to_string_lossy()], None)
        .map_err(|e| format!("md5sum failed: {}", e))?;
    if !out.status.success() {
        return Err(format!("md5sum failed with status {}", out.status));
    }
    let line = String::from_utf8_lossy(&out.stdout);
    let hex = line.split_whitespace().next().unwrap_or("").to_string();
    if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("unexpected md5sum output: {:?}", line));
    }
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(path).map_err(|e| e.to_string())?;
    f.write_all(hex.as_bytes()).map_err(|e| e.to_string())?;
    Ok(hex)
}

#[tauri::command]
pub async fn build_tar_md5(app_handle: AppHandle, input_dir: String, output_tar: String) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        log(&app, format!("[System] Creating Odin .tar.md5: {}", output_tar));
        let mut cmd = util::cmd("tar");
        cmd.arg("-cvf").arg(&output_tar).arg("-C").arg(&input_dir).arg(".");
        util::run_logged(&app, "tar", cmd)?;
        let hex = append_md5(Path::new(&output_tar))?;
        log(&app, format!("[System] MD5 appended: {}", hex));
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Raw to Android sparse, the direction Odin and some fastboot paths want.
#[tauri::command]
pub async fn to_sparse(app_handle: AppHandle, input: String, output: String) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        log(&app, format!("[System] Converting raw to sparse: {} to {}", input, output));
        let mut cmd = util::cmd("img2simg");
        cmd.arg(&input).arg(&output).arg("4096");
        util::run_logged(&app, "img2simg", cmd)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn compress_lz4(app_handle: AppHandle, input: String, output: String) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        log(&app, format!("[System] Compressing LZ4: {} to {}", input, output));
        let mut cmd = util::cmd("lz4");
        cmd.arg("-B6").arg("--content-size").arg(&input).arg(&output);
        util::run_logged(&app, "lz4", cmd)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lpdump_group_and_slots() {
        let sample = "\
Metadata version: 10.0\n\
Metadata size: 12288 bytes\n\
Metadata max size: 65536 bytes\n\
Metadata slot count: 3\n\
Header flags: none\n\
Partition table:\n\
------------------------\n\
  0: system_a 0 123456\n\
Groups:\n\
------------------------\n\
Group qti_dynamic_partitions_a: Maximum size 8589934592 bytes\n\
Group qti_dynamic_partitions_b: Maximum size 8589934592 bytes\n";
        let (group, slots) = parse_lpdump(sample);
        assert_eq!(group.as_deref(), Some("qti_dynamic_partitions_a"));
        assert_eq!(slots, Some(3));
        assert_eq!(parse_lpdump("nothing here"), (None, None));
    }

    /// The load-bearing pipeline test: our fs_config binary, file_contexts
    /// discovery and debugfs metadata batch produce an image whose files are
    /// root:root and SELinux-labelled. Skips silently when the host toolchain
    /// is unavailable.
    #[test]
    fn ext4_metadata_end_to_end() {
        let need = ["mke2fs", "debugfs"];
        if need.iter().any(|t| util::find_tool(t).is_none()) {
            eprintln!("skipping: host toolchain missing {:?}", need);
            return;
        }
        use crate::fsconfig::{write_fs_config, ContextRules};

        let tmp = std::env::temp_dir().join(format!("tk-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let tree = tmp.join("system");
        std::fs::create_dir_all(tree.join("bin")).unwrap();
        std::fs::create_dir_all(tree.join("etc/selinux")).unwrap();
        std::fs::write(tree.join("bin/tool"), "#!/system/bin/sh\n").unwrap();
        std::fs::write(tree.join("readme"), "hello").unwrap();
        std::os::unix::fs::symlink("bin/tool", tree.join("tool-link")).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(tree.join("bin/tool"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // The ROM's own contexts file, as it would be found on extraction.
        std::fs::write(
            tree.join("etc/selinux/file_contexts"),
            "/system/bin(/.*)? u:object_r:system_file:s0\n\
             /system/(/.*)? u:object_r:system_file:s0\n\
             /system u:object_r:system_file:s0\n",
        )
        .unwrap();

        let img = tmp.join("system_new.img");
        let inodes = (count_entries(tree.to_str().unwrap()) * 115 / 100).max(1024);
        let mut cmd = util::cmd("sh");
        cmd.arg("-c").arg(format!(
            "truncate -s 8M {out} && mkfs.ext4 -F -b 4096 -I 256 -N {inodes} \
             -O ^has_journal,^resize_inode,^metadata_csum -d {dir} {out}",
            out = img.display(), dir = tree.display(), inodes = inodes
        ));
        assert!(cmd.status().unwrap().success(), "mke2fs failed");

        // The debugfs fallback path (what runs wherever e2fsdroid is broken).
        let contexts = tree.join("etc/selinux/file_contexts");
        let rules = Some(ContextRules::load(&contexts).unwrap());
        let script = tmp.join("meta.txt");
        let (n, skipped) = crate::fsconfig::write_debugfs_script(&tree, "/system", rules.as_ref(), &script).unwrap();
        assert!(n > 10, "expected a real batch, got {} commands", n);
        assert_eq!(skipped, 0, "no path in this fixture should be unaddressable");
        let mut dbg = util::cmd("debugfs");
        dbg.arg("-w").arg("-f").arg(&script).arg(&img);
        let out = dbg.output().unwrap();
        assert!(
            out.status.success(),
            "debugfs batch failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );

        // Ground truth from debugfs: ownership, mode, SELinux label.
        let stat = |path: &str| {
            let o = util::capture("debugfs", &["-R", &format!("stat {}", path), &img.to_string_lossy()], None).unwrap();
            assert!(o.status.success(), "debugfs stat {} failed", path);
            String::from_utf8_lossy(&o.stdout).into_owned()
        };
        let owner = |text: &str| -> (u32, u32) {
            let line = text.lines().find(|l| l.contains("User:")).unwrap_or("");
            let nums: Vec<u32> = line
                .split_whitespace()
                .filter_map(|t| t.parse::<u32>().ok())
                .collect();
            assert!(nums.len() >= 2, "cannot parse owner line: {:?}", line);
            (nums[0], nums[1])
        };

        let tool = stat("/bin/tool");
        assert_eq!(owner(&tool), (0, 0), "bin/tool not root:root:\n{}", tool);
        assert!(tool.contains("0755"), "bin/tool mode wrong:\n{}", tool);

        let readme = stat("/readme");
        assert_eq!(owner(&readme), (0, 0), "readme not root:root:\n{}", readme);
        assert!(readme.contains("0644"), "readme mode wrong:\n{}", readme);

        // SELinux labels live in security.selinux xattrs.
        let label = |path: &str| {
            let o = util::capture(
                "debugfs",
                &["-R", &format!("ea_get {} security.selinux", path), &img.to_string_lossy()],
                None,
            ).unwrap();
            String::from_utf8_lossy(&o.stdout).into_owned()
        };
        assert!(label("/bin/tool").contains("system_file"), "bin/tool label missing");
        assert!(label("/readme").contains("system_file"), "readme label missing");
        // The symlink got labelled too.
        assert!(label("/tool-link").contains("system_file"), "symlink label missing");

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    /// Does the host sload.f2fs accept our fs_config binary and contexts?
    /// Label verification would need a mount; acceptance is the assumption
    /// worth pinning here.
    #[test]
    fn f2fs_metadata_end_to_end() {
        if ["mkfs.f2fs", "sload.f2fs"].iter().any(|t| util::find_tool(t).is_none()) {
            eprintln!("skipping: f2fs-tools missing");
            return;
        }
        use crate::fsconfig::write_fs_config;

        let tmp = std::env::temp_dir().join(format!("tk-e2e-f2fs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let tree = tmp.join("system");
        std::fs::create_dir_all(tree.join("bin")).unwrap();
        std::fs::write(tree.join("bin/tool"), "x").unwrap();
        std::fs::write(tree.join("readme"), "y").unwrap();
        std::fs::write(
            tmp.join("file_contexts"),
            "/system(/.*)? u:object_r:system_file:s0\n",
        )
        .unwrap();

        let fs_config = tmp.join("fs_config");
        write_fs_config(&tree, "/system", &fs_config).unwrap();

        let img = tmp.join("system_new.img");
        let mut cmd = util::cmd("sh");
        cmd.arg("-c").arg(format!(
            "truncate -s 64M {out} && mkfs.f2fs -f {out}", out = img.display()
        ));
        assert!(cmd.status().unwrap().success(), "mkfs.f2fs failed");

        let mut sload = util::cmd("sload.f2fs");
        sload.arg("-f").arg(&tree)
            .arg("-C").arg(&fs_config)
            .arg("-s").arg(tmp.join("file_contexts"))
            .arg("-t").arg("/system")
            .arg(&img);
        let out = sload.output().unwrap();
        assert!(
            out.status.success(),
            "sload.f2fs rejected the metadata inputs: rc={}\nstdout: {}\nstderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );

        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
