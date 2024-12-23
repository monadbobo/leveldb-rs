use crate::util::coding::decode_fixed32;

pub fn hash(data: &[u8], seed: u32) -> u32 {
    let m = 0xc6a4a793_u32;
    let r = 24_u32;
    let mut h = seed ^ (m.wrapping_mul(data.len() as u32));

    // Pick up four bytes at a time
    let mut chunks = data.chunks_exact(4);
    for chunk in &mut chunks {
        let w = decode_fixed32(chunk);
        h = h.wrapping_add(w);
        h = h.wrapping_mul(m);
        h ^= h >> 16;
    }

    // Pick up remaining bytes
    let remainder = chunks.remainder();
    match remainder.len() {
        3 => {
            h = h.wrapping_add((remainder[2] as u32) << 16);
            h = h.wrapping_add((remainder[1] as u32) << 8);
            h = h.wrapping_add(remainder[0] as u32);
            h = h.wrapping_mul(m);
            h ^= h >> r;
        }
        2 => {
            h = h.wrapping_add((remainder[1] as u32) << 8);
            h = h.wrapping_add(remainder[0] as u32);
            h = h.wrapping_mul(m);
            h ^= h >> r;
        }
        1 => {
            h = h.wrapping_add(remainder[0] as u32);
            h = h.wrapping_mul(m);
            h ^= h >> r;
        }
        _ => {}
    }

    h
}
