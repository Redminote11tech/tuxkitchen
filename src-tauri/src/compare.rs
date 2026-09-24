use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

// Tree comparison, the honest way to check your own work: walk two
// directory trees (stock vs edited, or two firmware releases), mark every
// path, and compare same-size files byte for byte. Read-only by design.
//
// What it deliberately does not do: jar/dex method-level diffs, image
// superblock diffs, or SELinux metadata diffs. Extraction loses uid/gid and
// contexts to the host filesystem, so metadata comparison here would lie.

const CHUNK: usize = 1024 * 1024;
const DEFAULT_CAP: usize = 50_000;

#[derive(Debug, Serialize, Clone)]
pub struct DiffEntry {
    pub path: String,
    /// "identical" | "added" | "removed" | "changed" | "type-changed"
    pub status: String,
    pub left_size: Option<u64>,
    pub right_size: Option<u64>,
    pub is_dir: bool,
}

#[derive(Debug, Serialize, Default)]
pub struct CompareSummary {
    pub identical: usize,
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    pub type_changed: usize,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct CompareResult {
    pub summary: CompareSummary,
    pub entries: Vec<DiffEntry>,
}

type Tree = BTreeMap<String, (PathBuf, bool)>;

fn walk(root: &Path) -> Result<Tree, String> {
    let mut map = BTreeMap::new();
    fn walk_inner(dir: &Path, rel: &str, map: &mut Tree, depth: usize) -> Result<(), String> {
        if depth > 32 {
            return Err(format!("tree too deep: {}", dir.display()));
        }
        let rd = std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {}", dir.display(), e))?;
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let child_rel = if rel.is_empty() { name.clone() } else { format!("{}/{}", rel, name) };
            let Ok(meta) = entry.metadata() else { continue };
            let is_dir = meta.is_dir();
            map.insert(child_rel.clone(), (entry.path(), is_dir));
            if is_dir {
                walk_inner(&entry.path(), &child_rel, map, depth + 1)?;
            }
        }
        Ok(())
    }
    walk_inner(root, "", &mut map, 0)?;
    Ok(map)
}

/// Exact content comparison: sizes first, then chunked byte equality.
fn files_differ(a: &Path, b: &Path) -> Result<bool, String> {
    let mut fa = File::open(a).map_err(|e| format!("cannot open {}: {}", a.display(), e))?;
    let mut fb = File::open(b).map_err(|e| format!("cannot open {}: {}", b.display(), e))?;
    let sa = fa.metadata().map_err(|e| e.to_string())?.len();
    let sb = fb.metadata().map_err(|e| e.to_string())?.len();
    if sa != sb {
        return Ok(true);
    }
    let mut ba = vec![0u8; CHUNK];
    let mut bb = vec![0u8; CHUNK];
    let mut pos = 0u64;
    loop {
        fa.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
        fb.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
        let na = read_chunk(&mut fa, &mut ba)?;
        let nb = read_chunk(&mut fb, &mut bb)?;
        if na != nb {
            return Ok(true);
        }
        if na == 0 {
            return Ok(false);
        }
        if ba[..na] != bb[..nb] {
            return Ok(true);
        }
        pos += na as u64;
    }
}

