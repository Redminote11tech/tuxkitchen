use std::fs;
use std::path::{Path, PathBuf};

/// Android filesystem metadata plumbing: the fs_config binary file that
/// e2fsdroid -C / sload.f2fs -C consume (uid/gid/mode per path) and the
/// file_contexts discovery used to label images on build.
///
/// Binary fs_config format (libcutils fs_config.cpp), little-endian per record:
///   u16 length  (whole record: 16-byte header + name + trailing NUL)
///   u16 uid, u16 gid, u16 mode, u64 capabilities
///   name bytes followed by NUL
//
/// Recursively collect (full path, mode) for every directory and regular file
/// under `tree`, mapped onto `mount_point`. Ownership is forced to root:root:
/// the extracted tree is owned by the invoking user, which is never what a
/// device expects.
fn collect(dir: &Path, prefix: &str, out: &mut Vec<(String, u16)>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let full = format!("{}/{}", prefix, name);
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            out.push((full.clone(), 0o755));
            collect(&path, &full, out);
        } else if meta.is_file() {
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                if meta.permissions().mode() & 0o111 != 0 { 0o755 } else { 0o644 }
            };
            #[cfg(not(unix))]
            let mode = 0o644;
            out.push((full, mode));
        }
        // Symlinks are skipped: the filesystem driver records them from the
        // image itself and fs_config entries on symlink paths are ignored.
    }
}

/// Write the fs_config binary for `tree` mounted at `mount_point` to `out`.
/// Returns the number of entries written.
pub fn write_fs_config(tree: &Path, mount_point: &str, out: &Path) -> Result<usize, String> {
    let prefix = mount_point.trim_end_matches('/');
    let mut entries: Vec<(String, u16)> = Vec::new();
    // The image root itself (the mount point) gets a standard dir mode.
    entries.push((prefix.to_string(), 0o755));
    collect(tree, prefix, &mut entries);
    // Longest path first: correct under both first-match and longest-match
    // prefix lookup, which the different libcutils versions use.
    entries.sort_by_key(|(path, _)| (std::cmp::Reverse(path.len()), path.clone()));

    let mut buf: Vec<u8> = Vec::with_capacity(entries.len() * 48);
    for (path, mode) in &entries {
        let name = path.as_bytes();
        let len = (16 + name.len() + 1) as u16;
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes()); // uid root
        buf.extend_from_slice(&0u16.to_le_bytes()); // gid root
        buf.extend_from_slice(&mode.to_le_bytes());
        buf.extend_from_slice(&0u64.to_le_bytes()); // capabilities
        buf.extend_from_slice(name);
        buf.push(0);
    }
    out.parent()
        .map(fs::create_dir_all)
        .transpose()
        .map_err(|e| e.to_string())?;
    fs::write(out, &buf).map_err(|e| e.to_string())?;
    Ok(entries.len())
}

/// Find build-time file_contexts for a partition tree: the tree's own
/// etc/selinux copy first, then workspace-level candidates. Returns the text
/// file and, if one exists, the compiled .bin (which needs decompiling).
pub fn find_file_contexts(workspace: &Path, tree: &Path) -> (Option<PathBuf>, Option<PathBuf>) {
    let text_candidates = [
        tree.join("etc/selinux/file_contexts"),
        tree.join("etc/selinux/file_contexts.txt"),
        workspace.join("file_contexts"),
        workspace.join("file_contexts.txt"),
        workspace.join("system/etc/selinux/file_contexts"),
    ];
    let bin_candidates = [
        tree.join("etc/selinux/file_contexts.bin"),
        workspace.join("file_contexts.bin"),
        workspace.join("system/etc/selinux/file_contexts.bin"),
    ];
    let text = text_candidates.iter().find(|p| p.is_file()).cloned();
    let bin = bin_candidates.iter().find(|p| p.is_file()).cloned();
    (text, bin)
}

/// Ordered file_contexts rules for the coverage check. Lookup is first-match,
/// matching the runtime selabel_file backend over an in-order contexts file.
pub struct ContextRules {
    rules: Vec<(regex::Regex, String)>,
    skipped: usize,
}

