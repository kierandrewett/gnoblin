//! The wire form of [`Data`]: the options a layout reference carries to the compositor.
//!
//! The form is a version byte, then one tagged value. Each table is written in key
//! order, because [`Data::Table`] is a `BTreeMap`, so equal values encode to equal
//! bytes. Decoding is bounded in nesting, entry count and total length: the bytes come
//! from a configuration the compositor does not trust to be small.

use std::collections::BTreeMap;

use super::data::{Data, DataError, LayoutKey};

/// The version byte an encoded value starts with. A decoder refuses any other value, so
/// the form can change without a silently wrong read.
pub const ENCODING_VERSION: u8 = 1;

/// The most bytes one encoded value may occupy.
pub const MAX_ENCODED_LEN: usize = 1 << 20;

/// The deepest table nesting a decoded value may have.
const MAX_DEPTH: usize = 32;

/// The most table entries one decoded value may hold.
const MAX_ENTRIES: usize = 65_536;

const TAG_BOOLEAN: u8 = 0;
const TAG_INTEGER: u8 = 1;
const TAG_NUMBER: u8 = 2;
const TAG_STRING: u8 = 3;
const TAG_TABLE: u8 = 4;

const KEY_BOOLEAN: u8 = 0;
const KEY_INTEGER: u8 = 1;
const KEY_NUMBER: u8 = 2;
const KEY_STRING: u8 = 3;

impl Data {
    /// The canonical bytes of this value. Refused when the bytes would exceed
    /// [`MAX_ENCODED_LEN`], so an encoded value is always one [`Data::decode`] accepts.
    pub fn encode(&self) -> Result<Vec<u8>, DataError> {
        let mut out = vec![ENCODING_VERSION];
        write_value(self, &mut out);
        if out.len() > MAX_ENCODED_LEN {
            return Err(DataError(format!(
                "layout options encode to {} bytes, more than the {MAX_ENCODED_LEN} allowed",
                out.len()
            )));
        }
        Ok(out)
    }

    /// Read a value written by [`Data::encode`]. Refused on an unknown version, a
    /// truncated or trailing byte, an unknown tag, or a value past the nesting, entry
    /// and length bounds.
    pub fn decode(bytes: &[u8]) -> Result<Self, DataError> {
        if bytes.len() > MAX_ENCODED_LEN {
            return Err(DataError(format!(
                "layout options are {} bytes, more than the {MAX_ENCODED_LEN} allowed",
                bytes.len()
            )));
        }
        let (&version, rest) = bytes
            .split_first()
            .ok_or_else(|| DataError("layout options are empty".into()))?;
        if version != ENCODING_VERSION {
            return Err(DataError(format!(
                "layout options use encoding version {version}, not {ENCODING_VERSION}"
            )));
        }
        let mut reader = ModuleReader {
            bytes: rest,
            entries: MAX_ENTRIES,
        };
        let value = reader.value(0)?;
        if !reader.bytes.is_empty() {
            return Err(DataError("layout options have trailing bytes".into()));
        }
        Ok(value)
    }
}

fn write_value(value: &Data, out: &mut Vec<u8>) {
    match value {
        Data::Boolean(flag) => out.extend_from_slice(&[TAG_BOOLEAN, u8::from(*flag)]),
        Data::Integer(number) => {
            out.push(TAG_INTEGER);
            out.extend_from_slice(&number.to_be_bytes());
        }
        Data::Number(number) => {
            out.push(TAG_NUMBER);
            out.extend_from_slice(&number.to_bits().to_be_bytes());
        }
        Data::String(text) => {
            out.push(TAG_STRING);
            write_text(text, out);
        }
        Data::Table(map) => {
            out.push(TAG_TABLE);
            out.extend_from_slice(&(map.len() as u32).to_be_bytes());
            for (key, value) in map {
                write_key(key, out);
                write_value(value, out);
            }
        }
    }
}

