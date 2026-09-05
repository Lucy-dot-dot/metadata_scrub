const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

const CHUNK_TYPES_TO_DROP: &[&[u8; 4]] = &[b"eXIf"];
const TEXT_CHUNK_TYPES: &[&[u8; 4]] = &[b"tEXt", b"zTXt", b"iTXt"];
const TEXT_KEYWORDS_TO_DROP: &[&str] = &[
    "XML:com.adobe.xmp",
    "Raw profile type exif",
    "Raw profile type iptc",
    "Raw profile type photoshop",
];

pub fn scrub(input: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
    anyhow::ensure!(
        input.len() >= 8 && input[0..8] == SIGNATURE,
        "not a PNG file"
    );

    let mut out = Vec::with_capacity(input.len());
    out.extend_from_slice(&SIGNATURE);

    let mut i = 8usize;
    let mut removed = false;

    while i < input.len() {
        anyhow::ensure!(i + 8 <= input.len(), "corrupt PNG: truncated chunk header");
        let length = u32::from_be_bytes(input[i..i + 4].try_into().unwrap()) as usize;
        let chunk_type = &input[i + 4..i + 8];
        let data_start = i + 8;
        let chunk_end = data_start
            .checked_add(length)
            .ok_or_else(|| anyhow::anyhow!("corrupt PNG: bad chunk length"))?
            .checked_add(4)
            .ok_or_else(|| anyhow::anyhow!("corrupt PNG: bad chunk length"))?;
        anyhow::ensure!(
            chunk_end <= input.len(),
            "corrupt PNG: chunk extends past end of file"
        );

        if chunk_should_be_dropped(chunk_type, &input[data_start..data_start + length]) {
            removed = true;
        } else {
            out.extend_from_slice(&input[i..chunk_end]);
        }
        i = chunk_end;
    }

    anyhow::ensure!(removed || out.len() > 8, "corrupt PNG: no chunks found");
    Ok(removed.then_some(out))
}

fn chunk_should_be_dropped(chunk_type: &[u8], data: &[u8]) -> bool {
    if CHUNK_TYPES_TO_DROP.iter().any(|t| chunk_type == *t) {
        return true;
    }
    if TEXT_CHUNK_TYPES.iter().any(|t| chunk_type == *t)
        && let Some(null) = data.iter().position(|&b| b == 0)
    {
        let keyword = String::from_utf8_lossy(&data[..null]);
        if TEXT_KEYWORDS_TO_DROP.iter().any(|k| keyword.eq_ignore_ascii_case(k)) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc32(data: &[u8]) -> u32 {
        let mut table = [0u32; 256];
        for (i, entry) in table.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *entry = c;
        }
        let mut crc = 0xFFFF_FFFFu32;
        for &b in data {
            crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
        }
        crc ^ 0xFFFF_FFFF
    }

    fn chunk(ctype: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = (data.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(ctype);
        v.extend_from_slice(data);
        let mut crc_input = ctype.to_vec();
        crc_input.extend_from_slice(data);
        v.extend_from_slice(&crc32(&crc_input).to_be_bytes());
        v
    }

    fn fixture() -> Vec<u8> {
        let mut v = SIGNATURE.to_vec();
        v.extend_from_slice(&chunk(b"IHDR", &[1, 2, 3, 4, 5, 6, 7, 8, 9, 0]));
        v.extend_from_slice(&chunk(b"eXIf", &[0xAA, 0xBB]));
        v.extend_from_slice(&chunk(b"iTXt", b"XML:com.adobe.xmp\0\0\0\0<x/>"));
        v.extend_from_slice(&chunk(b"tEXt", b"Software\0MyApp"));
        v.extend_from_slice(&chunk(b"IDAT", &[0x78, 0x9C]));
        v.extend_from_slice(&chunk(b"IEND", b""));
        v
    }

    #[test]
    fn strips_exif_and_xmp_keeps_other_text() {
        let input = fixture();
        let cleaned = scrub(&input).unwrap().unwrap();
        assert!(!cleaned.windows(4).any(|w| w == b"eXIf"));
        assert!(!cleaned.windows(4).any(|w| w == b"iTXt"));
        assert!(cleaned.windows(4).any(|w| w == b"tEXt"));
        assert!(cleaned.windows(4).any(|w| w == b"IDAT"));
        assert!(cleaned.windows(4).any(|w| w == b"IEND"));
        assert_eq!(&cleaned[..8], &SIGNATURE);
    }

    #[test]
    fn clean_png_returns_none() {
        let mut v = SIGNATURE.to_vec();
        v.extend_from_slice(&chunk(b"IHDR", &[0; 10]));
        v.extend_from_slice(&chunk(b"IDAT", &[1]));
        v.extend_from_slice(&chunk(b"IEND", b""));
        assert!(scrub(&v).unwrap().is_none());
    }
}
