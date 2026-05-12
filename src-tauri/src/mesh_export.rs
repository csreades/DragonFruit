/// Read a little-endian `f32` from a byte slice at the given byte offset.
#[inline(always)]
fn read_f32_le(data: &[u8], off: usize) -> f32 {
    f32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

/// Write binary STL directly to `dest_path` from raw triangle staging data.
///
/// Staging format: 9 × f32 per triangle (v0x v0y v0z  v1x v1y v1z  v2x v2y v2z), LE.
fn write_binary_stl(raw: &[u8], tri_count: usize, dest_path: &str) -> Result<(), String> {
    use std::io::{BufWriter, Write};

    let file =
        std::fs::File::create(dest_path).map_err(|e| format!("Failed creating STL file: {e}"))?;
    let mut w = BufWriter::with_capacity(256 * 1024, file);

    // 80-byte header (all zeros)
    w.write_all(&[0u8; 80])
        .map_err(|e| format!("STL write: {e}"))?;
    w.write_all(&(tri_count as u32).to_le_bytes())
        .map_err(|e| format!("STL write: {e}"))?;

    for i in 0..tri_count {
        let base = i * 36; // 9 floats × 4 bytes
        let v0x = read_f32_le(raw, base);
        let v0y = read_f32_le(raw, base + 4);
        let v0z = read_f32_le(raw, base + 8);
        let v1x = read_f32_le(raw, base + 12);
        let v1y = read_f32_le(raw, base + 16);
        let v1z = read_f32_le(raw, base + 20);
        let v2x = read_f32_le(raw, base + 24);
        let v2y = read_f32_le(raw, base + 28);
        let v2z = read_f32_le(raw, base + 32);

        // Face normal = cross(v1 - v0, v2 - v0), normalized
        let e1x = v1x - v0x;
        let e1y = v1y - v0y;
        let e1z = v1z - v0z;
        let e2x = v2x - v0x;
        let e2y = v2y - v0y;
        let e2z = v2z - v0z;
        let mut nx = e1y * e2z - e1z * e2y;
        let mut ny = e1z * e2x - e1x * e2z;
        let mut nz = e1x * e2y - e1y * e2x;
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        if len > 1e-30 {
            let inv = 1.0 / len;
            nx *= inv;
            ny *= inv;
            nz *= inv;
        }

        // Normal
        w.write_all(&nx.to_le_bytes())
            .map_err(|e| format!("STL write: {e}"))?;
        w.write_all(&ny.to_le_bytes())
            .map_err(|e| format!("STL write: {e}"))?;
        w.write_all(&nz.to_le_bytes())
            .map_err(|e| format!("STL write: {e}"))?;
        // 3 vertices
        w.write_all(&raw[base..base + 36])
            .map_err(|e| format!("STL write: {e}"))?;
        // Attribute byte count
        w.write_all(&0u16.to_le_bytes())
            .map_err(|e| format!("STL write: {e}"))?;
    }

    w.flush().map_err(|e| format!("STL flush: {e}"))?;
    Ok(())
}

/// Write a 3MF file (DEFLATE-compressed ZIP) directly to `dest_path` from raw
/// triangle staging data.
///
/// The `zip` crate handles DEFLATE compression, CRC32, and central directory
/// bookkeeping.  XML text for vertex/triangle tags compresses ~10–20× with
/// DEFLATE, so the resulting 3MF is typically smaller than the equivalent
/// binary STL.
fn write_3mf(raw: &[u8], tri_count: usize, dest_path: &str) -> Result<(), String> {
    use std::io::{BufWriter, Write};
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    let file =
        std::fs::File::create(dest_path).map_err(|e| format!("Failed creating 3MF file: {e}"))?;
    // Large outer buffer so 4 MB compressed chunks land on disk efficiently.
    let buf_writer = BufWriter::with_capacity(4 * 1024 * 1024, file);
    let mut zip = ZipWriter::new(buf_writer);

    // Level 1 = fastest deflate. Repetitive XML (vertex/triangle tags) still
    // compresses 5–8× even at level 1, so output size stays reasonable while
    // the compressor runs ~5× faster than the default level 6.
    let deflate_opts = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(1));

    // ── [Content_Types].xml ──
    zip.start_file("[Content_Types].xml", deflate_opts)
        .map_err(|e| format!("3MF zip: {e}"))?;
    zip.write_all(
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
          <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
          <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
          <Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>\
          </Types>",
    )
    .map_err(|e| format!("3MF zip: {e}"))?;

    // ── _rels/.rels ──
    zip.start_file("_rels/.rels", deflate_opts)
        .map_err(|e| format!("3MF zip: {e}"))?;
    zip.write_all(
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
          <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
          <Relationship Target=\"/3D/3dmodel.model\" Id=\"rel0\" \
          Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>\
          </Relationships>",
    )
    .map_err(|e| format!("3MF zip: {e}"))?;

    // ── 3D/3dmodel.model ──
    // The old approach called zip.write_all once per vertex/triangle — for a
    // 2 M-triangle mesh that is ~8 M individual DEFLATE feed calls, each with
    // full compressor overhead. Instead we accumulate XML into a 4 MB in-memory
    // chunk and flush to DEFLATE in bulk, reducing compressor calls by >1000×.
    zip.start_file("3D/3dmodel.model", deflate_opts)
        .map_err(|e| format!("3MF zip: {e}"))?;

    const CHUNK: usize = 4 * 1024 * 1024;
    let mut buf: Vec<u8> = Vec::with_capacity(CHUNK + 512);

    buf.extend_from_slice(
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
          <model unit=\"millimeter\" xml:lang=\"en-US\" \
          xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\
          <resources><object id=\"1\" type=\"model\"><mesh><vertices>",
    );

    // ── Vertices ──
    for i in 0..tri_count {
        let base = i * 36;
        for j in 0..3usize {
            let fbase = base + j * 12;
            let x = read_f32_le(raw, fbase);
            let y = read_f32_le(raw, fbase + 4);
            let z = read_f32_le(raw, fbase + 8);
            write!(buf, "<vertex x=\"{x:.4}\" y=\"{y:.4}\" z=\"{z:.4}\"/>").unwrap();
        }
        // Flush every ~4 MB so we never hold more than one chunk in memory.
        if buf.len() >= CHUNK {
            zip.write_all(&buf).map_err(|e| format!("3MF zip: {e}"))?;
            buf.clear();
        }
    }

    buf.extend_from_slice(b"</vertices><triangles>");

    // ── Triangles (sequential indices: tri i → 3i, 3i+1, 3i+2) ──
    for i in 0..tri_count {
        let v0 = i * 3;
        write!(
            buf,
            "<triangle v1=\"{v0}\" v2=\"{}\" v3=\"{}\"/>",
            v0 + 1,
            v0 + 2
        )
        .unwrap();
        if buf.len() >= CHUNK {
            zip.write_all(&buf).map_err(|e| format!("3MF zip: {e}"))?;
            buf.clear();
        }
    }

    buf.extend_from_slice(
        b"</triangles></mesh></object></resources>\
          <build><item objectid=\"1\"/></build></model>",
    );
    zip.write_all(&buf).map_err(|e| format!("3MF zip: {e}"))?;

    zip.finish().map_err(|e| format!("3MF zip finish: {e}"))?;
    Ok(())
}

