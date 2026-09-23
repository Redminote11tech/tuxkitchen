use tauri::{AppHandle, Emitter};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

// sdat2img in Rust: turn system.new.dat + system.transfer.list into a raw
// filesystem image.
//
// Transfer lists produced by img2sdat in full mode (what ROM kitchens build
// and ship) consist of `new` and `zero` commands only; `move` reuses blocks
// already written to the output. Incremental-OTA lists that require `bsdiff`
// or `imgdiff` patches against an old image are rejected rather than guessed,
// as are stash-sourced moves.

const BLOCK: u64 = 4096;

/// "2,0,3,5,6" -> [(0,3), (5,6)]: first token is the range count, then
/// start/end pairs, end exclusive.
fn parse_rangeset(s: &str) -> Result<Vec<(u64, u64)>, String> {
    let nums: Vec<u64> = s
        .split(',')
        .map(|t| t.parse::<u64>().map_err(|_| format!("bad rangeset token {:?}", t)))
        .collect::<Result<_, _>>()?;
    if nums.is_empty() || nums[0] as usize != (nums.len() - 1) / 2 {
        return Err(format!("rangeset count mismatch in {:?}", s));
    }
    let mut out = Vec::with_capacity(nums[0] as usize);
    let mut i = 1;
    while i + 1 < nums.len() {
        let (start, end) = (nums[i], nums[i + 1]);
        if end < start {
            return Err(format!("inverted range in {:?}", s));
        }
        out.push((start, end));
        i += 2;
    }
    Ok(out)
}

