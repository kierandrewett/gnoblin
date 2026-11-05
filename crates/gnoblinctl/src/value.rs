//! Typed values at the edges: a command line token in, JSON and text out.
//!
//! A schema type is the only description of an argument, so parsing and printing
//! go through [`gnoblin_schema::Type`] rather than through a per-member helper: a
//! type the schema adds is handled here once, and a token that does not fit is a
//! usage error rather than a frame the compositor has to refuse.
//!
//! Two conventions the schema does not state, and why:
//!
//! - `bytes` is written as hex (`00ff`), because a command line cannot carry
//!   arbitrary octets and hex is unambiguous.
//! - a `list<...>` argument is written as a JSON array (`'["a","b"]'`), because one
//!   token has to carry a variable-length value and JSON already says where it ends.

use gnoblin_model::{ObjectId, Value};
use gnoblin_schema::Type;

/// Parse one token as a value of `ty`.
pub(crate) fn parse(ty: &Type, text: &str) -> Result<Value, String> {
    match ty {
        Type::Bool => match text {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err(format!("`{text}` is not a bool; write `true` or `false`")),
        },
        Type::Int => text
            .parse::<i64>()
            .map(Value::Int)
            .map_err(|_| format!("`{text}` is not an int")),
        Type::Uint => text
            .parse::<u64>()
            .map(Value::Uint)
            .map_err(|_| format!("`{text}` is not a uint")),
        Type::Double => text
            .parse::<f64>()
            .map(Value::Double)
            .map_err(|_| format!("`{text}` is not a double")),
        Type::Text => Ok(Value::Text(text.to_owned())),
        Type::Bytes => from_hex(text).map(Value::Bytes),
        Type::ObjectRef(_) => text
            .parse::<u64>()
            .map(|raw| Value::Object(ObjectId::new(raw)))
            .map_err(|_| format!("`{text}` is not an object identity")),
        Type::List(inner) => {
            let items: Vec<serde_json::Value> = serde_json::from_str(text)
                .map_err(|error| format!("`{text}` is not a JSON array of {inner}: {error}"))?;
            items
                .iter()
                .map(|item| from_json(inner, item))
                .collect::<Result<Vec<Value>, String>>()
                .map(Value::List)
        }
    }
}

/// Build a value of `ty` from its JSON form.
///
/// This is the list-item path of [`parse`]; it is also what a JSON reader would
/// use, which is why it takes the type rather than guessing from the JSON kind.
fn from_json(ty: &Type, json: &serde_json::Value) -> Result<Value, String> {
    match (ty, json) {
        (Type::Bool, serde_json::Value::Bool(value)) => Ok(Value::Bool(*value)),
        (Type::Int, serde_json::Value::Number(number)) => number
            .as_i64()
            .map(Value::Int)
            .ok_or_else(|| format!("{number} is not an int")),
        (Type::Uint, serde_json::Value::Number(number)) => number
            .as_u64()
            .map(Value::Uint)
            .ok_or_else(|| format!("{number} is not a uint")),
        (Type::Double, serde_json::Value::Number(number)) => number
            .as_f64()
            .map(Value::Double)
            .ok_or_else(|| format!("{number} is not a double")),
        (Type::Text, serde_json::Value::String(text)) => Ok(Value::Text(text.clone())),
        (Type::Bytes, serde_json::Value::String(text)) => from_hex(text).map(Value::Bytes),
        (Type::ObjectRef(_), serde_json::Value::Number(number)) => number
            .as_u64()
            .map(|raw| Value::Object(ObjectId::new(raw)))
            .ok_or_else(|| format!("{number} is not an object identity")),
        (Type::List(inner), serde_json::Value::Array(items)) => items
            .iter()
            .map(|item| from_json(inner, item))
            .collect::<Result<Vec<Value>, String>>()
            .map(Value::List),
        (ty, json) => Err(format!("{json} is not a {ty}")),
    }
}

/// The JSON form of a value.
pub(crate) fn to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Bool(value) => serde_json::Value::Bool(*value),
        Value::Int(value) => serde_json::Value::from(*value),
        Value::Uint(value) => serde_json::Value::from(*value),
        Value::Double(value) => serde_json::Number::from_f64(*value).map_or_else(
            || serde_json::Value::String(value.to_string()),
            serde_json::Value::Number,
        ),
        Value::Text(value) => serde_json::Value::String(value.clone()),
        Value::Bytes(value) => serde_json::Value::String(to_hex(value)),
        Value::Object(value) => serde_json::Value::from(value.get()),
        Value::List(values) => serde_json::Value::Array(values.iter().map(to_json).collect()),
    }
}

