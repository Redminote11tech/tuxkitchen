use tauri::{AppHandle, Emitter};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn log(app: &AppHandle, line: String) {
    let _ = app.emit("log-event", line);
}

// Android OTA payload.bin (system/update_engine/update_metadata.proto).
// Header: "CrAU" + u64be major_version (2) + u64be manifest_size +
// u32be metadata_signature_size = 24 bytes. Blobs follow the manifest.
// The manifest is a protobuf DeltaArchiveManifest; only the subset needed
// for full-payload extraction is decoded here. Diff operations (SOURCE_COPY,
// BSDIFF, PUFFDIFF...) require the old image and are reported, not guessed.

const BLOCK: u64 = 4096; // overridden by the manifest's block_size when present

const OP_SOURCE_COPY: u64 = 0;
const OP_SOURCE_BSDIFF: u64 = 1;
const OP_SOURCE_MOVE: u64 = 2;
const OP_BSDIFF: u64 = 3;
const OP_SOURCE_HASH: u64 = 4;
const OP_ZERO: u64 = 5;
const OP_REPLACE: u64 = 6;
const OP_REPLACE_BZ: u64 = 7;
const OP_REPLACE_XZ: u64 = 8;
const OP_PUFFDIFF: u64 = 9;
const OP_SOURCE_BROTLI: u64 = 10;
const OP_BROTLI_BSDIFF: u64 = 11;

// ---- minimal protobuf reader ----

struct Proto<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Proto<'a> {
    fn new(data: &'a [u8]) -> Self {
        Proto { data, pos: 0 }
    }

    fn varint(&mut self) -> Option<u64> {
        let mut value = 0u64;
        let mut shift = 0;
        loop {
            let b = *self.data.get(self.pos)?;
            self.pos += 1;
            value |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Some(value);
            }
            shift += 7;
            if shift > 63 {
                return None;
            }
        }
    }

    /// Next (field_number, wire_type); None at end or on malformed data.
    fn field(&mut self) -> Option<(u64, u64)> {
        let key = self.varint()?;
        Some((key >> 3, key & 7))
    }

    /// Skip a value of the given wire type.
    fn skip(&mut self, wire: u64) -> Option<()> {
        match wire {
            0 => {
                self.varint()?;
            }
            1 => self.pos += 8,
            2 => {
                let len = self.varint()? as usize;
                self.pos += len;
            }
            5 => self.pos += 4,
            _ => return None,
        }
        if self.pos > self.data.len() {
            return None;
        }
        Some(())
    }

    /// Length-delimited payload of the current field, leaving pos past it.
    fn bytes(&mut self, wire: u64) -> Option<&'a [u8]> {
        if wire != 2 {
            self.skip(wire)?;
            return None;
        }
        let len = self.varint()? as usize;
        if self.pos + len > self.data.len() {
            return None;
        }
        let out = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Some(out)
    }
}

#[derive(Default)]
struct Extent {
    start: u64,
    num: u64,
}

#[derive(Default)]
struct Operation {
    op_type: u64,
    data_offset: u64,
    data_length: u64,
    dst: Vec<Extent>,
}

#[derive(Default)]
struct Partition {
    name: String,
    size: Option<u64>,
    ops: Vec<Operation>,
}

fn parse_extents(data: &[u8], out: &mut Vec<Extent>) {
    let mut p = Proto::new(data);
    let mut e = Extent::default();
    while let Some((f, w)) = p.field() {
        match (f, w) {
            (1, 0) => e.start = p.varint().unwrap_or(0),
            (2, 0) => {
                e.num = p.varint().unwrap_or(0);
                out.push(Extent { ..e });
                e = Extent::default();
            }
            _ => {
                p.skip(w);
            }
        }
    }
}

fn parse_operation(data: &[u8]) -> Operation {
    let mut op = Operation::default();
    let mut p = Proto::new(data);
    while let Some((f, w)) = p.field() {
        match (f, w) {
            (1, 0) => op.op_type = p.varint().unwrap_or(0),
            (2, 0) => op.data_offset = p.varint().unwrap_or(0),
            (3, 0) => op.data_length = p.varint().unwrap_or(0),
            (6, 2) => {
                if let Some(b) = p.bytes(w) {
                    parse_extents(b, &mut op.dst);
                }
            }
            _ => {
                p.skip(w);
            }
        }
    }
    op
}