fn write_key(key: &LayoutKey, out: &mut Vec<u8>) {
    match key {
        LayoutKey::Boolean(flag) => out.extend_from_slice(&[KEY_BOOLEAN, u8::from(*flag)]),
        LayoutKey::Integer(number) => {
            out.push(KEY_INTEGER);
            out.extend_from_slice(&number.to_be_bytes());
        }
        // A float key is kept by its bit pattern, so it round-trips exactly.
        LayoutKey::Number(bits) => {
            out.push(KEY_NUMBER);
            out.extend_from_slice(&bits.to_be_bytes());
        }
        LayoutKey::String(text) => {
            out.push(KEY_STRING);
            write_text(text, out);
        }
    }
}

fn write_text(text: &str, out: &mut Vec<u8>) {
    out.extend_from_slice(&(text.len() as u32).to_be_bytes());
    out.extend_from_slice(text.as_bytes());
}

/// A cursor over encoded bytes. `entries` counts down the table entries still allowed,
/// across the whole value, so a shared or wide table cannot exceed the budget.
struct ModuleReader<'a> {
    bytes: &'a [u8],
    entries: usize,
}

impl ModuleReader<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], DataError> {
        if self.bytes.len() < count {
            return Err(DataError("layout options are truncated".into()));
        }
        let (head, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Ok(head)
    }

    fn byte(&mut self) -> Result<u8, DataError> {
        Ok(self.take(1)?[0])
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], DataError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn length(&mut self) -> Result<usize, DataError> {
        Ok(u32::from_be_bytes(self.array::<4>()?) as usize)
    }

