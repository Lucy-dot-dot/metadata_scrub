const MAX_DEPTH: usize = 16;
const XMP_UUID: [u8; 16] = [
    0xBE, 0x7A, 0xCF, 0xCB, 0x97, 0xA9, 0x42, 0xE8, 0x9C, 0x71, 0x99, 0x94, 0x91, 0xE3, 0xAF, 0xAC,
];

const MOOV_CHILDREN_TO_DROP: &[&[u8; 4]] = &[b"udta", b"meta", b"keys", b"ilst"];
const TRAK_CHILDREN_TO_DROP: &[&[u8; 4]] = &[b"udta", b"meta"];
const PLAIN_CONTAINERS: &[&[u8; 4]] = &[b"mdia", b"minf", b"stbl", b"edts", b"mvex"];
const TIME_BOXES: &[&[u8; 4]] = &[b"mvhd", b"tkhd", b"mdhd"];

pub fn scrub(input: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
    anyhow::ensure!(input.len() >= 8, "not an MP4/MOV file");
    let top = parse_boxes(input, 0, input.len(), 0)?;

    let mut removed = false;
    let mut out = Vec::with_capacity(input.len());
    for b in &top {
        let raw = &input[b.start..b.end];
        match &b.typ[..] {
            b"moov" => match scrub_container(input, b, MOOV_CHILDREN_TO_DROP, &mut removed)? {
                Some(cleaned) => out.extend_from_slice(&cleaned),
                None => out.extend_from_slice(raw),
            },
            b"uuid" if is_xmp_uuid(input, b) => removed = true,
            _ => out.extend_from_slice(raw),
        }
    }
    Ok(removed.then_some(out))
}

struct BoxRef {
    typ: [u8; 4],
    start: usize,
    end: usize,
    body: std::ops::Range<usize>,
    largesize: bool,
}

fn zero_creation_times(raw: &[u8]) -> Option<Vec<u8>> {
    if raw.len() < 12 {
        return None;
    }
    let version = raw[8];
    let time_range = match version {
        0 if raw.len() >= 20 => 12..20,
        1 if raw.len() >= 28 => 12..28,
        _ => return None,
    };
    if raw[time_range.clone()].iter().all(|&b| b == 0) {
        return None;
    }
    let mut out = raw.to_vec();
    out[time_range].fill(0);
    Some(out)
}

fn parse_boxes(data: &[u8], start: usize, end: usize, depth: usize) -> anyhow::Result<Vec<BoxRef>> {
    anyhow::ensure!(depth <= MAX_DEPTH, "corrupt MP4: nesting too deep");
    let mut boxes = Vec::new();
    let mut i = start;
    while i + 8 <= end {
        let size32 = u32::from_be_bytes(data[i..i + 4].try_into().unwrap()) as u64;
        let typ: [u8; 4] = data[i + 4..i + 8].try_into().unwrap();
        let (largesize, size, header_len) = if size32 == 1 {
            anyhow::ensure!(i + 16 <= end, "corrupt MP4: truncated largesize header");
            let s = u64::from_be_bytes(data[i + 8..i + 16].try_into().unwrap());
            (true, s, 16)
        } else if size32 == 0 {
            (false, (end - i) as u64, 8)
        } else {
            (false, size32, 8)
        };
        anyhow::ensure!(size >= header_len as u64, "corrupt MP4: box smaller than header");
        let box_end = i
            .checked_add(size as usize)
            .ok_or_else(|| anyhow::anyhow!("corrupt MP4: box size overflow"))?;
        anyhow::ensure!(box_end <= end, "corrupt MP4: box extends past end of file");
        boxes.push(BoxRef { typ, start: i, end: box_end, body: i + header_len..box_end, largesize });
        i = box_end;
    }
    Ok(boxes)
}

