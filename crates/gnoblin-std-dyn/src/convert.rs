//! `Dyn` to the numbers, text and records of the table, with the errors a script is told.
//!
//! Every conversion names what it was converting (`argument `terms` of `collection_where``, `GnoblinStdPlacement.bounds`), so a
//! script that passes the wrong kind learns where, not just that.

use std::rc::Rc;

use crate::{Callable, Dyn, StdError};

/// What a [`Dyn`] is, in the words of an error.
fn kind(value: &Dyn) -> &'static str {
    match value {
        Dyn::Nil => "nothing",
        Dyn::Bool(_) => "a boolean",
        Dyn::Int(_) | Dyn::Uint(_) | Dyn::Float(_) => "a number",
        Dyn::Text(_) => "text",
        Dyn::List(_) => "a list",
        Dyn::Map(_) => "a record",
        Dyn::Object(_) => "an object",
    }
}

fn wrong(what: &str, wanted: &str, value: &Dyn) -> StdError {
    StdError::type_error(format!("{what} must be {wanted}, not {}", kind(value)))
}

impl Dyn {
    /// Whether this is [`Dyn::Nil`].
    #[must_use]
    pub fn is_nil(&self) -> bool {
        matches!(self, Dyn::Nil)
    }

    /// An unsigned integer: a whole, non-negative number, or a boolean as 0 and 1.
    ///
    /// # Errors
    ///
    /// The value is not a number, is negative, has a fraction, or does not fit.
    pub fn to_u64(&self, what: &str) -> Result<u64, StdError> {
        match self {
            Dyn::Bool(flag) => Ok(u64::from(*flag)),
            Dyn::Uint(number) => Ok(*number),
            Dyn::Int(number) => u64::try_from(*number).map_err(|_| wrong(what, "a non-negative number", self)),
            Dyn::Float(number) if number.fract() == 0.0 && *number >= 0.0 && *number < 18_446_744_073_709_551_616.0 => {
                Ok(*number as u64)
            }
            other => Err(wrong(what, "a whole non-negative number", other)),
        }
    }

    /// A signed integer.
    ///
    /// # Errors
    ///
    /// The value is not a whole number that fits.
    pub fn to_i64(&self, what: &str) -> Result<i64, StdError> {
        match self {
            Dyn::Bool(flag) => Ok(i64::from(*flag)),
            Dyn::Int(number) => Ok(*number),
            Dyn::Uint(number) => i64::try_from(*number).map_err(|_| wrong(what, "a smaller number", self)),
            Dyn::Float(number)
                if number.fract() == 0.0
                    && *number >= -9_223_372_036_854_775_808.0
                    && *number < 9_223_372_036_854_775_808.0 =>
            {
                Ok(*number as i64)
            }
            other => Err(wrong(what, "a whole number", other)),
        }
    }

    /// A `u32`.
    ///
    /// # Errors
    ///
    /// As [`Dyn::to_u64`], and the number is above `u32::MAX`.
    pub fn to_u32(&self, what: &str) -> Result<u32, StdError> {
        u32::try_from(self.to_u64(what)?).map_err(|_| wrong(what, "a number below 2^32", self))
    }

    /// A `u8`.
    ///
    /// # Errors
    ///
    /// As [`Dyn::to_u64`], and the number is above 255.
    pub fn to_u8(&self, what: &str) -> Result<u8, StdError> {
        u8::try_from(self.to_u64(what)?).map_err(|_| wrong(what, "a number below 256", self))
    }

    /// An `i32`.
    ///
    /// # Errors
    ///
    /// As [`Dyn::to_i64`], and the number does not fit.
    pub fn to_i32(&self, what: &str) -> Result<i32, StdError> {
        i32::try_from(self.to_i64(what)?).map_err(|_| wrong(what, "a smaller number", self))
    }

    /// A `usize`.
    ///
    /// # Errors
    ///
    /// As [`Dyn::to_u64`], and the number does not fit.
    pub fn to_usize(&self, what: &str) -> Result<usize, StdError> {
        usize::try_from(self.to_u64(what)?).map_err(|_| wrong(what, "a smaller number", self))
    }

