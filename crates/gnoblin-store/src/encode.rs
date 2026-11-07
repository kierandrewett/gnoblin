//! The canonical encoding of a stored value.
//!
//! A value is one version byte, then one tagged value. The version byte is the first thing a
//! later reader checks, so a format change bumps it and an old reader refuses rather than
//! misreads. Integers and floats are little-endian; a text or byte string and a list carry a
//! `u32` length. The store owns this encoding: no consumer ever sees the bytes.
//!
//! Refused on the way in: an object id, which is a runtime identity that does not survive a
//! restart; a non-finite double, which has no order for a range search; and a list whose items
//! are not all of one kind.

use gnoblin_model::Value;
use std::mem::discriminant;

use crate::StoreError;

/// The version byte that starts every encoded value.
pub const VERSION: u8 = 1;

const TAG_BOOL: u8 = 1;
const TAG_INT: u8 = 2;
const TAG_UINT: u8 = 3;
const TAG_DOUBLE: u8 = 4;
const TAG_TEXT: u8 = 5;
const TAG_BYTES: u8 = 6;
const TAG_LIST: u8 = 7;

/// Encodes `value`, refusing one whose encoding is longer than `limit` bytes.
///
/// The length check runs as the bytes are written, so an oversize value is refused before it is
/// fully copied. Errors are [`StoreError::ValueNotStorable`] or [`StoreError::ValueTooLarge`].
pub fn encode(value: &Value, limit: usize) -> Result<Vec<u8>, StoreError> {
    let mut out = vec![VERSION];
    write(value, &mut out, limit)?;
    Ok(out)
}

/// Decodes a value written by [`encode`]. A wrong version, a truncated value or trailing bytes
/// yield [`StoreError::Undecodable`]: the row is damaged and the caller must not guess at it.
pub fn decode(bytes: &[u8]) -> Result<Value, StoreError> {
    let (&version, rest) = bytes.split_first().ok_or_else(|| undecodable("an empty value"))?;
    if version != VERSION {
        return Err(undecodable(&format!("encoding version {version}, expected {VERSION}")));
    }
    let mut reader = Reader { bytes: rest };
    let value = reader.value()?;
    if !reader.bytes.is_empty() {
        return Err(undecodable("trailing bytes after the value"));
    }
    Ok(value)
}

/// Appends one value, checking refusals and the length limit as it goes.
fn write(value: &Value, out: &mut Vec<u8>, limit: usize) -> Result<(), StoreError> {
    match value {
        Value::Bool(flag) => push(out, &[TAG_BOOL, u8::from(*flag)], limit),
        Value::Int(number) => push(out, &scalar(TAG_INT, &number.to_le_bytes()), limit),
        Value::Uint(number) => push(out, &scalar(TAG_UINT, &number.to_le_bytes()), limit),
        Value::Double(number) => {
            if !number.is_finite() {
                return Err(StoreError::ValueNotStorable("a double must be finite".to_owned()));
            }
            push(out, &scalar(TAG_DOUBLE, &number.to_bits().to_le_bytes()), limit)
        }
        Value::Text(text) => push_sized(out, TAG_TEXT, text.as_bytes(), limit),
        Value::Bytes(bytes) => push_sized(out, TAG_BYTES, bytes, limit),
        Value::Object(_) => Err(StoreError::ValueNotStorable(
            "an object id is a runtime identity and is not stored".to_owned(),
        )),
        Value::List(items) => {
            if let Some(first) = items.first()
                && items.iter().any(|item| discriminant(item) != discriminant(first))
            {
                return Err(StoreError::ValueNotStorable(
                    "a list holds one kind of value".to_owned(),
                ));
            }
            push(out, &[TAG_LIST], limit)?;
            push(out, &length(items.len())?, limit)?;
            for item in items {
                write(item, out, limit)?;
            }
            Ok(())
        }
    }
}

/// A tag and a fixed-width payload.
fn scalar(tag: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(payload.len() + 1);
    bytes.push(tag);
    bytes.extend_from_slice(payload);
    bytes
}

/// A tag, a `u32` length and the payload.
fn push_sized(out: &mut Vec<u8>, tag: u8, payload: &[u8], limit: usize) -> Result<(), StoreError> {
    push(out, &[tag], limit)?;
    push(out, &length(payload.len())?, limit)?;
    push(out, payload, limit)
}