fn read_chunk(f: &mut File, buf: &mut [u8]) -> Result<usize, String> {
    let mut filled = 0;
    while filled < buf.len() {
        match f.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(filled)
}

pub fn compare(left: &Path, right: &Path, cap: usize) -> Result<CompareResult, String> {
    if !left.is_dir() {
        return Err(format!("left side is not a directory: {}", left.display()));
    }
    if !right.is_dir() {
        return Err(format!("right side is not a directory: {}", right.display()));
    }
    let lt = walk(left)?;
    let rt = walk(right)?;

    let mut entries: Vec<DiffEntry> = Vec::new();
    let mut summary = CompareSummary::default();
    let mut truncated = false;

    let push = |entry: DiffEntry, summary: &mut CompareSummary, entries: &mut Vec<DiffEntry>, truncated: &mut bool| {
        match entry.status.as_str() {
            "identical" => summary.identical += 1,
            "added" => summary.added += 1,
            "removed" => summary.removed += 1,
            "changed" => summary.changed += 1,
            "type-changed" => summary.type_changed += 1,
            _ => {}
        }
        if entries.len() < cap {
            entries.push(entry);
        } else {
            *truncated = true;
        }
    };

    let mut keys = lt.keys().chain(rt.keys()).cloned().collect::<Vec<_>>();
    keys.sort_unstable();
    keys.dedup();

    for key in keys {
        let l = lt.get(&key);
        let r = rt.get(&key);
        match (l, r) {
            (Some((lp, ldir)), Some((rp, rdir))) => {
                if ldir != rdir {
                    push(
                        DiffEntry {
                            path: key,
                            status: "type-changed".into(),
                            left_size: None,
                            right_size: None,
                            is_dir: *ldir,
                        },
                        &mut summary,
                        &mut entries,
                        &mut truncated,
                    );
                } else if *ldir {
                    push(
                        DiffEntry {
                            path: key,
                            status: "identical".into(),
                            left_size: None,
                            right_size: None,
                            is_dir: true,
                        },
                        &mut summary,
                        &mut entries,
                        &mut truncated,
                    );
                } else {
                    let lsize = std::fs::metadata(lp).map(|m| m.len()).ok();
                    let rsize = std::fs::metadata(rp).map(|m| m.len()).ok();
                    let status = if files_differ(lp, rp)? { "changed" } else { "identical" };
                    push(
                        DiffEntry {
                            path: key,
                            status: status.into(),
                            left_size: lsize,
                            right_size: rsize,
                            is_dir: false,
                        },
                        &mut summary,
                        &mut entries,
                        &mut truncated,
                    );
                }
            }
            (Some((lp, ldir)), None) => {
                let size = if *ldir { None } else { std::fs::metadata(lp).map(|m| m.len()).ok() };
                push(
                    DiffEntry {
                        path: key,
                        status: "removed".into(),
                        left_size: size,
                        right_size: None,
                        is_dir: *ldir,
                    },
                    &mut summary,
                    &mut entries,
                    &mut truncated,
                );
            }
            (None, Some((rp, rdir))) => {
                let size = if *rdir { None } else { std::fs::metadata(rp).map(|m| m.len()).ok() };
                push(
                    DiffEntry {
                        path: key,
                        status: "added".into(),
                        left_size: None,
                        right_size: size,
                        is_dir: *rdir,
                    },
                    &mut summary,
                    &mut entries,
                    &mut truncated,
                );
            }
            (None, None) => unreachable!(),
        }
    }

    summary.truncated = truncated;
    Ok(CompareResult { summary, entries })
}

#[tauri::command]
pub async fn compare_trees(left: String, right: String) -> Result<CompareResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        compare(Path::new(&left), Path::new(&right), DEFAULT_CAP)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, rel: &str, content: &[u8]) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    fn mkdir(dir: &Path, rel: &str) {
        std::fs::create_dir_all(dir.join(rel)).unwrap();
    }

    #[test]
    fn covers_all_statuses() {
        let base = std::env::temp_dir().join(format!("tk-cmp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let l = base.join("left");
        let r = base.join("right");
        std::fs::create_dir_all(&l).unwrap();
        std::fs::create_dir_all(&r).unwrap();

        mkdir(&l, "same_dir");
        mkdir(&r, "same_dir");
        touch(&l, "same_dir/inner.txt", b"x"); // dir with same-named child on both sides

        touch(&l, "identical.txt", b"same bytes");
        touch(&r, "identical.txt", b"same bytes");

        touch(&l, "changed.txt", b"old contents");
        touch(&r, "changed.txt", b"new contents");

        touch(&l, "same_size_differs.bin", &[1u8; 512]);
        touch(&r, "same_size_differs.bin", &[2u8; 512]);

        touch(&l, "removed_only.txt", b"bye");
        touch(&r, "added_only.txt", b"hi");

        touch(&l, "type_changed", b"was a file");
        mkdir(&r, "type_changed");
        touch(&r, "type_changed/now_dir.txt", b"");

        let res = compare(&l, &r, 1000).unwrap();
        let status_of = |p: &str| res.entries.iter().find(|e| e.path == p).map(|e| e.status.clone());

        assert_eq!(status_of("identical.txt").as_deref(), Some("identical"));
        assert_eq!(status_of("changed.txt").as_deref(), Some("changed"));
        assert_eq!(status_of("same_size_differs.bin").as_deref(), Some("changed"));
        assert_eq!(status_of("removed_only.txt").as_deref(), Some("removed"));
        assert_eq!(status_of("added_only.txt").as_deref(), Some("added"));
        assert_eq!(status_of("type_changed").as_deref(), Some("type-changed"));
        assert_eq!(status_of("same_dir").as_deref(), Some("identical"));
        assert_eq!(status_of("same_dir/inner.txt").as_deref(), Some("removed"));

        assert_eq!(res.summary.changed, 2);
        assert_eq!(res.summary.removed, 2);
        assert_eq!(res.summary.added, 2);
        assert_eq!(res.summary.type_changed, 1);
        assert!(!res.summary.truncated);

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn compares_large_files_in_chunks() {
        let base = std::env::temp_dir().join(format!("tk-cmp-big-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let l = base.join("left");
        let r = base.join("right");
        std::fs::create_dir_all(&l).unwrap();
        std::fs::create_dir_all(&r).unwrap();

        // 3 MiB: multi-chunk, differing only in the very last chunk.
        let mut big = vec![7u8; 3 * 1024 * 1024];
        touch(&l, "big.img", &big);
        let last = big.len() - 1;
        big[last] = 8;
        touch(&r, "big.img", &big);

        let res = compare(&l, &r, 100).unwrap();
        let e = res.entries.iter().find(|e| e.path == "big.img").unwrap();
        assert_eq!(e.status, "changed");
        assert_eq!(e.left_size, Some(3 * 1024 * 1024));
        assert_eq!(e.right_size, Some(3 * 1024 * 1024));

        // Flip it back: identical.
        std::fs::write(r.join("big.img"), vec![7u8; 3 * 1024 * 1024]).unwrap();
        let res = compare(&l, &r, 100).unwrap();
        let e = res.entries.iter().find(|e| e.path == "big.img").unwrap();
        assert_eq!(e.status, "identical");

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn truncates_at_cap() {
        let base = std::env::temp_dir().join(format!("tk-cmp-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let l = base.join("left");
        let r = base.join("right");
        std::fs::create_dir_all(&l).unwrap();
        std::fs::create_dir_all(&r).unwrap();
        for i in 0..20 {
            touch(&r, &format!("f{}.txt", i), b"new");
        }
        let res = compare(&l, &r, 10).unwrap();
        assert_eq!(res.entries.len(), 10);
        assert_eq!(res.summary.added, 20); // summary counts everything
        assert!(res.summary.truncated);

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn rejects_missing_sides() {
        let base = std::env::temp_dir().join(format!("tk-cmp-miss-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let l = base.join("nope");
        assert!(compare(&l, &base, 10).is_err());
        assert!(compare(&base, &l, 10).is_err());
    }
}