fn parse_partition(data: &[u8]) -> Partition {
    let mut part = Partition::default();
    let mut p = Proto::new(data);
    while let Some((f, w)) = p.field() {
        match (f, w) {
            (1, 2) => {
                if let Some(b) = p.bytes(w) {
                    part.name = String::from_utf8_lossy(b).into_owned();
                }
            }
            (6, 2) => {
                // new_partition_info: ImageInfo { 1: size }
                if let Some(b) = p.bytes(w) {
                    let mut ip = Proto::new(b);
                    while let Some((ifield, iw)) = ip.field() {
                        match (ifield, iw) {
                            (1, 0) => part.size = ip.varint(),
                            _ => {
                                ip.skip(iw);
                            }
                        }
                    }
                }
            }
            (8, 2) => {
                if let Some(b) = p.bytes(w) {
                    part.ops.push(parse_operation(b));
                }
            }
            _ => {
                p.skip(w);
            }
        }
    }
    part
}

fn parse_manifest(data: &[u8]) -> (u64, Vec<Partition>) {
    let mut block_size = BLOCK;
    let mut parts = Vec::new();
    let mut p = Proto::new(data);
    while let Some((f, w)) = p.field() {
        match (f, w) {
            (3, 2) => {
                if let Some(b) = p.bytes(w) {
                    parts.push(parse_partition(b));
                }
            }
            (7, 0) => {
                if let Some(bs) = p.varint() {
                    if bs >= 512 {
                        block_size = bs;
                    }
                }
            }
            _ => {
                p.skip(w);
            }
        }
    }
    (block_size, parts)
}

/// Decompress a blob slice with a system tool, writing the plain bytes to dst.
fn decompress_with(tool: &str, args: &[&str], src: &Path, dst: &Path) -> Result<(), String> {
    let out_file = File::create(dst).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(tool);
    cmd.args(args).arg(src).stdout(Stdio::from(out_file)).stderr(Stdio::null());
    let status = cmd
        .status()
        .map_err(|e| format!("failed to spawn {}: {}", tool, e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} failed with status {}", tool, status))
    }
}

struct Blob<'a> {
    file: &'a Path,
    offset: u64,
    length: u64,
}

/// Apply one blob-backed operation: decompress (bzip2/xz) or copy, then place
/// the result into the destination extents of `out`.
fn apply_blob_op(op: &Operation, blob: &Blob, out: &mut File, tmp: &Path) -> Result<(), String> {
    let src = File::open(blob.file).map_err(|e| e.to_string())?;
    let mut reader = src;
    reader
        .seek(SeekFrom::Start(blob.offset))
        .map_err(|e| e.to_string())?;
    let mut tmp_in = File::create(tmp).map_err(|e| e.to_string())?;
    std::io::copy(&mut reader.take(blob.length), &mut tmp_in).map_err(|e| e.to_string())?;
    drop(tmp_in);

    let plain = if op.op_type == OP_REPLACE {
        tmp.to_path_buf()
    } else {
        let tool = if op.op_type == OP_REPLACE_BZ { "bzip2" } else { "xz" };
        let dst_plain = tmp.with_extension("plain");
        decompress_with(tool, &["-dc"], tmp, &dst_plain)?;
        dst_plain
    };

    let mut data = fs::File::open(&plain).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 256 * 1024];
    for e in &op.dst {
        let mut remaining = e.num * BLOCK;
        out.seek(SeekFrom::Start(e.start * BLOCK)).map_err(|e| e.to_string())?;
        while remaining > 0 {
            let want = remaining.min(buf.len() as u64) as usize;
            let got = std::io::Read::read(&mut data, &mut buf[..want]).map_err(|e| e.to_string())?;
            if got == 0 {
                return Err(format!(
                    "decompressed data shorter than destination extents (op type {})",
                    op.op_type
                ));
            }
            out.write_all(&buf[..got]).map_err(|e| e.to_string())?;
            remaining -= got as u64;
        }
    }
    Ok(())
}

fn write_zeros(op: &Operation, out: &mut File) -> Result<(), String> {
    let zero = vec![0u8; 256 * 1024];
    for e in &op.dst {
        let mut remaining = e.num * BLOCK;
        out.seek(SeekFrom::Start(e.start * BLOCK)).map_err(|e| e.to_string())?;
        while remaining > 0 {
            let want = remaining.min(zero.len() as u64) as usize;
            out.write_all(&zero[..want]).map_err(|e| e.to_string())?;
            remaining -= want as u64;
        }
    }
    Ok(())
}

fn dst_blocks(op: &Operation) -> u64 {
    op.dst.iter().map(|e| e.num).sum()
}

