//! Finding every newline in a text, a block at a time.

/// Calls `visit` with the byte offset of every `\n`, in order. Short lines
/// make a `memchr` call per line cost more than the scan itself, so the text
/// is scanned a block at a time and only the set bits are visited.
pub(super) fn for_each_newline(bytes: &[u8], visit: impl FnMut(usize)) {
    #[cfg(target_arch = "x86_64")]
    return for_each_newline_sse2(bytes, visit);
    #[cfg(not(target_arch = "x86_64"))]
    for_each_newline_swar(bytes, visit);
}

/// 16 bytes per step with SSE2, which every x86_64 CPU has.
#[cfg(target_arch = "x86_64")]
pub(super) fn for_each_newline_sse2(bytes: &[u8], mut visit: impl FnMut(usize)) {
    use std::arch::x86_64::{
        __m128i, _mm_cmpeq_epi8, _mm_loadu_si128, _mm_movemask_epi8, _mm_set1_epi8,
    };
    let (chunks, remainder) = bytes.as_chunks::<16>();
    let mut base = 0;
    // SAFETY: SSE2 is part of the x86_64 baseline; each load reads exactly
    // the 16 bytes of `chunk` (unaligned loads are allowed).
    let newline = unsafe { _mm_set1_epi8(b'\n' as i8) };
    for chunk in chunks {
        let mut mask = unsafe {
            let block = _mm_loadu_si128(chunk.as_ptr().cast::<__m128i>());
            _mm_movemask_epi8(_mm_cmpeq_epi8(block, newline)) as u32
        };
        while mask != 0 {
            visit(base + mask.trailing_zeros() as usize);
            mask &= mask - 1;
        }
        base += 16;
    }
    for (i, &byte) in remainder.iter().enumerate() {
        if byte == b'\n' {
            visit(base + i);
        }
    }
}

/// Eight bytes per step in plain integer arithmetic, for targets without
/// the SSE2 path (and as its oracle in the tests).
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
pub(super) fn for_each_newline_swar(bytes: &[u8], mut visit: impl FnMut(usize)) {
    const LOW_7: u64 = 0x7f7f_7f7f_7f7f_7f7f;
    const NEWLINES: u64 = 0x0a0a_0a0a_0a0a_0a0a;
    let (chunks, remainder) = bytes.as_chunks::<8>();
    let mut base = 0;
    for &chunk in chunks {
        let x = u64::from_le_bytes(chunk) ^ NEWLINES;
        // The high bit of each byte that was `\n` (so is zero in `x`), and
        // no other bit: the exact zero-byte test, free of false positives.
        let mut zero_bytes = !(((x & LOW_7) + LOW_7) | x | LOW_7);
        while zero_bytes != 0 {
            visit(base + (zero_bytes.trailing_zeros() / 8) as usize);
            zero_bytes &= zero_bytes - 1;
        }
        base += 8;
    }
    for (i, &byte) in remainder.iter().enumerate() {
        if byte == b'\n' {
            visit(base + i);
        }
    }
}

#[cfg(test)]
mod newline_offset_tests {
    use super::{for_each_newline, for_each_newline_swar};

    #[test]
    fn finds_exactly_the_newlines_at_every_alignment_and_length() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        // Bytes that stress the zero-byte test: 0x0a itself, its neighbours,
        // 0x8a (high bit set), 0x00 and 0xff.
        let alphabet = [b'\n', b'\n', 0x09, 0x0b, 0x8a, 0x00, 0xff, b'a', b' '];
        for len in 0..140 {
            for _ in 0..200 {
                let bytes: Vec<u8> = (0..len)
                    .map(|_| alphabet[(next() % alphabet.len() as u64) as usize])
                    .collect();
                let expected: Vec<usize> = bytes
                    .iter()
                    .enumerate()
                    .filter(|&(_, &b)| b == b'\n')
                    .map(|(i, _)| i)
                    .collect();
                let mut got = Vec::new();
                for_each_newline(&bytes, |at| got.push(at));
                assert_eq!(got, expected, "{bytes:?}");
                let mut swar = Vec::new();
                for_each_newline_swar(&bytes, |at| swar.push(at));
                assert_eq!(swar, expected, "swar {bytes:?}");
            }
        }
    }
}