fn scrub_container(
    input: &[u8],
    parent: &BoxRef,
    drop_types: &[&[u8; 4]],
    removed: &mut bool,
) -> anyhow::Result<Option<Vec<u8>>> {
    let children = parse_boxes(input, parent.body.start, parent.body.end, MAX_DEPTH)?;
    let mut changed = false;
    let mut body = Vec::with_capacity(parent.body.len());

    for child in &children {
        let raw = &input[child.start..child.end];
        if drop_types.iter().any(|t| child.typ == **t) {
            changed = true;
            *removed = true;
            continue;
        }
        if TIME_BOXES.iter().any(|t| child.typ == **t)
            && let Some(zeroed) = zero_creation_times(raw)
        {
            changed = true;
            *removed = true;
            body.extend_from_slice(&zeroed);
            continue;
        }
        let plain_container = PLAIN_CONTAINERS.iter().any(|t| child.typ == **t);
        let (drop_list, recurse) = match &child.typ[..] {
            b"moov" => (Some(MOOV_CHILDREN_TO_DROP), true),
            b"trak" => (Some(TRAK_CHILDREN_TO_DROP), true),
            _ => (None, plain_container),
        };
        if recurse {
            let drops = drop_list.unwrap_or(&[]);
            match scrub_container(input, child, drops, removed)? {
                Some(cleaned) => {
                    changed = true;
                    body.extend_from_slice(&cleaned);
                }
                None => body.extend_from_slice(raw),
            }
        } else {
            body.extend_from_slice(raw);
        }
    }

    if !changed {
        return Ok(None);
    }

    let new_size = parent.header_len() as u64 + body.len() as u64;
    let mut out = Vec::with_capacity(new_size as usize);
    if new_size <= u32::MAX as u64 {
        out.extend_from_slice(&(new_size as u32).to_be_bytes());
        out.extend_from_slice(&parent.typ);
    } else {
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&parent.typ);
        out.extend_from_slice(&new_size.to_be_bytes());
    }
    out.extend_from_slice(&body);
    Ok(Some(out))
}

impl BoxRef {
    fn header_len(&self) -> usize {
        if self.largesize { 16 } else { 8 }
    }
}

fn is_xmp_uuid(input: &[u8], b: &BoxRef) -> bool {
    input[b.body.start..].starts_with(&XMP_UUID)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box32(typ: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(typ);
        v.extend_from_slice(body);
        v
    }

    fn fixture() -> Vec<u8> {
        let udta_gps = box32(b"udta", &box32(b"\xA9xyz", &[0xDE, 0xAD]));
        let tkhd = box32(b"tkhd", &[0, 0, 0, 0, 0xDE, 0xAD, 0xBE, 0xEF, 1, 2, 3, 4]);
        let trak = box32(b"trak", &[tkhd.as_slice(), udta_gps.as_slice()].concat());
        let ilst = box32(b"ilst", &box32(b"data", &[1, 2, 3]));
        let meta = box32(
            b"meta",
            [0, 0, 0, 0].iter().copied().chain(box32(b"hdlr", &[9])).collect::<Vec<u8>>().as_slice(),
        );
        let moov = box32(
            b"moov",
            &[box32(b"mvhd", &[0; 8]).as_slice(), trak.as_slice(), meta.as_slice(), box32(b"keys", &[7]).as_slice(), ilst.as_slice()].concat(),
        );
        let mut xmp = box32(b"uuid", &XMP_UUID);
        xmp.extend_from_slice(b"<xmp/>");
        [box32(b"ftyp", b"isom"), moov, box32(b"free", &[0xAA]), box32(b"mdat", &[1, 2, 3, 4, 5]), xmp].concat()
    }

    #[test]
    fn strips_metadata_boxes_keeps_media() {
        let input = fixture();
        let cleaned = scrub(&input).unwrap().unwrap();
        assert!(!cleaned.windows(4).any(|w| w == b"udta"));
        assert!(!cleaned.windows(4).any(|w| w == b"keys"));
        assert!(!cleaned.windows(4).any(|w| w == b"ilst"));
        assert!(!cleaned.windows(4).any(|w| w == b"\xA9xyz"));
        assert!(cleaned.windows(4).any(|w| w == b"ftyp"));
        assert!(cleaned.windows(4).any(|w| w == b"mvhd"));
        assert!(cleaned.windows(4).any(|w| w == b"tkhd"));
        assert!(cleaned.windows(4).any(|w| w == b"mdat"));
        let mdat_pos = cleaned.windows(4).position(|w| w == b"mdat").unwrap();
        assert_eq!(&cleaned[mdat_pos + 4..mdat_pos + 9], &[1, 2, 3, 4, 5]);

        let tkhd_pos = cleaned.windows(4).position(|w| w == b"tkhd").unwrap();
        assert_eq!(&cleaned[tkhd_pos + 8..tkhd_pos + 16], &[0; 8]);
        assert_eq!(&cleaned[tkhd_pos - 4..tkhd_pos], &[0, 0, 0, 20]);

        let mut offset = 0usize;
        while offset < cleaned.len() {
            let size = u32::from_be_bytes(cleaned[offset..offset + 4].try_into().unwrap()) as usize;
            assert!(size >= 8 && offset + size <= cleaned.len(), "box sizes corrupted at {offset}");
            offset += size;
        }
        assert_eq!(offset, cleaned.len());
    }

    #[test]
    fn clean_mp4_returns_none() {
        let input = [box32(b"ftyp", b"isom"), box32(b"mdat", &[1])].concat();
        assert!(scrub(&input).unwrap().is_none());
    }

}