    /// A floating-point number; an integer or a boolean is converted.
    ///
    /// # Errors
    ///
    /// The value is not a number.
    pub fn to_f64(&self, what: &str) -> Result<f64, StdError> {
        match self {
            Dyn::Float(number) => Ok(*number),
            Dyn::Int(number) => Ok(*number as f64),
            Dyn::Uint(number) => Ok(*number as f64),
            Dyn::Bool(flag) => Ok(f64::from(u8::from(*flag))),
            other => Err(wrong(what, "a number", other)),
        }
    }

    /// Text.
    ///
    /// # Errors
    ///
    /// The value is not text.
    pub fn to_text(&self, what: &str) -> Result<&str, StdError> {
        match self {
            Dyn::Text(text) => Ok(text),
            other => Err(wrong(what, "text", other)),
        }
    }

    /// A list.
    ///
    /// # Errors
    ///
    /// The value is not a list.
    pub fn to_list(&self, what: &str) -> Result<&[Dyn], StdError> {
        match self {
            Dyn::List(items) => Ok(items),
            other => Err(wrong(what, "a list", other)),
        }
    }

    /// A function or an object to call back. A record of functions is an object whose methods are its entries, so a script
    /// may hand a table of functions where it would hand a class.
    ///
    /// # Errors
    ///
    /// The value is neither.
    pub fn to_callable(&self, what: &str) -> Result<Rc<dyn Callable>, StdError> {
        match self {
            Dyn::Object(object) => Ok(Rc::clone(object)),
            Dyn::Map(entries) => Ok(Rc::new(Methods(entries.clone()))),
            other => Err(wrong(what, "a function or an object", other)),
        }
    }
}

/// A record of functions as an object.
struct Methods(Vec<(String, Dyn)>);

impl Methods {
    fn find(&self, name: &str) -> Option<&Rc<dyn Callable>> {
        self.0.iter().find_map(|(key, value)| match value {
            Dyn::Object(function) if key == name => Some(function),
            _ => None,
        })
    }
}

impl Callable for Methods {
    fn invoke(&self, _args: Vec<Dyn>) -> Result<Dyn, String> {
        Err("a record of functions is called by the name of one of them".to_owned())
    }

    fn invoke_method(&self, name: &str, args: Vec<Dyn>) -> Result<Dyn, String> {
        self.find(name)
            .ok_or_else(|| format!("the object has no method `{name}`"))?
            .invoke(args)
    }

    fn has_method(&self, name: &str) -> bool {
        self.find(name).is_some()
    }
}

/// Reads the fields of a record a script gave, and refuses a field the record does not have.
pub struct MapReader<'a> {
    what: &'a str,
    entries: &'a [(String, Dyn)],
    used: Vec<bool>,
}

impl<'a> MapReader<'a> {
    /// Reads `value` as the record called `what`. Nothing is the empty record.
    ///
    /// # Errors
    ///
    /// The value is neither a record nor nothing.
    pub fn new(value: &'a Dyn, what: &'a str) -> Result<MapReader<'a>, StdError> {
        let entries: &[(String, Dyn)] = match value {
            Dyn::Map(entries) => entries,
            Dyn::Nil => &[],
            other => return Err(wrong(what, "a record", other)),
        };
        Ok(MapReader {
            what,
            entries,
            used: vec![false; entries.len()],
        })
    }

    /// The field `key`, unless it is absent or nothing: an absent field keeps its zero.
    pub fn take(&mut self, key: &str) -> Option<&'a Dyn> {
        let at = self.entries.iter().position(|(name, _)| name == key)?;
        self.used[at] = true;
        match &self.entries[at].1 {
            Dyn::Nil => None,
            value => Some(value),
        }
    }

    /// The path of a field, for the message of a conversion error.
    #[must_use]
    pub fn path(&self, key: &str) -> String {
        format!("`{}.{key}`", self.what)
    }

    /// Fails on a field nothing took: a misspelt name is an error, not a silent zero.
    ///
    /// # Errors
    ///
    /// The record has a field the type does not.
    pub fn finish(self) -> Result<(), StdError> {
        match self.entries.iter().zip(&self.used).find(|(_, used)| !**used) {
            None => Ok(()),
            Some(((name, _), _)) => Err(StdError::type_error(format!("`{}` has no field `{name}`", self.what))),
        }
    }
}
