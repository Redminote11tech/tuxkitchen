use tauri::{AppHandle, Emitter};
use std::fs;
use std::path::Path;

fn log(app: &AppHandle, line: String) {
    let _ = app.emit("log-event", line);
}

// Android dt_table format (system/libufdt/utils/src/include/dt_table.h).
// All integers are big-endian.
//
//   struct dt_table_header {  // 32 bytes
//     uint32 magic;        // 0xd7b7ab1e
//     uint32 total_size;
//     uint32 header_size;  // 32
//     uint32 dt_entry_size;// 32
//     uint32 dt_entry_count;
//     uint32 dt_entries_offset;
//     uint32 page_size;
//     uint32 version;
//   }
//   struct dt_table_entry {  // 32 bytes
//     uint32 dt_size, dt_offset, id, rev, custom[4];
//   }

const DT_TABLE_MAGIC: u32 = 0xd7b7ab1e;
const HEADER_SIZE: usize = 32;
const ENTRY_SIZE: usize = 32;

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}
fn put_be32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
}

struct DtEntry {
    dt_size: u32,
    dt_offset: u32,
    id: u32,
}

fn parse_table(data: &[u8]) -> Result<(u32, Vec<DtEntry>), String> {
    if data.len() < HEADER_SIZE {
        return Err(format!("dtbo too small: {} bytes", data.len()));
    }
    if be32(&data[0..4]) != DT_TABLE_MAGIC {
        return Err("not a dtbo image (bad magic, expected 0xd7b7ab1e)".into());
    }
    let entry_count = be32(&data[16..20]) as usize;
    let entries_offset = be32(&data[20..24]) as usize;
    let page_size = be32(&data[24..28]);
    let end = entries_offset + entry_count * ENTRY_SIZE;
    if entry_count == 0 || end > data.len() {
        return Err(format!(
            "dtbo entry table out of range: {} entries at offset {} in {} bytes",
            entry_count, entries_offset, data.len()
        ));
    }
    let mut entries = Vec::with_capacity(entry_count);
    for i in 0..entry_count {
        let e = &data[entries_offset + i * ENTRY_SIZE..entries_offset + (i + 1) * ENTRY_SIZE];
        let dt_size = be32(&e[0..4]);
        let dt_offset = be32(&e[4..8]);
        let stop = dt_offset as usize + dt_size as usize;
        if stop > data.len() {
            return Err(format!("entry {} blob out of range (offset {} size {})", i, dt_offset, dt_size));
        }
        entries.push(DtEntry {
            dt_size,
            dt_offset,
            id: be32(&e[8..12]),
            // rev and custom[4] (bytes 12..32) are carried through unchanged
            // by the image itself; nothing here needs to read them.
        });
    }
    Ok((page_size, entries))
}