impl ContextRules {
    pub fn load(path: &Path) -> Result<ContextRules, String> {
        let data = fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
        let mut rules = Vec::new();
        let mut skipped = 0usize;
        for line in data.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // "<regex> <context>"; the regex never contains whitespace in
            // practice, so the first whitespace run is the separator.
            let mut it = line.splitn(2, char::is_whitespace);
            let (Some(pat), Some(ctx)) = (it.next(), it.next()) else { continue };
            let ctx = ctx.trim();
            match regex::Regex::new(pat) {
                Ok(re) => rules.push((re, ctx.to_string())),
                Err(_) => skipped += 1,
            }
        }
        Ok(ContextRules { rules, skipped })
    }

    pub fn lookup(&self, path: &str) -> Option<&str> {
        self.rules
            .iter()
            .find(|(re, _)| re.is_match(path))
            .map(|(_, ctx)| ctx.as_str())
    }

    pub fn skipped(&self) -> usize {
        self.skipped
    }

    /// Paths under `tree` that no rule covers, mapped to their on-device path.
    /// Returned list is capped at `cap` entries; the second element is the
    /// total count of uncovered paths.
    pub fn uncovered(&self, tree: &Path, mount_point: &str, cap: usize) -> (Vec<String>, usize) {
        let prefix = mount_point.trim_end_matches('/');
        let mut found = Vec::new();
        let mut total = 0usize;
        walk(tree, prefix, &mut |rel_full, is_file| {
            if self.lookup(rel_full).is_none() {
                total += 1;
                if found.len() < cap {
                    let label = if is_file { "" } else { "/" };
                    found.push(format!("{}{}", rel_full, label));
                }
            }
        });
        (found, total)
    }
}

fn walk(tree: &Path, prefix: &str, f: &mut impl FnMut(&str, bool)) {
    let Ok(rd) = fs::read_dir(tree) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        let full = format!("{}/{}", prefix, entry.file_name().to_string_lossy());
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            walk(&path, &full, f);
        } else {
            f(&full, true);
        }
    }
}

/// debugfs's filespec parser does not honor quotes, so paths containing
/// whitespace or quotes cannot be addressed; they are counted and skipped
/// instead of corrupting the batch.
fn addressable(path: &str) -> bool {
    !path.chars().any(|c| c.is_whitespace() || c == '"')
}

