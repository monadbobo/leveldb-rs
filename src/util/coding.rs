#[inline]
pub fn decode_fixed32(ptr: &[u8]) -> u32 {
    assert!(ptr.len() >= 4);
    u32::from_le_bytes([ptr[0], ptr[1], ptr[2], ptr[3]])
}

#[inline]
pub fn decode_fixed64(ptr: &[u8]) -> u64 {
    assert!(ptr.len() >= 8);
    u64::from_le_bytes([ptr[0], ptr[1], ptr[2], ptr[3], ptr[4], ptr[5], ptr[6], ptr[7]])
}

#[inline]
pub fn encode_fixed32(value: u32) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

#[inline]
pub fn encode_fixed64(value: u64) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

pub fn put_fixed32(value: u32) -> Vec<u8> {
    encode_fixed32(value)
}

pub fn put_fixed64(value: u64) -> Vec<u8> {
    encode_fixed64(value)
}

pub fn varint_length(v: u64) -> usize {
    let mut value = v;
    let mut len = 1;
    while value >= 128 {
        value >>= 7;
        len += 1;
    }
    len
}

pub fn encode_varint32(v: u32) -> Vec<u8> {
    const B: u8 = 128;
    let mut result = Vec::with_capacity(5);

    if v < (1 << 7) {
        result.push(v as u8);
    } else if v < (1 << 14) {
        result.push((v | B as u32) as u8);
        result.push((v >> 7) as u8);
    } else if v < (1 << 21) {
        result.push((v | B as u32) as u8);
        result.push(((v >> 7) | B as u32) as u8);
        result.push((v >> 14) as u8);
    } else if v < (1 << 28) {
        result.push((v | B as u32) as u8);
        result.push(((v >> 7) | B as u32) as u8);
        result.push(((v >> 14) | B as u32) as u8);
        result.push((v >> 21) as u8);
    } else {
        result.push((v | B as u32) as u8);
        result.push(((v >> 7) | B as u32) as u8);
        result.push(((v >> 14) | B as u32) as u8);
        result.push(((v >> 21) | B as u32) as u8);
        result.push((v >> 28) as u8);
    }

    result
}

pub fn put_varint32(v: u32) -> Vec<u8> {
    encode_varint32(v)
}

pub fn put_length_prefixed_slice(v: &[u8]) -> Vec<u8> {
    let mut result = put_varint32(v.len() as u32);
    result.extend_from_slice(v);
    result
}

pub fn encode_varint64(v: u64) -> Vec<u8> {
    const B: u8 = 128;
    let mut result = Vec::with_capacity(10);
    let mut value = v;

    while value >= B as u64 {
        result.push((value | B as u64) as u8);
        value >>= 7;
    }
    result.push(value as u8);

    result
}

pub fn put_varint64(v: u64) -> Vec<u8> {
    encode_varint64(v)
}

pub fn get_varint32(v: &[u8]) -> Option<(usize, u32)> {
    match v.first() {
        Some(&byte) if byte & 128 == 0 => Some((1, byte as u32)),
        Some(_) => get_varint32_fallback(v),
        None => None,
    }
}

pub fn get_varint32_fallback(v: &[u8]) -> Option<(usize, u32)> {
    let mut result = 0u32;
    let mut bytes_read = 0;

    for (shift, &byte) in v.iter().enumerate() {
        bytes_read += 1;
        let shift = shift * 7;

        if shift > 28 {
            return None; // Overflow
        }

        if byte & 128 != 0 {
            // More bytes are present
            result |= ((byte & 127) as u32) << shift;
        } else {
            result |= (byte as u32) << shift;
            return Some((bytes_read, result));
        }
    }

    None
}

pub fn get_length_prefixed_slice(v: &[u8]) -> Option<(usize, &[u8])> {
    let (bytes_read, len) = get_varint32(v)?;
    if v.len() >= len as usize {
        Some((bytes_read + len as usize, &v[bytes_read..bytes_read + len as usize]))
    } else {
        None
    }
}