/// Apply one transfer list. Returns the final image size in blocks. The
/// logger keeps this testable without a Tauri AppHandle.
fn apply(transfer: &Path, dat: &Path, out: &Path, log: &mut impl FnMut(String)) -> Result<u64, String> {
    let text = fs::read_to_string(transfer)
        .map_err(|e| format!("cannot read {}: {}", transfer.display(), e))?;
    let lines: Vec<&str> = text.lines().map(|l| l.trim()).collect();
    let mut i = 0;

    let version: u64 = lines
        .get(i)
        .and_then(|l| l.parse().ok())
        .ok_or("transfer list: missing version")?;
    i += 1;
    if !(2..=4).contains(&version) {
        return Err(format!(
            "transfer list version {} not supported (need 2-4: the Android 5.1+ dat era)",
            version
        ));
    }
    let new_blocks: u64 = lines
        .get(i)
        .and_then(|l| l.parse().ok())
        .ok_or("transfer list: missing block count")?;
    i += 1;
    let _stash_count: u64 = lines
        .get(i)
        .and_then(|l| l.parse().ok())
        .ok_or("transfer list: missing stash count")?;
    i += 1;
    if version >= 4 {
        let _stash_entries: u64 = lines
            .get(i)
            .and_then(|l| l.parse().ok())
            .ok_or("transfer list: missing stash entry count")?;
        i += 1;
    }

    let mut dat_file = File::open(dat).map_err(|e| format!("cannot read {}: {}", dat.display(), e))?;
    // Read+write: `move` and `stash` read back blocks already written to
    // this same handle (File::create alone would be write-only).
    let mut out_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(out)
        .map_err(|e| format!("cannot create {}: {}", out.display(), e))?;
    out_file.set_len(new_blocks * BLOCK).map_err(|e| e.to_string())?;

    let mut stashes: HashMap<String, Vec<u8>> = HashMap::new();
    let mut stats = (0u64, 0u64, 0u64); // new, zero, move blocks

    let zero_buf = vec![0u8; BLOCK as usize * 256];

    while i < lines.len() {
        let cmd = lines[i];
        i += 1;
        if cmd.is_empty() {
            continue;
        }
        match cmd {
            "new" => {
                let dst = parse_rangeset(lines.get(i).ok_or("new: missing ranges")?)?;
                i += 1;
                for (start, end) in dst {
                    let mut remaining = end - start;
                    out_file.seek(SeekFrom::Start(start * BLOCK)).map_err(|e| e.to_string())?;
                    while remaining > 0 {
                        let mut block = vec![0u8; BLOCK as usize];
                        dat_file
                            .read_exact(&mut block)
                            .map_err(|_| "dat shorter than the transfer list expects".to_string())?;
                        out_file.write_all(&block).map_err(|e| e.to_string())?;
                        remaining -= 1;
                        stats.0 += 1;
                    }
                }
            }
            "zero" => {
                let dst = parse_rangeset(lines.get(i).ok_or("zero: missing ranges")?)?;
                i += 1;
                for (start, end) in dst {
                    let mut remaining = (end - start) * BLOCK;
                    out_file.seek(SeekFrom::Start(start * BLOCK)).map_err(|e| e.to_string())?;
                    while remaining > 0 {
                        let n = remaining.min(zero_buf.len() as u64) as usize;
                        out_file.write_all(&zero_buf[..n]).map_err(|e| e.to_string())?;
                        remaining -= n as u64;
                        stats.1 += n as u64 / BLOCK;
                    }
                }
            }
            "move" => {
                let onehash = lines.get(i).ok_or("move: missing hash")?.to_string();
                let src = parse_rangeset(lines.get(i + 1).ok_or("move: missing source ranges")?)?;
                let dst = parse_rangeset(lines.get(i + 2).ok_or("move: missing dest ranges")?)?;
                i += 3;
                if onehash == "-" {
                    return Err(
                        "stash-sourced move encountered - incremental OTA lists are not supported".into(),
                    );
                }
                // Read all source blocks before writing: source and dest may
                // overlap inside the output image.
                let mut buf = Vec::new();
                for (start, end) in &src {
                    out_file.seek(SeekFrom::Start(start * BLOCK)).map_err(|e| e.to_string())?;
                    let mut part = vec![0u8; ((end - start) * BLOCK) as usize];
                    out_file
                        .read_exact(&mut part)
                        .map_err(|_| "move source beyond written output".to_string())?;
                    buf.extend_from_slice(&part);
                }
                let mut offset = 0usize;
                for (start, end) in dst {
                    let n = ((end - start) * BLOCK) as usize;
                    if offset + n > buf.len() {
                        return Err("move destination larger than source".into());
                    }
                    out_file.seek(SeekFrom::Start(start * BLOCK)).map_err(|e| e.to_string())?;
                    out_file.write_all(&buf[offset..offset + n]).map_err(|e| e.to_string())?;
                    offset += n;
                    stats.2 += (n / BLOCK as usize) as u64;
                }
            }
            "stash" => {
                let hash = lines.get(i).ok_or("stash: missing hash")?.to_string();
                let src = parse_rangeset(lines.get(i + 1).ok_or("stash: missing ranges")?)?;
                i += 2;
                let mut buf = Vec::new();
                for (start, end) in &src {
                    out_file.seek(SeekFrom::Start(start * BLOCK)).map_err(|e| e.to_string())?;
                    let mut part = vec![0u8; ((end - start) * BLOCK) as usize];
                    out_file.read_exact(&mut part).map_err(|e| e.to_string())?;
                    buf.extend_from_slice(&part);
                }
                stashes.insert(hash, buf);
            }
            "free" => {
                let hash = lines.get(i).ok_or("free: missing hash")?.to_string();
                i += 1;
                stashes.remove(&hash);
            }
            "erase" => {
                i += 1; // discard request for a block device; meaningless on a file
            }
            "bsdiff" | "imgdiff" => {
                return Err(
                    "incremental OTA transfer list (bsdiff/imgdiff) needs the old image - not supported".into(),
                );
            }
            other => return Err(format!("unknown transfer command {:?}", other)),
        }
    }

    log(format!(
        "[sdat] applied: {} new block(s), {} zeroed, {} moved",
        stats.0, stats.1, stats.2
    ));
    Ok(new_blocks)
}

/// Convert `<prefix>.new.dat` (+ sibling `<prefix>.transfer.list`) to a raw
/// image at `output`.
pub fn convert(app: &AppHandle, dat_path: &str, output: &str) -> Result<String, String> {
    let dat = Path::new(dat_path);
    if !dat.is_file() {
        return Err(format!("dat not found: {}", dat_path));
    }
    let name = dat.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let prefix = name
        .strip_suffix(".new.dat")
        .map(|p| p.to_string())
        .unwrap_or_else(|| name.trim_end_matches(".dat").trim_end_matches('.').to_string());
    let transfer = dat
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("{}.transfer.list", prefix));
    if !transfer.is_file() {
        return Err(format!(
            "transfer list not found: {} - it must sit next to the dat",
            transfer.display()
        ));
    }
    let mut logger = |line: String| {
        let _ = app.emit("log-event", line);
    };
    let blocks = apply(&transfer, dat, Path::new(output), &mut logger)?;
    let out_path = Path::new(output).to_string_lossy().to_string();
    logger(format!("[sdat] wrote {} ({} blocks)", out_path, blocks));
    Ok(out_path)
}