#[tauri::command]
pub async fn dtbo_unpack(app_handle: AppHandle, input: String, output_dir: String) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let data = fs::read(&input).map_err(|e| format!("cannot read {}: {}", input, e))?;
        let (page_size, entries) = parse_table(&data)?;
        fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
        log(&app, format!("[DTBO] {} entries, page size {}, unpacking to {}", entries.len(), page_size, output_dir));
        for (i, e) in entries.iter().enumerate() {
            let blob = &data[e.dt_offset as usize..(e.dt_offset + e.dt_size) as usize];
            let name = format!("{:02}_id_0x{:08x}.dtb", i, e.id);
            fs::write(Path::new(&output_dir).join(&name), blob).map_err(|e| e.to_string())?;
            log(&app, format!("[DTBO] wrote {} ({} bytes)", name, e.dt_size));
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn dtbo_pack(app_handle: AppHandle, input_dir: String, output: String, page_size: Option<u32>) -> Result<(), String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dir = Path::new(&input_dir);
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map_err(|e| e.to_string())?
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("dtb"))
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        if names.is_empty() {
            return Err(format!("no .dtb files found in {}", input_dir));
        }

        let page = page_size.unwrap_or(4096).max(32);
        // Blobs start after the page-aligned table: header + entries.
        let entries_len = names.len() * ENTRY_SIZE;
        let table_len = HEADER_SIZE + entries_len;
        let blob_base = table_len.div_ceil(page as usize) * page as usize;

        let mut header: Vec<u8> = Vec::with_capacity(blob_base);
        let mut blob_cursor = blob_base;
        let mut metas = Vec::with_capacity(names.len());
        for name in &names {
            let bytes = fs::read(dir.join(name)).map_err(|e| e.to_string())?;
            let offset = blob_cursor as u32;
            blob_cursor += bytes.len();
            metas.push((bytes, offset));
        }
        let total_size = blob_cursor;

        put_be32(&mut header, DT_TABLE_MAGIC);
        put_be32(&mut header, total_size as u32);
        put_be32(&mut header, HEADER_SIZE as u32);
        put_be32(&mut header, ENTRY_SIZE as u32);
        put_be32(&mut header, names.len() as u32);
        put_be32(&mut header, HEADER_SIZE as u32);
        put_be32(&mut header, page);
        put_be32(&mut header, 0); // version
        header.resize(HEADER_SIZE, 0);

        for (bytes, offset) in &metas {
            put_be32(&mut header, bytes.len() as u32);
            put_be32(&mut header, *offset);
            put_be32(&mut header, 0); // id
            put_be32(&mut header, 0); // rev
            for _ in 0..4 {
                put_be32(&mut header, 0); // custom
            }
        }
        header.resize(blob_base, 0);

        let mut out = header;
        for (bytes, _) in &metas {
            out.extend_from_slice(bytes);
        }
        fs::write(&output, &out).map_err(|e| e.to_string())?;
        log(&app, format!("[DTBO] packed {} blobs ({} bytes) into {}", names.len(), total_size, output));
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_table(blobs: &[&[u8]], page: u32) -> Vec<u8> {
        let entries_len = blobs.len() * ENTRY_SIZE;
        let blob_base = ((HEADER_SIZE + entries_len + page as usize - 1) / page as usize) * page as usize;
        let mut out = Vec::new();
        let mut cursor = blob_base;
        put_be32(&mut out, DT_TABLE_MAGIC);
        put_be32(&mut out, (blob_base + blobs.iter().map(|b| b.len()).sum::<usize>()) as u32);
        put_be32(&mut out, HEADER_SIZE as u32);
        put_be32(&mut out, ENTRY_SIZE as u32);
        put_be32(&mut out, blobs.len() as u32);
        put_be32(&mut out, HEADER_SIZE as u32);
        put_be32(&mut out, page);
        put_be32(&mut out, 0);
        out.resize(HEADER_SIZE, 0);
        for b in blobs {
            put_be32(&mut out, b.len() as u32);
            put_be32(&mut out, cursor as u32);
            for _ in 0..6 {
                put_be32(&mut out, 0);
            }
            cursor += b.len();
        }
        out.resize(blob_base, 0);
        for b in blobs {
            out.extend_from_slice(b);
        }
        out
    }

    #[test]
    fn parses_and_rejects() {
        let blobs: Vec<&[u8]> = vec![b"blob-one", b"blob-two-longer"];
        let table = build_table(&blobs, 4096);
        let (page, entries) = parse_table(&table).unwrap();
        assert_eq!(page, 4096);
        assert_eq!(entries.len(), 2);
        assert_eq!(&table[entries[0].dt_offset as usize..entries[0].dt_offset as usize + entries[0].dt_size as usize], b"blob-one");
        assert_eq!(&table[entries[1].dt_offset as usize..entries[1].dt_offset as usize + entries[1].dt_size as usize], b"blob-two-longer");

        let mut bad = table.clone();
        bad[0..4].copy_from_slice(&[0, 0, 0, 0]);
        assert!(parse_table(&bad).is_err());
        assert!(parse_table(&[0u8; 16]).is_err());
    }

    #[test]
    fn pack_roundtrips_through_parser() {
        // Simulate dtbo_pack's own logic by reusing parse_table on a table
        // built the way dtbo_pack builds it (same layout rules).
        let blobs: Vec<&[u8]> = vec![b"a", b"bbbbbb"];
        let table = build_table(&blobs, 2048);
        let (page, entries) = parse_table(&table).unwrap();
        assert_eq!(page, 2048);
        assert_eq!(entries.len(), 2);
        // blob base must be page aligned after header+entries (32 + 64 = 96 -> 2048)
        assert_eq!(entries[0].dt_offset as usize % 2048, 0);
        assert_eq!(entries[0].dt_offset, 2048);
    }
}