    fn text(&mut self) -> Result<String, DataError> {
        let length = self.length()?;
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| DataError("layout options hold text that is not UTF-8".into()))
    }

    fn value(&mut self, depth: usize) -> Result<Data, DataError> {
        Ok(match self.byte()? {
            TAG_BOOLEAN => Data::Boolean(self.byte()? != 0),
            TAG_INTEGER => Data::Integer(i64::from_be_bytes(self.array::<8>()?)),
            TAG_NUMBER => Data::Number(f64::from_bits(u64::from_be_bytes(self.array::<8>()?))),
            TAG_STRING => Data::String(self.text()?),
            TAG_TABLE => {
                if depth >= MAX_DEPTH {
                    return Err(DataError("layout options nest too deeply".into()));
                }
                let count = self.length()?;
                let mut map = BTreeMap::new();
                for _ in 0..count {
                    self.entries = self
                        .entries
                        .checked_sub(1)
                        .ok_or_else(|| DataError("layout options hold too many entries".into()))?;
                    let key = self.key()?;
                    let value = self.value(depth + 1)?;
                    map.insert(key, value);
                }
                Data::Table(map)
            }
            other => return Err(DataError(format!("layout options hold an unknown tag {other}"))),
        })
    }

    fn key(&mut self) -> Result<LayoutKey, DataError> {
        Ok(match self.byte()? {
            KEY_BOOLEAN => LayoutKey::Boolean(self.byte()? != 0),
            KEY_INTEGER => LayoutKey::Integer(i64::from_be_bytes(self.array::<8>()?)),
            KEY_NUMBER => LayoutKey::Number(u64::from_be_bytes(self.array::<8>()?)),
            KEY_STRING => LayoutKey::String(self.text()?),
            other => {
                return Err(DataError(format!("layout options hold an unknown key tag {other}")));
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(entries: Vec<(LayoutKey, Data)>) -> Data {
        Data::Table(entries.into_iter().collect())
    }

    fn sample() -> Data {
        table(vec![
            (LayoutKey::String("gap".into()), Data::Integer(8)),
            (LayoutKey::String("ratio".into()), Data::Number(0.5)),
            (LayoutKey::String("name".into()), Data::String("tall".into())),
            (LayoutKey::Integer(1), Data::Boolean(true)),
            (
                LayoutKey::String("nested".into()),
                table(vec![(LayoutKey::String("on".into()), Data::Boolean(false))]),
            ),
        ])
    }

    #[test]
    fn a_value_round_trips_through_its_canonical_bytes() {
        let value = sample();
        let bytes = value.encode().unwrap();
        assert_eq!(bytes[0], ENCODING_VERSION, "the first byte is the version");
        assert_eq!(Data::decode(&bytes).unwrap(), value);
    }

    #[test]
    fn equal_tables_encode_to_equal_bytes_whatever_their_insertion_order() {
        let forward = table(vec![
            (LayoutKey::String("a".into()), Data::Integer(1)),
            (LayoutKey::String("b".into()), Data::Integer(2)),
        ]);
        let backward = table(vec![
            (LayoutKey::String("b".into()), Data::Integer(2)),
            (LayoutKey::String("a".into()), Data::Integer(1)),
        ]);
        assert_eq!(forward.encode().unwrap(), backward.encode().unwrap());
    }

    #[test]
    fn a_float_round_trips_by_its_bit_pattern() {
        let value = Data::Number(-0.0);
        let decoded = Data::decode(&value.encode().unwrap()).unwrap();
        match decoded {
            Data::Number(number) => assert_eq!(number.to_bits(), (-0.0f64).to_bits()),
            other => panic!("expected a number, found {other:?}"),
        }
    }

    #[test]
    fn an_unknown_version_is_refused() {
        let mut bytes = sample().encode().unwrap();
        bytes[0] = ENCODING_VERSION + 1;
        let error = Data::decode(&bytes).unwrap_err();
        assert!(error.0.contains("encoding version"), "{error:?}");
    }

    #[test]
    fn an_empty_value_is_refused() {
        assert!(Data::decode(&[]).is_err());
    }

    #[test]
    fn a_truncated_value_is_refused_at_every_length() {
        let bytes = sample().encode().unwrap();
        for length in 1..bytes.len() {
            assert!(
                Data::decode(&bytes[..length]).is_err(),
                "a prefix of {length} bytes decoded"
            );
        }
    }

    #[test]
    fn trailing_bytes_are_refused() {
        let mut bytes = sample().encode().unwrap();
        bytes.push(0);
        assert!(Data::decode(&bytes).is_err());
    }

    #[test]
    fn an_unknown_tag_is_refused() {
        assert!(Data::decode(&[ENCODING_VERSION, 9]).is_err());
    }

    #[test]
    fn an_oversize_value_is_refused_on_encode_and_on_decode() {
        let big = Data::String("x".repeat(MAX_ENCODED_LEN));
        assert!(big.encode().is_err(), "the encoding is over the limit");
        let too_long = vec![ENCODING_VERSION; MAX_ENCODED_LEN + 1];
        assert!(Data::decode(&too_long).is_err(), "the input is over the limit");
    }

    #[test]
    fn a_table_nested_past_the_depth_bound_is_refused() {
        let mut value = Data::Boolean(true);
        for _ in 0..=MAX_DEPTH {
            value = table(vec![(LayoutKey::String("x".into()), value)]);
        }
        let bytes = value.encode().unwrap();
        assert!(Data::decode(&bytes).is_err(), "depth {} decoded", MAX_DEPTH + 1);
    }

    #[test]
    fn a_table_with_more_entries_than_the_bound_is_refused() {
        // Each entry is a boolean key with a boolean value: four bytes per entry.
        let count = MAX_ENTRIES + 1;
        let mut bytes = vec![ENCODING_VERSION, TAG_TABLE];
        bytes.extend_from_slice(&(count as u32).to_be_bytes());
        for _ in 0..count {
            bytes.extend_from_slice(&[KEY_BOOLEAN, 0, TAG_BOOLEAN, 0]);
        }
        let error = Data::decode(&bytes).unwrap_err();
        assert!(error.0.contains("too many entries"), "{error:?}");
    }
}
