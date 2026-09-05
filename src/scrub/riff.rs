use anyhow::bail;

const EXIF_MASK: u8 = 0x08;
const XMP_MASK: u8 = 0x04;

pub fn scrub_webp(input: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
    anyhow::ensure!(
        input.len() >= 12 && &input[0..4] == b"RIFF" && &input[8..12] == b"WEBP",
        "not a WebP file"
    );

    let data = &input[12..];
    let chunks = parse_chunks(data, data.len())?;
    let mut removed = false;
    let mut out = Vec::with_capacity(input.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(b"WEBP");

    for (fourcc, range) in &chunks {
        let full = &data[range.start..range.end];
        if fourcc == b"EXIF" || fourcc == b"XMP " {
            removed = true;
            continue;
        }
        if fourcc == b"VP8X" && full.len() >= 12 {
            let mut chunk = full.to_vec();
            chunk[8] &= !(EXIF_MASK | XMP_MASK);
            out.extend_from_slice(&chunk);
            continue;
        }
        out.extend_from_slice(full);
    }

    if !removed {
        return Ok(None);
    }
    let riff_size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff_size.to_le_bytes());
    Ok(Some(out))
}

pub fn scrub_avi(input: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
    anyhow::ensure!(
        input.len() >= 12 && &input[0..4] == b"RIFF" && &input[8..12] == b"AVI ",
        "not an AVI file"
    );

    let data = &input[12..];
    let chunks = parse_chunks(data, data.len())?;
    let mut out = Vec::with_capacity(input.len());
    let mut removed = false;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(b"AVI ");

    for (fourcc, range) in &chunks {
        let full = &data[range.start..range.end];
        match scrub_avi_chunk(fourcc, full, &mut removed)? {
            Edit::Keep => out.extend_from_slice(full),
            Edit::Drop => {}
            Edit::Replace(cleaned) => out.extend_from_slice(&cleaned),
        }
    }

    if !removed {
        return Ok(None);
    }
    let riff_size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff_size.to_le_bytes());
    Ok(Some(out))
}

enum Edit {
    Keep,
    Drop,
    Replace(Vec<u8>),
}

fn scrub_avi_chunk(fourcc: &[u8; 4], full: &[u8], removed: &mut bool) -> anyhow::Result<Edit> {
    if fourcc == b"IDIT" {
        *removed = true;
        return Ok(Edit::Drop);
    }
    if fourcc != b"LIST" || full.len() < 12 {
        return Ok(Edit::Keep);
    }
    let list_type = &full[8..12];
    if list_type == b"INFO" {
        *removed = true;
        return Ok(Edit::Drop);
    }
    if list_type == b"movi" {
        return Ok(Edit::Keep);
    }

    let children = parse_chunks(&full[12..], full.len() - 12)?;
    let mut out = Vec::with_capacity(full.len());
    out.extend_from_slice(b"LIST");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(list_type);

    let mut changed = false;
    for (child_fourcc, range) in &children {
        let child = &full[12 + range.start..12 + range.end];
        match scrub_avi_chunk(child_fourcc, child, removed)? {
            Edit::Keep => out.extend_from_slice(child),
            Edit::Drop => changed = true,
            Edit::Replace(cleaned) => {
                changed = true;
                out.extend_from_slice(&cleaned);
            }
        }
    }

    if !changed {
        return Ok(Edit::Keep);
    }
    let list_size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&list_size.to_le_bytes());
    Ok(Edit::Replace(out))
}

/// Parse a sequence of RIFF chunks. Each returned range covers the entire chunk
/// including header and trailing pad byte.
fn parse_chunks(data: &[u8], limit: usize) -> anyhow::Result<Vec<([u8; 4], std::ops::Range<usize>)>> {
    let mut chunks = Vec::new();
    let mut i = 0usize;
    while i + 8 <= limit.min(data.len()) {
        let fourcc: [u8; 4] = data[i..i + 4].try_into().unwrap();
        let size = u32::from_le_bytes(data[i + 4..i + 8].try_into().unwrap()) as usize;
        let padded = size + (size & 1);
        let end = i
            .checked_add(8)
            .and_then(|e| e.checked_add(padded))
            .ok_or_else(|| anyhow::anyhow!("corrupt RIFF: chunk size overflow"))?;
        if end > data.len() {
            bail!("corrupt RIFF: chunk extends past end of file");
        }
        chunks.push((fourcc, i..end));
        i = end;
    }
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn riff(form: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&((payload.len() + 4) as u32).to_le_bytes());
        v.extend_from_slice(form);
        v.extend_from_slice(payload);
        v
    }

    fn chunk(fourcc: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = fourcc.to_vec();
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    #[test]
    fn webp_strips_exif_xmp_and_updates_vp8x_flags() {
        let mut vp8x = vec![0x10, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x0A, 0x00];
        vp8x[0] |= EXIF_MASK | XMP_MASK;
        let mut payload = chunk(b"VP8X", &vp8x);
        payload.extend(chunk(b"VP8 ", &[1, 2, 3]));
        payload.extend(chunk(b"EXIF", &[0xAA]));
        payload.extend(chunk(b"XMP ", b"<x/>"));
        let input = riff(b"WEBP", &payload);

        let cleaned = scrub_webp(&input).unwrap().unwrap();
        assert!(!cleaned.windows(4).any(|w| w == b"EXIF"));
        assert!(!cleaned.windows(4).any(|w| w == b"XMP "));
        assert!(cleaned.windows(4).any(|w| w == b"VP8 "));
        let vp8x_pos = cleaned.windows(4).position(|w| w == b"VP8X").unwrap();
        assert_eq!(cleaned[vp8x_pos + 8] & (EXIF_MASK | XMP_MASK), 0);
        let declared = u32::from_le_bytes(cleaned[4..8].try_into().unwrap());
        assert_eq!(declared as usize, cleaned.len() - 8);
    }

    #[test]
    fn webp_clean_returns_none() {
        let input = riff(b"WEBP", &chunk(b"VP8 ", &[1, 2, 3]));
        assert!(scrub_webp(&input).unwrap().is_none());
    }

    fn list(list_type: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut data = list_type.to_vec();
        data.extend_from_slice(payload);
        chunk(b"LIST", &data)
    }

    #[test]
    fn avi_strips_nested_info_list() {
        let info = list(b"INFO", b"ISFTlib");
        let strl = list(b"strl", &chunk(b"strh", &[1, 2]));
        let mut hdrl_payload = info;
        hdrl_payload.extend(&strl);
        let hdrl = list(b"hdrl", &hdrl_payload);
        let movi = list(b"movi", &chunk(b"00db", &[9, 9]));

        let mut payload = hdrl;
        payload.extend(&movi);
        payload.extend(chunk(b"IDIT", b"Wed Aug 29"));
        let input = riff(b"AVI ", &payload);

        let cleaned = scrub_avi(&input).unwrap().unwrap();
        assert!(!cleaned.windows(4).any(|w| w == b"INFO"));
        assert!(!cleaned.windows(4).any(|w| w == b"IDIT"));
        assert!(cleaned.windows(4).any(|w| w == b"movi"));
        assert!(cleaned.windows(4).any(|w| w == b"strl"));
        let declared = u32::from_le_bytes(cleaned[4..8].try_into().unwrap());
        assert_eq!(declared as usize, cleaned.len() - 8);
    }

    #[test]
    fn avi_clean_returns_none() {
        let input = riff(b"AVI ", &chunk(b"JUNK", &[1]));
        assert!(scrub_avi(&input).unwrap().is_none());
    }

}
