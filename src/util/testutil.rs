use rand::rngs::ThreadRng;
use rand::thread_rng;
use rand::Rng;

pub fn random_string(rng: &mut ThreadRng, len: i32) -> String {
    let mut result = String::with_capacity(len as usize);
    for _ in 0..len {
        let c = (b' ' + rng.gen_range(0..95)) as char;
        result.push(c);
    }
    result
}

pub fn random_key(rng: &mut ThreadRng, len: i32) -> Vec<u8> {
    const TEST_CHARS: [u8; 10] = [
        b'\0', b'\x01', b'a', b'b', b'c', b'd', b'e', b'\xfd', b'\xfe', b'\xff',
    ];

    let mut result = Vec::with_capacity(len as usize);
    for _ in 0..len {
        let idx = rng.gen_range(0..TEST_CHARS.len());
        result.push(TEST_CHARS[idx]);
    }
    result
}

pub(crate) fn skewed(rng: &mut ThreadRng, max_log: u32) -> u32 {
    let base = rng.gen_range(0..=max_log);
    rng.gen_range(0..1u32.wrapping_shl(base))
}