#[tauri::command]
pub async fn sdat_to_img(
    app_handle: AppHandle,
    dat_path: String,
    output: String,
) -> Result<String, String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || convert(&app, &dat_path, &output))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_list(dir: &Path, lines: &[&str], dat: &[u8]) -> Vec<u8> {
        let transfer = dir.join("system.transfer.list");
        let dat_file = dir.join("system.new.dat");
        let out = dir.join("system.img");
        fs::write(&transfer, lines.join("\n")).unwrap();
        fs::write(&dat_file, dat).unwrap();
        let mut noop = |_: String| {};
        let blocks = apply(&transfer, &dat_file, &out, &mut noop).unwrap();
        assert_eq!(blocks, lines[1].parse::<u64>().unwrap());
        fs::read(&out).unwrap()
    }

    #[test]
    fn full_new_v4() {
        let dir = std::env::temp_dir().join(format!("tk-sdat-a-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let dat: Vec<u8> = (0..6 * BLOCK as usize).map(|i| (i % 253) as u8).collect();
        let out = run_list(&dir, &["4", "6", "0", "0", "new", "1,0,6"], &dat);
        assert_eq!(out, dat);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn zero_holes_and_multi_range_new() {
        let dir = std::env::temp_dir().join(format!("tk-sdat-b-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let blk = |fill: u8| vec![fill; BLOCK as usize];
        let mut dat = blk(0xAA);
        dat.extend(blk(0xBB));
        // image: block0 = 0xAA, block1 = zeros, block2 = 0xBB, block3 = zeros
        let out = run_list(
            &dir,
            &["4", "4", "0", "0", "new", "1,0,1", "zero", "1,1,2", "new", "1,2,3", "zero", "1,3,4"],
            &dat,
        );
        assert_eq!(&out[0..BLOCK as usize], &blk(0xAA)[..]);
        assert!(out[BLOCK as usize..2 * BLOCK as usize].iter().all(|&b| b == 0));
        assert_eq!(&out[2 * BLOCK as usize..3 * BLOCK as usize], &blk(0xBB)[..]);
        assert!(out[3 * BLOCK as usize..].iter().all(|&b| b == 0));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn move_reuses_blocks_within_output() {
        let dir = std::env::temp_dir().join(format!("tk-sdat-c-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let blk = |fill: u8| vec![fill; BLOCK as usize];
        let dat = [blk(0x11), blk(0x22)].concat();
        // copy block0 to block2, block1 to block3 (overlapping-source safe)
        let out = run_list(
            &dir,
            &["4", "4", "0", "0", "new", "1,0,2", "move", "deadbeef", "1,0,2", "1,2,4"],
            &dat,
        );
        assert_eq!(&out[2 * BLOCK as usize..3 * BLOCK as usize], &blk(0x11)[..]);
        assert_eq!(&out[3 * BLOCK as usize..4 * BLOCK as usize], &blk(0x22)[..]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_bsdiff_and_unknown_versions() {
        let dir = std::env::temp_dir().join(format!("tk-sdat-d-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let transfer = dir.join("system.transfer.list");
        let dat_file = dir.join("system.new.dat");
        let out = dir.join("system.img");
        let mut noop = |_: String| {};

        fs::write(&dat_file, vec![0u8; BLOCK as usize]).unwrap();
        fs::write(&transfer, "4\n1\n0\n0\nbsdiff\nhash\n1,0,1\n1,0,1\n").unwrap();
        assert!(apply(&transfer, &dat_file, &out, &mut noop)
            .unwrap_err()
            .contains("bsdiff"));

        fs::write(&transfer, "1\n1\nnew\n1,0,1\n").unwrap();
        assert!(apply(&transfer, &dat_file, &out, &mut noop)
            .unwrap_err()
            .contains("version 1"));

        fs::write(&transfer, "5\n1\n0\n0\nnew\n1,0,1\n").unwrap();
        assert!(apply(&transfer, &dat_file, &out, &mut noop)
            .unwrap_err()
            .contains("version 5"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rangeset_parses_strictly() {
        assert_eq!(parse_rangeset("1,0,6").unwrap(), vec![(0, 6)]);
        assert_eq!(parse_rangeset("2,0,3,5,6").unwrap(), vec![(0, 3), (5, 6)]);
        assert!(parse_rangeset("3,0,1").is_err());
        assert!(parse_rangeset("").is_err());
    }
}