/// Extract every decodable partition of a full OTA payload into `output_dir`
/// as `<partition>.img`. Diff-based partitions are skipped and reported; the
/// successfully extracted ones stay on disk.
pub fn extract(app: &AppHandle, payload_path: &str, output_dir: &str) -> Result<Vec<String>, String> {
    let data = fs::read(payload_path)
        .map_err(|e| format!("cannot read {}: {}", payload_path, e))?;
    if data.len() < 24 || &data[0..4] != b"CrAU" {
        return Err("not a payload.bin (missing CrAU magic)".into());
    }
    let major = u64::from_be_bytes(data[4..12].try_into().unwrap());
    if major != 2 {
        return Err(format!("unsupported payload major version {}", major));
    }
    let manifest_size = u64::from_be_bytes(data[12..20].try_into().unwrap()) as usize;
    let sig_size = u32::from_be_bytes(data[20..24].try_into().unwrap()) as usize;
    if 24 + manifest_size + sig_size > data.len() {
        return Err("payload manifest extends past end of file".into());
    }
    let (block_size, parts) = parse_manifest(&data[24..24 + manifest_size]);
    if parts.is_empty() {
        return Err("payload manifest contains no partitions".into());
    }
    let blob_start = (24 + manifest_size + sig_size) as u64;

    fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;
    log(app, format!("[Payload] {} partition(s), block size {}", parts.len(), block_size));

    let payload_file = PathBuf::from(payload_path);
    let out_dir = Path::new(output_dir);
    let mut extracted = Vec::new();
    let mut skipped: Vec<(String, Vec<String>)> = Vec::new();
    let tmp = out_dir.join(".tk-blob.tmp");

    for part in &parts {
        if part.name.is_empty() {
            continue;
        }
        let dest = out_dir.join(format!("{}.img", part.name));
        let _ = fs::remove_file(&dest);
        let mut out = match File::create(&dest) {
            Ok(f) => f,
            Err(e) => {
                log(app, format!("[Payload] cannot create {}: {}", dest.display(), e));
                continue;
            }
        };
        let image_size = part
            .size
            .unwrap_or_else(|| part.ops.iter().map(dst_blocks).max().unwrap_or(0) * block_size);
        out.set_len(image_size).map_err(|e| e.to_string())?;

        let mut diff_ops: Vec<String> = Vec::new();
        for (i, op) in part.ops.iter().enumerate() {
            let result = if op.op_type == OP_ZERO {
                write_zeros(op, &mut out)
            } else if op.op_type == OP_REPLACE || op.op_type == OP_REPLACE_BZ || op.op_type == OP_REPLACE_XZ {
                apply_blob_op(
                    op,
                    &Blob { file: &payload_file, offset: blob_start + op.data_offset, length: op.data_length },
                    &mut out,
                    &tmp,
                )
            } else {
                if let Some(name) = diff_op_name(op.op_type) {
                    diff_ops.push(format!("op {} ({})", i, name));
                } else {
                    diff_ops.push(format!("op {} (type {})", i, op.op_type));
                }
                Ok(())
            };
            if let Err(e) = result {
                log(app, format!("[Payload] {} op {}: {}", part.name, i, e));
            }
        }

        if !diff_ops.is_empty() {
            drop(out);
            let _ = fs::remove_file(&dest);
            skipped.push((part.name.clone(), diff_ops));
        } else {
            log(app, format!("[Payload] extracted {} ({} bytes, {} ops)", part.name, image_size, part.ops.len()));
            extracted.push(part.name.clone());
        }
    }
    let _ = fs::remove_file(&tmp);
    let _ = fs::remove_file(tmp.with_extension("plain"));

    if !skipped.is_empty() {
        let detail: Vec<String> = skipped
            .iter()
            .map(|(n, ops)| format!("{} ({} diff op(s))", n, ops.len()))
            .collect();
        log(app, format!("[Payload] extracted: {}", extracted.join(", ")));
        return Err(format!(
            "full-payload extraction only: these partitions use delta operations that need the old image: {}",
            detail.join(", ")
        ));
    }
    if extracted.is_empty() {
        return Err("no partitions could be extracted from the payload".into());
    }
    Ok(extracted)
}

fn diff_op_name(t: u64) -> Option<&'static str> {
    match t {
        OP_SOURCE_COPY => Some("SOURCE_COPY"),
        OP_SOURCE_BSDIFF => Some("SOURCE_BSDIFF"),
        OP_SOURCE_MOVE => Some("SOURCE_MOVE"),
        OP_BSDIFF => Some("BSDIFF"),
        OP_SOURCE_HASH => Some("SOURCE_HASH"),
        OP_PUFFDIFF => Some("PUFFDIFF"),
        OP_SOURCE_BROTLI => Some("SOURCE_BROTLI"),
        OP_BROTLI_BSDIFF => Some("BROTLI_BSDIFF"),
        _ => None,
    }
}

