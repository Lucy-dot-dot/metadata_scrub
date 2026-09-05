const SOI: u8 = 0xD8;
const SOS: u8 = 0xDA;
const EOI: u8 = 0xD9;
const APP1: u8 = 0xE1;
const APP13: u8 = 0xED;
const COM: u8 = 0xFE;

const EXIF_ID: &[u8] = b"Exif\0\0";
const XMP_ID: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const PHOTOSHOP_ID: &[u8] = b"Photoshop 3.0\0";

pub fn scrub(input: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
    anyhow::ensure!(input.len() >= 4, "not a JPEG file");
    anyhow::ensure!(input[0] == 0xFF && input[1] == SOI, "not a JPEG file");

    let mut out = Vec::with_capacity(input.len());
    out.extend_from_slice(&input[0..2]);

    let mut i = 2usize;
    let mut removed = false;

    while i < input.len() {
        if input[i] != 0xFF {
            anyhow::bail!("corrupt JPEG: expected marker at offset {i}");
        }
        while i < input.len() && input[i] == 0xFF {
            i += 1;
        }
        let Some(&marker) = input.get(i) else {
            anyhow::bail!("corrupt JPEG: truncated marker");
        };
        i += 1;

        if marker == 0x00 {
            anyhow::bail!("corrupt JPEG: invalid marker at offset {}", i - 1);
        }

        if marker == SOS || marker == EOI {
            out.extend_from_slice(&input[i - 2..]);
            return Ok(removed.then_some(out));
        }

        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            out.push(0xFF);
            out.push(marker);
            continue;
        }

        let length = read_u16(input, i)? as usize;
        anyhow::ensure!(length >= 2, "corrupt JPEG: bad segment length at offset {i}");
        let segment_end = i.checked_add(length).ok_or_else(|| anyhow::anyhow!("corrupt JPEG: bad segment length"))?;
        anyhow::ensure!(segment_end <= input.len(), "corrupt JPEG: segment extends past end of file");

        let payload = &input[i + 2..segment_end];
        let drop = (marker == APP1 && (payload.starts_with(EXIF_ID) || payload.starts_with(XMP_ID)))
            || (marker == APP13 && payload.starts_with(PHOTOSHOP_ID))
            || marker == COM;

        if drop {
            removed = true;
        } else {
            out.extend_from_slice(&input[i - 2..segment_end]);
        }
        i = segment_end;
    }

    Ok(removed.then_some(out))
}

fn read_u16(data: &[u8], at: usize) -> anyhow::Result<u16> {
    let hi = *data.get(at).ok_or_else(|| anyhow::anyhow!("corrupt JPEG: truncated"))?;
    let lo = *data.get(at + 1).ok_or_else(|| anyhow::anyhow!("corrupt JPEG: truncated"))?;
    Ok(u16::from_be_bytes([hi, lo]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, marker];
        v.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        v.extend_from_slice(payload);
        v
    }

    fn fixture() -> Vec<u8> {
        let mut v = vec![0xFF, SOI];
        v.extend_from_slice(&seg(APP1, EXIF_ID)); // exif
        v.extend_from_slice(&seg(APP1, b"http://ns.adobe.com/xap/1.0/\0<xmp/>")); // xmp
        v.extend_from_slice(&seg(0xE2, b"ICC_PROFILE\0.....")); // icc: keep
        v.extend_from_slice(&seg(COM, b"made by a camera"));
        v.extend_from_slice(&seg(0xDB, &[1, 2, 3])); // dqt: keep
        v.push(0xFF);
        v.push(SOS);
        v.extend_from_slice(&[0xAB, 0xCD, 0x11, 0x22, 0xFF, 0xD9]);
        v
    }

    #[test]
    fn strips_exif_xmp_comments_keeps_icc_and_scan() {
        let input = fixture();
        let cleaned = scrub(&input).unwrap().unwrap();
        assert!(!cleaned.windows(2).any(|w| w == [0xFF, APP1]));
        assert!(!cleaned.windows(2).any(|w| w == [0xFF, COM]));
        assert!(cleaned.windows(11).any(|w| w == b"ICC_PROFILE".as_slice()));
        let scan_start = input.windows(2).rposition(|w| w == [0xFF, SOS]).unwrap();
        let cleaned_scan_start = cleaned.windows(2).rposition(|w| w == [0xFF, SOS]).unwrap();
        assert_eq!(&input[scan_start..], &cleaned[cleaned_scan_start..]);
    }

    #[test]
    fn clean_jpeg_returns_none() {
        let mut v = vec![0xFF, SOI];
        v.extend_from_slice(&seg(0xDB, &[1, 2, 3]));
        v.push(0xFF);
        v.push(SOS);
        v.extend_from_slice(&[1, 2, 3, 0xFF, 0xD9]);
        assert!(scrub(&v).unwrap().is_none());
    }
}
