//! Timestamps for event lines. Output is UTC so it is the same on every machine
//! and easy to line up with compositor logs.

use std::time::{SystemTime, UNIX_EPOCH};

/// Formats milliseconds since the Unix epoch as `HH:MM:SS.mmm` in UTC.
pub fn clock_utc(epoch_millis: u128) -> String {
    let millis = epoch_millis % 86_400_000;
    let hours = millis / 3_600_000;
    let minutes = (millis / 60_000) % 60;
    let seconds = (millis / 1_000) % 60;
    let rest = millis % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{rest:03}")
}

/// The current UTC time as `HH:MM:SS.mmm`.
pub fn now_utc() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    clock_utc(millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_time_of_day() {
        assert_eq!(clock_utc(0), "00:00:00.000");
        assert_eq!(clock_utc(3_723_004), "01:02:03.004");
        // One day later is the same time of day.
        assert_eq!(clock_utc(86_400_000 + 1_500), "00:00:01.500");
    }
}
