const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode_bytes(input: &[u8]) -> String {
    if input.is_empty() {
        return String::new();
    }
    let mut out = Vec::with_capacity(input.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
        out.push(TABLE[((n >> 18) & 63) as usize]);
        out.push(TABLE[((n >> 12) & 63) as usize]);
        out.push(TABLE[((n >> 6) & 63) as usize]);
        out.push(TABLE[(n & 63) as usize]);
        i += 3;
    }
    let rem = input.len() - i;
    if rem == 1 {
        let n = (input[i] as u32) << 16;
        out.push(TABLE[((n >> 18) & 63) as usize]);
        out.push(TABLE[((n >> 12) & 63) as usize]);
        out.push(b'=');
        out.push(b'=');
    } else if rem == 2 {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        out.push(TABLE[((n >> 18) & 63) as usize]);
        out.push(TABLE[((n >> 12) & 63) as usize]);
        out.push(TABLE[((n >> 6) & 63) as usize]);
        out.push(b'=');
    }
    // SAFETY: TABLE and '=' are ASCII.
    unsafe { String::from_utf8_unchecked(out) }
}

pub fn encode_f32_le(values: &[f32]) -> String {
    if values.is_empty() {
        return String::new();
    }
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for &v in values {
        let clean = if v.is_finite() { v } else { 0.0 };
        bytes.extend_from_slice(&clean.to_le_bytes());
    }
    encode_bytes(&bytes)
}

pub fn decode_bytes(input: &str) -> Option<Vec<u8>> {
    let mut lut = [255u8; 256];
    for (i, &c) in TABLE.iter().enumerate() {
        lut[c as usize] = i as u8;
    }
    let bytes: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if bytes.is_empty() {
        return Some(Vec::new());
    }
    if bytes.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity((bytes.len() / 4) * 3);
    for chunk in bytes.chunks_exact(4) {
        let a = lut[chunk[0] as usize];
        let b = lut[chunk[1] as usize];
        if a == 255 || b == 255 {
            return None;
        }
        let c = if chunk[2] == b'=' { 0 } else { lut[chunk[2] as usize] };
        let d = if chunk[3] == b'=' { 0 } else { lut[chunk[3] as usize] };
        if (chunk[2] != b'=' && c == 255) || (chunk[3] != b'=' && d == 255) {
            return None;
        }
        let n = ((a as u32) << 18) | ((b as u32) << 12) | ((c as u32) << 6) | (d as u32);
        out.push(((n >> 16) & 0xff) as u8);
        if chunk[2] != b'=' {
            out.push(((n >> 8) & 0xff) as u8);
        }
        if chunk[3] != b'=' {
            out.push((n & 0xff) as u8);
        }
    }
    Some(out)
}