#[tauri::command]
pub async fn extract_payload(app_handle: AppHandle, payload_path: String, output_dir: String) -> Result<Vec<String>, String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || extract(&app, &payload_path, &output_dir))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- protobuf writer helpers for the synthetic payload ----
    fn varint(v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        let mut v = v;
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
        out
    }
    fn key(field: u64, wire: u64) -> Vec<u8> {
        varint((field << 3) | wire)
    }
    fn len_field(field: u64, data: &[u8]) -> Vec<u8> {
        let mut out = key(field, 2);
        out.extend(varint(data.len() as u64));
        out.extend_from_slice(data);
        out
    }

    #[test]
    fn extracts_full_payload_partitions() {
        // manifest: block_size(7)=4096; partitions(3) = one "system"
        let mut extent_zero = key(1, 0);
        extent_zero.extend(varint(0)); // start_block
        extent_zero.extend(key(2, 0));
        extent_zero.extend(varint(1)); // num_blocks

        let mut op_zero = key(1, 0);
        op_zero.extend(varint(OP_ZERO));
        op_zero.extend(len_field(6, &extent_zero)); // dst_extents

        // REPLACE blobs always carry exactly dst_blocks * block_size bytes.
        let blob: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let mut extent_one = key(1, 0);
        extent_one.extend(varint(1)); // start_block
        extent_one.extend(key(2, 0));
        extent_one.extend(varint(1)); // num_blocks
        let mut op_replace = key(1, 0);
        op_replace.extend(varint(OP_REPLACE));
        op_replace.extend(key(2, 0));
        op_replace.extend(varint(0)); // data_offset
        op_replace.extend(key(3, 0));
        op_replace.extend(varint(blob.len() as u64)); // data_length
        op_replace.extend(len_field(6, &extent_one));

        let mut image_info = key(1, 0);
        image_info.extend(varint(2 * 4096)); // size

        let mut part = len_field(1, b"system");
        part.extend(len_field(6, &image_info));
        part.extend(len_field(8, &op_zero));
        part.extend(len_field(8, &op_replace));

        let mut manifest = key(7, 0);
        manifest.extend(varint(4096));
        manifest.extend(len_field(3, &part));

        // header
        let mut payload = Vec::new();
        payload.extend_from_slice(b"CrAU");
        payload.extend_from_slice(&2u64.to_be_bytes());
        payload.extend_from_slice(&(manifest.len() as u64).to_be_bytes());
        payload.extend_from_slice(&0u32.to_be_bytes());
        payload.extend_from_slice(&manifest);
        payload.extend_from_slice(&blob);

        let tmp = std::env::temp_dir().join(format!("tk-payload-{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        let pfile = tmp.join("payload.bin");
        fs::write(&pfile, &payload).unwrap();
        let outdir = tmp.join("out");
        fs::create_dir_all(&outdir).unwrap();

        let (bs, parts) = parse_manifest(&manifest);
        assert_eq!(bs, 4096);
        assert_eq!(parts.len(), 1);

        // run the real extraction through a fake app-less path: extract() needs
        // an AppHandle only for logging, so test the pieces it composes.
        let parsed = parse_manifest(&manifest).1;
        assert_eq!(parsed[0].ops.len(), 2);
        assert_eq!(parsed[0].ops[0].op_type, OP_ZERO);
        assert_eq!(parsed[0].ops[1].data_length, blob.len() as u64);
        assert_eq!(dst_blocks(&parsed[0].ops[1]), 1);

        let mut out = File::create(outdir.join("system.img")).unwrap();
        out.set_len(2 * 4096).unwrap();
        write_zeros(&parsed[0].ops[0], &mut out).unwrap();
        let b = Blob { file: &pfile, offset: (24 + manifest.len()) as u64, length: blob.len() as u64 };
        apply_blob_op(&parsed[0].ops[1], &b, &mut out, &tmp.join(".t")).unwrap();
        drop(out);

        let got = fs::read(outdir.join("system.img")).unwrap();
        assert_eq!(got.len(), 8192);
        assert!(got[..4096].iter().all(|&b| b == 0));
        assert_eq!(&got[4096..4096 + blob.len()], blob);

        fs::remove_dir_all(&tmp).unwrap();
    }
}
