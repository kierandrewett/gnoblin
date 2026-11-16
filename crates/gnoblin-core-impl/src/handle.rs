//! Handle encoding.
//!
//! A handle the shell holds (a context, a registration) is a `u64`: 24 bits of tag above 40 bits of index. The index is
//! a sequence number the core never reuses; the tag is a hash of the index and the handle's kind. A value that is not a
//! handle the core issued, or that is a registration passed where a context is wanted, fails the tag test and is
//! rejected as stale instead of being looked up, so a shell's arithmetic mistake cannot reach another object.

/// The kind of a generation context handle.
pub(crate) const KIND_CTX: u64 = 1;
/// The kind of a registration handle.
pub(crate) const KIND_REG: u64 = 2;

const INDEX_BITS: u32 = 40;
const INDEX_MASK: u64 = (1 << INDEX_BITS) - 1;
const TAG_MASK: u64 = 0xFF_FFFF;

fn tag(kind: u64, index: u64) -> u64 {
    let mixed = (index ^ (kind << 56)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    ((mixed >> INDEX_BITS) & TAG_MASK).max(1)
}

/// The handle for `index` (which must be non-zero and below 2^40) of `kind`.
pub(crate) fn encode(kind: u64, index: u64) -> u64 {
    debug_assert!(index != 0 && index <= INDEX_MASK);
    (tag(kind, index) << INDEX_BITS) | index
}

/// The index inside `handle`, when it is a well-formed handle of `kind`.
pub(crate) fn decode(kind: u64, handle: u64) -> Option<u64> {
    let index = handle & INDEX_MASK;
    (index != 0 && handle >> INDEX_BITS == tag(kind, index)).then_some(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handle_round_trips() {
        for index in [1, 2, 77, INDEX_MASK] {
            assert_eq!(decode(KIND_CTX, encode(KIND_CTX, index)), Some(index));
            assert_eq!(decode(KIND_REG, encode(KIND_REG, index)), Some(index));
        }
    }

    #[test]
    fn a_handle_of_the_wrong_kind_is_rejected() {
        let ctx = encode(KIND_CTX, 5);
        assert_eq!(decode(KIND_REG, ctx), None);
        assert_eq!(decode(KIND_CTX, ctx), Some(5));
    }

    #[test]
    fn zero_and_forged_values_are_rejected() {
        assert_eq!(decode(KIND_CTX, 0), None);
        assert_eq!(decode(KIND_CTX, 7), None, "a bare index has no tag");
        let good = encode(KIND_CTX, 9);
        assert_eq!(decode(KIND_CTX, good ^ (1 << 41)), None, "a flipped tag bit");
        assert_eq!(decode(KIND_CTX, u64::MAX), None);
    }
}
