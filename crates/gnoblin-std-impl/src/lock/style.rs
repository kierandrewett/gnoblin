//! The look of the lock screen and what it says: the `lock_style` record, its stock values, and reading one a shell gave.

use gnoblin_object::{Fail, Res, Val};

use crate::object::record::{field, record};

/// Everything that sets how the screen looks and what it says. Lengths are logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LockStyle {
    pub(crate) scrim_fill: String,
    pub(crate) card_width: f64,
    pub(crate) card_height: f64,
    pub(crate) card_radius: f64,
    pub(crate) card_padding: f64,
    pub(crate) card_gap: f64,
    pub(crate) card_fill: String,
    pub(crate) card_top: f64,
    pub(crate) clock_size: f64,
    pub(crate) clock_color: String,
    pub(crate) entry_height: f64,
    pub(crate) entry_radius: f64,
    pub(crate) entry_fill: String,
    pub(crate) entry_size: f64,
    pub(crate) entry_color: String,
    pub(crate) entry_limit: u64,
    pub(crate) message_size: f64,
    pub(crate) hint_color: String,
    pub(crate) error_color: String,
    pub(crate) hint: String,
    pub(crate) checking: String,
    pub(crate) denied: String,
    pub(crate) busy: String,
    pub(crate) failed: String,
    pub(crate) rate_limited: String,
}

impl Default for LockStyle {
    /// The stock look of the lock screen.
    fn default() -> LockStyle {
        LockStyle {
            scrim_fill: "#11111bf2".to_owned(),
            card_width: 360.0,
            card_height: 220.0,
            card_radius: 16.0,
            card_padding: 20.0,
            card_gap: 12.0,
            card_fill: "#1e1e2e".to_owned(),
            card_top: 1.0 / 3.0,
            clock_size: 44.0,
            clock_color: "#cdd6f4".to_owned(),
            entry_height: 44.0,
            entry_radius: 10.0,
            entry_fill: "#313244".to_owned(),
            entry_size: 15.0,
            entry_color: "#cdd6f4".to_owned(),
            entry_limit: 1024,
            message_size: 13.0,
            hint_color: "#a6adc8".to_owned(),
            error_color: "#f38ba8".to_owned(),
            hint: "Enter your password.".to_owned(),
            checking: "Checking...".to_owned(),
            denied: "Wrong password.".to_owned(),
            busy: "Checking the previous attempt.".to_owned(),
            failed: "The password could not be checked.".to_owned(),
            rate_limited: "Too many attempts. Try again in {seconds} s.".to_owned(),
        }
    }
}

impl LockStyle {
    /// The `lock_style` record of this style: the fields in the order of the catalog.
    pub(crate) fn to_val(&self) -> Val {
        record(vec![
            Val::Text(self.scrim_fill.clone()),
            Val::Double(self.card_width),
            Val::Double(self.card_height),
            Val::Double(self.card_radius),
            Val::Double(self.card_padding),
            Val::Double(self.card_gap),
            Val::Text(self.card_fill.clone()),
            Val::Double(self.card_top),
            Val::Double(self.clock_size),
            Val::Text(self.clock_color.clone()),
            Val::Double(self.entry_height),
            Val::Double(self.entry_radius),
            Val::Text(self.entry_fill.clone()),
            Val::Double(self.entry_size),
            Val::Text(self.entry_color.clone()),
            Val::Uint(self.entry_limit),
            Val::Double(self.message_size),
            Val::Text(self.hint_color.clone()),
            Val::Text(self.error_color.clone()),
            Val::Text(self.hint.clone()),
            Val::Text(self.checking.clone()),
            Val::Text(self.denied.clone()),
            Val::Text(self.busy.clone()),
            Val::Text(self.failed.clone()),
            Val::Text(self.rate_limited.clone()),
        ])
    }

    /// The style a `lock_style` value says; a field left out is the stock one.
    ///
    /// # Errors
    ///
    /// A field is not of its type, or a length is not a finite number of at least zero.
    pub(crate) fn of(value: &Val) -> Res<LockStyle> {
        let fields = value.record()?;
        let stock = LockStyle::default();
        let number = |at: usize, default: f64, what: &str| -> Res<f64> {
            let value = field(fields, at).map_or(Ok(default), Val::double)?;
            if value.is_finite() && value >= 0.0 {
                Ok(value)
            } else {
                Err(Fail::type_error(format!(
                    "a lock style's {what} is a finite number of at least 0"
                )))
            }
        };
        let text = |at: usize, default: &str| -> Res<String> {
            field(fields, at).map_or_else(|| Ok(default.to_owned()), |value| value.text().map(str::to_owned))
        };
        Ok(LockStyle {
            scrim_fill: text(0, &stock.scrim_fill)?,
            card_width: number(1, stock.card_width, "card width")?,
            card_height: number(2, stock.card_height, "card height")?,
            card_radius: number(3, stock.card_radius, "card radius")?,
            card_padding: number(4, stock.card_padding, "card padding")?,
            card_gap: number(5, stock.card_gap, "card gap")?,
            card_fill: text(6, &stock.card_fill)?,
            card_top: number(7, stock.card_top, "card top")?,
            clock_size: number(8, stock.clock_size, "clock size")?,
            clock_color: text(9, &stock.clock_color)?,
            entry_height: number(10, stock.entry_height, "entry height")?,
            entry_radius: number(11, stock.entry_radius, "entry radius")?,
            entry_fill: text(12, &stock.entry_fill)?,
            entry_size: number(13, stock.entry_size, "entry size")?,
            entry_color: text(14, &stock.entry_color)?,
            entry_limit: field(fields, 15).map_or(Ok(stock.entry_limit), Val::uint)?,
            message_size: number(16, stock.message_size, "message size")?,
            hint_color: text(17, &stock.hint_color)?,
            error_color: text(18, &stock.error_color)?,
            hint: text(19, &stock.hint)?,
            checking: text(20, &stock.checking)?,
            denied: text(21, &stock.denied)?,
            busy: text(22, &stock.busy)?,
            failed: text(23, &stock.failed)?,
            rate_limited: text(24, &stock.rate_limited)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_style_survives_its_record() {
        let stock = LockStyle::default();
        assert_eq!(LockStyle::of(&stock.to_val()).ok(), Some(stock));
    }

    #[test]
    fn a_field_left_out_is_the_stock_one() {
        let stock = LockStyle::default();
        let mut fields = vec![Val::None; 25];
        fields[1] = Val::Double(400.0);
        let style = LockStyle::of(&record(fields)).ok().unwrap_or_default();
        assert_eq!(style.card_width, 400.0);
        assert_eq!(style.card_height, stock.card_height);
        assert_eq!(style.rate_limited, stock.rate_limited);
    }
}