pub fn get_varint64(p: &[u8]) -> Option<(usize, u64)> {
    let mut result = 0u64;
    let mut bytes_read = 0;

    for (shift, &byte) in p.iter().take(10).enumerate() {
        bytes_read += 1;
        let shift = shift * 7;

        if shift > 63 {
            return None; // Overflow
        }

        if byte & 128 != 0 {
            // More bytes are present
            result |= ((byte & 127) as u64) << shift;
        } else {
            result |= (byte as u64) << shift;
            return Some((bytes_read, result));
        }
    }

    None // Reached end without finding end of varint
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixed32() {
        let mut s = Vec::new();
        for v in 0..100000u32 {
            s.extend_from_slice(&put_fixed32(v));
        }

        let mut p = s.as_slice();
        for v in 0..100000u32 {
            let actual = decode_fixed32(p);
            assert_eq!(v, actual);
            p = &p[4..];
        }
    }

    #[test]
    fn test_fixed64() {
        let mut s = Vec::new();
        for power in 0..=63 {
            let v = 1u64 << power;
            s.extend_from_slice(&put_fixed64(v - 1));
            s.extend_from_slice(&put_fixed64(v));
            s.extend_from_slice(&put_fixed64(v + 1));
        }

        let mut p = s.as_slice();
        for power in 0..=63 {
            let v = 1u64 << power;
            let actual = decode_fixed64(p);
            assert_eq!(v - 1, actual);
            p = &p[8..];

            let actual = decode_fixed64(p);
            assert_eq!(v, actual);
            p = &p[8..];

            let actual = decode_fixed64(p);
            assert_eq!(v + 1, actual);
            p = &p[8..];
        }
    }

    #[test]
    fn test_encoding_output() {
        let dst = put_fixed32(0x04030201u32);
        assert_eq!(4, dst.len());
        assert_eq!(0x01, dst[0]);
        assert_eq!(0x02, dst[1]);
        assert_eq!(0x03, dst[2]);
        assert_eq!(0x04, dst[3]);

        let dst = put_fixed64(0x0807060504030201u64);
        assert_eq!(8, dst.len());
        assert_eq!(0x01, dst[0]);
        assert_eq!(0x02, dst[1]);
        assert_eq!(0x03, dst[2]);
        assert_eq!(0x04, dst[3]);
        assert_eq!(0x05, dst[4]);
        assert_eq!(0x06, dst[5]);
        assert_eq!(0x07, dst[6]);
        assert_eq!(0x08, dst[7]);
    }

    #[test]
    fn test_varint32() {
        let mut s = Vec::new();
        for i in 0..(32 * 32) {
            let v = (i / 32) << (i % 32);
            s.extend_from_slice(&put_varint32(v));
        }

        let mut p = s.as_slice();
        for i in 0..(32 * 32) {
            let expected = (i / 32) << (i % 32);
            let (consumed, actual) = get_varint32(p).unwrap();
            assert_eq!(expected, actual);
            assert_eq!(encode_varint32(actual).len(), consumed);
            p = &p[consumed..];
        }
        assert!(p.is_empty());
    }

    #[test]
    fn test_varint64() {
        let mut values = vec![0, 100, u64::MAX, u64::MAX - 1];
        for k in 0..64 {
            let power = 1u64 << k;
            values.push(power);
            values.push(power - 1);
            values.push(power + 1);
        }

        let mut s = Vec::new();
        for &value in &values {
            s.extend_from_slice(&put_varint64(value));
        }

        let mut p = s.as_slice();
        for &expected in &values {
            let (consumed, actual) = get_varint64(p).unwrap();
            assert_eq!(expected, actual);
            assert_eq!(encode_varint64(actual).len(), consumed);
            p = &p[consumed..];
        }
        assert!(p.is_empty());
    }

    #[test]
    fn test_varint32_overflow() {
        let input = vec![0x81, 0x82, 0x83, 0x84, 0x85, 0x11];
        assert!(get_varint32(&input).is_none());
    }

    #[test]
    fn test_varint32_truncation() {
        let large_value = (1u32 << 31) + 100;
        let s = put_varint32(large_value);
        for len in 0..s.len() - 1 {
            assert!(get_varint32(&s[..len]).is_none());
        }
        let (_, result) = get_varint32(&s).unwrap();
        assert_eq!(large_value, result);
    }

    #[test]
    fn test_varint64_overflow() {
        let input = vec![0x81, 0x82, 0x83, 0x84, 0x85, 0x81, 0x82, 0x83, 0x84, 0x85, 0x11];
        assert!(get_varint64(&input).is_none());
    }

    #[test]
    fn test_varint64_truncation() {
        let large_value = (1u64 << 63) + 100;
        let s = put_varint64(large_value);
        for len in 0..s.len() - 1 {
            assert!(get_varint64(&s[..len]).is_none());
        }
        let (_, result) = get_varint64(&s).unwrap();
        assert_eq!(large_value, result);
    }

    #[test]
    fn test_strings() {
        let mut s = Vec::new();
        let empty = vec![];
        s.extend_from_slice(&put_length_prefixed_slice(&empty));
        s.extend_from_slice(&put_length_prefixed_slice(b"foo"));
        s.extend_from_slice(&put_length_prefixed_slice(b"bar"));
        s.extend_from_slice(&put_length_prefixed_slice(&vec![b'x'; 200]));

        let mut input = s.as_slice();
        let (consumed, v) = get_length_prefixed_slice(input).unwrap();
        assert_eq!(b"", v);
        input = &input[consumed..];

        let (consumed, v) = get_length_prefixed_slice(input).unwrap();
        assert_eq!(b"foo", v);
        input = &input[consumed..];

        let (consumed, v) = get_length_prefixed_slice(input).unwrap();
        assert_eq!(b"bar", v);
        input = &input[consumed..];

        let (consumed, v) = get_length_prefixed_slice(input).unwrap();
        assert_eq!(vec![b'x'; 200], v);
        input = &input[consumed..];

        assert!(input.is_empty());
    }
}