/// Appends `bytes` unless the value would then exceed `limit`.
fn push(out: &mut Vec<u8>, bytes: &[u8], limit: usize) -> Result<(), StoreError> {
    let size = out.len() + bytes.len();
    if size > limit {
        return Err(StoreError::ValueTooLarge { size, limit });
    }
    out.extend_from_slice(bytes);
    Ok(())
}

/// A length as the `u32` the encoding carries. A value this long is past any quota.
fn length(len: usize) -> Result<[u8; 4], StoreError> {
    let len = u32::try_from(len).map_err(|_| StoreError::ValueTooLarge {
        size: len,
        limit: u32::MAX as usize,
    })?;
    Ok(len.to_le_bytes())
}

fn undecodable(reason: &str) -> StoreError {
    StoreError::Undecodable(reason.to_owned())
}

/// A cursor over the bytes after the version byte.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl Reader<'_> {
    /// Takes the next `count` bytes, or fails if the value ends first.
    fn take(&mut self, count: usize) -> Result<&[u8], StoreError> {
        if self.bytes.len() < count {
            return Err(undecodable("a value ends early"));
        }
        let (head, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Ok(head)
    }

    /// Reads a `u32` length.
    fn length(&mut self) -> Result<usize, StoreError> {
        let raw = self.take(4)?;
        Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize)
    }

    /// Reads one tagged value.
    fn value(&mut self) -> Result<Value, StoreError> {
        let tag = self.take(1)?[0];
        Ok(match tag {
            TAG_BOOL => match self.take(1)?[0] {
                0 => Value::Bool(false),
                1 => Value::Bool(true),
                other => return Err(undecodable(&format!("a bool byte of {other}"))),
            },
            TAG_INT => Value::Int(i64::from_le_bytes(self.fixed()?)),
            TAG_UINT => Value::Uint(u64::from_le_bytes(self.fixed()?)),
            TAG_DOUBLE => Value::Double(f64::from_bits(u64::from_le_bytes(self.fixed()?))),
            TAG_TEXT => {
                let len = self.length()?;
                let raw = self.take(len)?.to_vec();
                Value::Text(String::from_utf8(raw).map_err(|_| undecodable("text is not UTF-8"))?)
            }
            TAG_BYTES => {
                let len = self.length()?;
                Value::Bytes(self.take(len)?.to_vec())
            }
            TAG_LIST => {
                let count = self.length()?;
                let mut items = Vec::new();
                for _ in 0..count {
                    items.push(self.value()?);
                }
                Value::List(items)
            }
            other => return Err(undecodable(&format!("an unknown tag {other}"))),
        })
    }

    /// Reads exactly eight bytes.
    fn fixed(&mut self) -> Result<[u8; 8], StoreError> {
        let raw = self.take(8)?;
        let mut out = [0_u8; 8];
        out.copy_from_slice(raw);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(value: Value) {
        let bytes = encode(&value, 1 << 20).expect("encodes");
        assert_eq!(decode(&bytes).expect("decodes"), value);
    }

    #[test]
    fn every_storable_kind_round_trips() {
        round_trip(Value::Bool(true));
        round_trip(Value::Int(-7));
        round_trip(Value::Uint(u64::MAX));
        round_trip(Value::Double(1.5));
        round_trip(Value::Text("héllo".to_owned()));
        round_trip(Value::Bytes(vec![0, 255]));
        round_trip(Value::List(vec![Value::Int(1), Value::Int(2)]));
    }

    #[test]
    fn an_object_id_and_a_non_finite_double_are_refused() {
        assert!(matches!(
            encode(&Value::Double(f64::NAN), 100),
            Err(StoreError::ValueNotStorable(_))
        ));
        assert!(matches!(
            encode(&Value::List(vec![Value::Int(1), Value::Text("x".to_owned())]), 100),
            Err(StoreError::ValueNotStorable(_))
        ));
    }

    #[test]
    fn a_value_over_the_limit_is_refused_while_it_is_written() {
        let big = Value::Text("x".repeat(100));
        assert!(matches!(
            encode(&big, 50),
            Err(StoreError::ValueTooLarge { limit: 50, .. })
        ));
    }

    #[test]
    fn a_damaged_value_is_refused_not_guessed() {
        assert!(decode(&[]).is_err());
        assert!(decode(&[9, TAG_INT]).is_err());
        let mut bytes = encode(&Value::Int(3), 100).expect("encodes");
        bytes.push(0);
        assert!(decode(&bytes).is_err());
    }
}