/// The text form of a value, for a person rather than a parser.
pub(crate) fn display(value: &Value) -> String {
    match value {
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Uint(value) => value.to_string(),
        Value::Double(value) => value.to_string(),
        Value::Text(value) => format!("{value:?}"),
        Value::Bytes(value) => to_hex(value),
        Value::Object(value) => value.get().to_string(),
        Value::List(values) => {
            let items: Vec<String> = values.iter().map(display).collect();
            format!("[{}]", items.join(", "))
        }
    }
}

/// Hex, lower case, two digits per octet.
fn to_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from_digit(u32::from(byte >> 4), 16).expect("a nibble is a hex digit"));
        text.push(char::from_digit(u32::from(byte & 0x0f), 16).expect("a nibble is a hex digit"));
    }
    text
}

/// Hex back to octets; an odd length or a non-hex digit is a usage error.
fn from_hex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err(format!("`{text}` is not hex: an odd number of digits"));
    }
    let digits: Vec<u8> = text.bytes().collect();
    let mut bytes = Vec::with_capacity(digits.len() / 2);
    for pair in digits.as_chunks::<2>().0 {
        let high = hex_digit(pair[0]).ok_or_else(|| format!("`{text}` is not hex"))?;
        let low = hex_digit(pair[1]).ok_or_else(|| format!("`{text}` is not hex"))?;
        bytes.push(high << 4 | low);
    }
    Ok(bytes)
}

/// One hex digit.
fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_schema::Type;

    fn list_of(inner: Type) -> Type {
        Type::List(Box::new(inner))
    }

    #[test]
    fn a_bool_is_only_true_or_false() {
        assert_eq!(parse(&Type::Bool, "true").unwrap(), Value::Bool(true));
        assert_eq!(parse(&Type::Bool, "false").unwrap(), Value::Bool(false));
        assert!(parse(&Type::Bool, "yes").is_err());
    }

    #[test]
    fn the_numeric_types_are_distinct() {
        assert_eq!(parse(&Type::Int, "-7").unwrap(), Value::Int(-7));
        assert_eq!(parse(&Type::Uint, "7").unwrap(), Value::Uint(7));
        assert_eq!(parse(&Type::Double, "0.5").unwrap(), Value::Double(0.5));
        assert!(parse(&Type::Uint, "-7").is_err(), "a uint has no sign");
        assert!(parse(&Type::Int, "1.5").is_err(), "an int has no fraction");
    }

    #[test]
    fn text_is_taken_as_written() {
        assert_eq!(parse(&Type::Text, "a=b c").unwrap(), Value::Text("a=b c".to_owned()));
    }

    #[test]
    fn bytes_are_hex() {
        assert_eq!(parse(&Type::Bytes, "00ff7A").unwrap(), Value::Bytes(vec![0, 255, 122]));
        assert!(parse(&Type::Bytes, "abc").is_err(), "an odd digit count is not hex");
        assert!(parse(&Type::Bytes, "zz").is_err());
    }

    #[test]
    fn an_object_reference_is_an_identity() {
        assert_eq!(
            parse(&Type::ObjectRef("window".to_owned()), "12").unwrap(),
            Value::Object(ObjectId::new(12))
        );
        assert!(parse(&Type::ObjectRef("window".to_owned()), "one").is_err());
    }

    #[test]
    fn a_list_is_a_json_array_of_its_item_type() {
        assert_eq!(
            parse(&list_of(Type::Uint), "[1,2]").unwrap(),
            Value::List(vec![Value::Uint(1), Value::Uint(2)])
        );
        assert!(
            parse(&list_of(Type::Uint), "[\"a\"]").is_err(),
            "an item that does not fit the inner type is refused"
        );
        assert!(parse(&list_of(Type::Text), "a").is_err(), "a bare word is not an array");
    }

    #[test]
    fn text_is_quoted_when_shown_and_json_keeps_it_a_string() {
        assert_eq!(display(&Value::Text("First".to_owned())), "\"First\"");
        assert_eq!(to_json(&Value::Text("First".to_owned())), serde_json::json!("First"));
        assert_eq!(display(&Value::List(vec![Value::Bool(true)])), "[true]");
        assert_eq!(display(&Value::Bytes(vec![0, 15])), "000f");
        assert_eq!(to_json(&Value::Uint(7)), serde_json::json!(7));
    }

    #[test]
    fn a_double_that_json_cannot_hold_becomes_its_text() {
        assert_eq!(
            to_json(&Value::Double(f64::NAN)),
            serde_json::Value::String("NaN".to_owned())
        );
    }
}