/// Exports raw staged geometry to a properly formatted mesh file.
///
/// JS sends raw triangle vertex data (9 × f32 LE per triangle) to a staging
/// file via `append_mesh_stage_chunk`, then calls this command to convert
/// the staging file into a valid STL or 3MF at the user-chosen destination.
#[tauri::command]
pub(crate) async fn export_mesh_file(
    staging_path: String,
    dest_path: String,
    format: String,
) -> Result<String, String> {
    // Flush and release the staged file appender if it was writing to our staging file,
    // so all buffered bytes are written before we read.
    {
        let mut lock = crate::staged_mesh_file_appender()
            .lock()
            .map_err(|e| format!("Appender lock poisoned: {e}"))?;
        let matches = lock.as_ref().is_some_and(|a| a.path == staging_path);
        if matches {
            if let Some(appender) = lock.as_mut() {
                use std::io::Write;
                appender
                    .writer
                    .flush()
                    .map_err(|e| format!("Failed flushing staging appender: {e}"))?;
            }
            *lock = None; // release file handle
        }
    }

    let raw = std::fs::read(&staging_path)
        .map_err(|e| format!("Failed reading staging file '{}': {e}", staging_path))?;

    if raw.len() % 36 != 0 {
        return Err(format!(
            "Invalid staging data: {} bytes is not a multiple of 36 (9 × f32 per triangle)",
            raw.len()
        ));
    }
    let tri_count = raw.len() / 36;
    if tri_count == 0 {
        return Err("Cannot export: no triangles in staged geometry.".into());
    }

    log::info!(
        "[export_mesh_file] {} triangles → {} format → {}",
        tri_count,
        format,
        dest_path
    );

    match format.as_str() {
        "stl" => write_binary_stl(&raw, tri_count, &dest_path)?,
        "3mf" => write_3mf(&raw, tri_count, &dest_path)?,
        _ => return Err(format!("Unsupported export format: {format}")),
    }

    // Clean up staging file
    let _ = std::fs::remove_file(&staging_path);

    Ok(dest_path)
}