/// Generate a debugfs command script that applies root:root ownership and
/// SELinux labels directly to a built ext4 image. This is the fallback path
/// for hosts whose e2fsdroid is unusable (notably Arch's android-tools
/// build). Context inheritance follows the nearest labelled parent, which is
/// what restorecon would give.
///
/// Context rules match device paths (`/system/...`) while debugfs commands
/// address image-relative paths (`/bin/...`), mirroring e2fsdroid's -a
/// mapping. Returns (commands written, paths skipped).
pub fn write_debugfs_script(
    tree: &Path,
    mount_point: &str,
    rules: Option<&ContextRules>,
    out: &Path,
) -> Result<(usize, usize), String> {
    let mount = mount_point.trim_end_matches('/');
    let mut cmds: Vec<String> = Vec::new();
    let mut skipped = 0usize;

    // Image root: ownership plus the label of the mount point itself.
    cmds.push("sif / uid 0".to_string());
    cmds.push("sif / gid 0".to_string());
    if let Some(rules) = rules {
        if let Some(ctx) = rules.lookup(mount) {
            cmds.push(format!("ea_set / security.selinux {}", ctx));
        }
    }

    fn emit(
        dir: &Path,
        device_prefix: &str,
        image_prefix: &str,
        inherited: Option<&str>,
        rules: Option<&ContextRules>,
        cmds: &mut Vec<String>,
        skipped: &mut usize,
    ) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let device_full = format!("{}/{}", device_prefix, name);
            let image_full = format!("{}/{}", image_prefix, name);
            let ctx = rules
                .and_then(|r| r.lookup(&device_full))
                .map(|s| s.to_string())
                .or_else(|| inherited.map(|s| s.to_string()));
            let Ok(meta) = entry.metadata() else { continue };
            if !addressable(&image_full) {
                *skipped += 1;
                if meta.is_dir() {
                    // Still descend: children below the unaddressable folder
                    // are addressable again.
                    emit(&entry.path(), &device_full, &image_full, ctx.as_deref(), rules, cmds, skipped);
                }
                continue;
            }
            if meta.is_dir() {
                cmds.push(format!("sif {} uid 0", image_full));
                cmds.push(format!("sif {} gid 0", image_full));
                if let Some(c) = &ctx {
                    cmds.push(format!("ea_set {} security.selinux {}", image_full, c));
                }
                emit(&entry.path(), &device_full, &image_full, ctx.as_deref(), rules, cmds, skipped);
            } else if meta.is_file() {
                cmds.push(format!("sif {} uid 0", image_full));
                cmds.push(format!("sif {} gid 0", image_full));
                if let Some(c) = &ctx {
                    cmds.push(format!("ea_set {} security.selinux {}", image_full, c));
                }
            } else {
                // Symlinks: label only; their ownership is irrelevant.
                if let Some(c) = &ctx {
                    cmds.push(format!("ea_set {} security.selinux {}", image_full, c));
                }
            }
        }
    }
    emit(
        tree,
        mount,
        "",
        rules.and_then(|r| r.lookup(mount)),
        rules,
        &mut cmds,
        &mut skipped,
    );

    out.parent()
        .map(fs::create_dir_all)
        .transpose()
        .map_err(|e| e.to_string())?;
    fs::write(out, cmds.join("\n") + "\n").map_err(|e| e.to_string())?;
    Ok((cmds.len(), skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fs_config_binary_layout() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = std::env::temp_dir().join(format!("tk-fsconfig-{}", std::process::id()));
        let tree = tmp.join("tree");
        fs::create_dir_all(tree.join("bin")).unwrap();
        fs::write(tree.join("bin/tool"), "x").unwrap();
        fs::set_permissions(tree.join("bin/tool"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(tree.join("readme"), "y").unwrap();
        let out = tmp.join("fs_config");
        let n = write_fs_config(&tree, "/system", &out).unwrap();
        assert_eq!(n, 4); // /system, /system/bin, /system/bin/tool, /system/readme

        let data = fs::read(&out).unwrap();
        // First record must be the deepest path (sorted longest-first): /system/bin/tool
        let len = u16::from_le_bytes([data[0], data[1]]) as usize;
        assert_eq!(len, 16 + b"/system/bin/tool".len() + 1);
        assert_eq!(u16::from_le_bytes([data[2], data[3]]), 0); // uid
        assert_eq!(u16::from_le_bytes([data[4], data[5]]), 0); // gid
        assert_eq!(u16::from_le_bytes([data[6], data[7]]), 0o755);
        let name = &data[16..len - 1];
        assert_eq!(name, b"/system/bin/tool");
        assert_eq!(data[len - 1], 0);
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn contexts_first_match_and_coverage() {
        let tmp = std::env::temp_dir().join(format!("tk-ctx-{}", std::process::id()));
        let tree = tmp.join("system");
        fs::create_dir_all(tree.join("bin")).unwrap();
        fs::write(tree.join("bin/sh"), "").unwrap();
        fs::write(tree.join("loose"), "").unwrap();
        let ctx = tmp.join("file_contexts");
        // Only /system/bin is covered: /system/loose must show up uncovered.
        fs::write(
            &ctx,
            "/system/bin/sh u:object_r:shell_exec:s0\n\
             /system/bin(/.*)? u:object_r:system_file:s0\n",
        )
        .unwrap();
        let rules = ContextRules::load(&ctx).unwrap();
        assert_eq!(rules.lookup("/system/bin/sh"), Some("u:object_r:shell_exec:s0"));
        assert_eq!(rules.lookup("/system/bin/tool"), Some("u:object_r:system_file:s0"));
        assert_eq!(rules.lookup("/system/loose"), None);
        let (uncovered, total) = rules.uncovered(&tree, "/system", 10);
        assert_eq!(total, 1);
        assert_eq!(uncovered, vec!["/system/loose"]);
        fs::remove_dir_all(&tmp).unwrap();
    }
}
