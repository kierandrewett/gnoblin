//! The EDID base block: identity, physical size and the preferred mode.

use crate::mode::Mode;
use crate::physical::PhysicalSize;

/// The length of an EDID base block.
const BASE_BLOCK: usize = 128;

/// The eight-byte header every base block starts with.
const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

/// The first byte of the four 18-byte descriptors.
const DESCRIPTORS: usize = 54;

/// The length of one descriptor.
const DESCRIPTOR_LENGTH: usize = 18;

/// Why a block could not be read.
///
/// The parser refuses a block rather than salvaging part of it: identity taken
/// from a block whose checksum does not add up is a vendor name for the wrong
/// monitor, which is worse than no name at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EdidError {
    /// The bytes are shorter than a base block.
    TooShort {
        /// How many bytes were supplied.
        length: usize,
    },
    /// The eight-byte header is not an EDID block header.
    BadHeader,
    /// The bytes do not sum to zero modulo 256, so the block is corrupt.
    Checksum,
}

/// What a monitor reported about itself in its EDID base block.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Edid {
    /// The manufacturer's three-letter code, when the block encodes one.
    pub manufacturer: Option<String>,
    /// The manufacturer's product code.
    pub product_code: u16,
    /// The numeric serial number, when the block carries a non-zero one.
    pub serial: Option<u32>,
    /// The panel size, zero on an axis the block does not report.
    pub physical: PhysicalSize,
    /// The monitor name descriptor, when the block carries one.
    pub name: Option<String>,
    /// The first detailed timing, which is the monitor's preferred mode.
    pub preferred_mode: Option<Mode>,
}

impl Edid {
    /// Reads the base block.
    ///
    /// Every field is optional because monitors leave parts of the block empty,
    /// and a caller that needs one of them has to say what to do without it. The
    /// only hard requirements are the header and the checksum, which is how a
    /// truncated read or a block from another device is caught before it names the
    /// wrong monitor.
    pub fn parse(bytes: &[u8]) -> Result<Self, EdidError> {
        if bytes.len() < BASE_BLOCK {
            return Err(EdidError::TooShort { length: bytes.len() });
        }
        let block = &bytes[..BASE_BLOCK];
        if block[..HEADER.len()] != HEADER {
            return Err(EdidError::BadHeader);
        }
        if block.iter().fold(0_u8, |sum, byte| sum.wrapping_add(*byte)) != 0 {
            return Err(EdidError::Checksum);
        }

        let manufacturer_code = u16::from_be_bytes([block[8], block[9]]);
        let product_code = u16::from_le_bytes([block[10], block[11]]);
        let serial_raw = u32::from_le_bytes([block[12], block[13], block[14], block[15]]);
        let physical = PhysicalSize::new(u32::from(block[21]) * 10, u32::from(block[22]) * 10);

        let mut name = None;
        let mut preferred_mode = None;
        for index in 0..4 {
            let start = DESCRIPTORS + index * DESCRIPTOR_LENGTH;
            let descriptor = &block[start..start + DESCRIPTOR_LENGTH];
            if is_display_descriptor(descriptor) {
                if descriptor[3] == 0xFC && name.is_none() {
                    name = monitor_name(&descriptor[5..]);
                }
            } else if preferred_mode.is_none() {
                preferred_mode = detailed_timing(descriptor);
            }
        }

        Ok(Self {
            manufacturer: manufacturer_name(manufacturer_code),
            product_code,
            serial: (serial_raw != 0).then_some(serial_raw),
            physical,
            name,
            preferred_mode,
        })
    }
}

/// Whether an 18-byte descriptor is a display descriptor rather than a timing.
///
/// The encoding is a zero pixel clock followed by a non-zero tag in byte 3: a zero
/// pixel clock with a zero tag is an unused slot, which has nothing to read.
fn is_display_descriptor(descriptor: &[u8]) -> bool {
    descriptor[0] == 0 && descriptor[1] == 0 && descriptor[3] != 0
}

/// Decodes the three five-bit letters of the manufacturer id.
///
/// A code with a zero or out-of-range letter is a monitor that did not set one,
/// so it yields no name rather than replacement characters.
fn manufacturer_name(code: u16) -> Option<String> {
    let mut name = String::with_capacity(3);
    for shift in [10, 5, 0] {
        let letter = ((code >> shift) & 0x1F) as u8;
        if !(1..=26).contains(&letter) {
            return None;
        }
        name.push(char::from(b'A' + letter - 1));
    }
    Some(name)
}

/// Reads an ASCII monitor name descriptor, which is space padded.
fn monitor_name(bytes: &[u8]) -> Option<String> {
    let mut name = String::new();
    for byte in bytes {
        if *byte == 0x00 || *byte == 0x0A {
            break;
        }
        if !(0x20..=0x7E).contains(byte) {
            return None;
        }
        name.push(char::from(*byte));
    }
    let trimmed = name.trim_end();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Reads a detailed timing descriptor as a mode.
///
/// The refresh rate comes from the pixel clock and the total blanking, both of
/// which the descriptor carries, so 59.94 Hz stays 59.94 Hz instead of being
/// rounded to 60 and selecting a different mode than the monitor prefers.
fn detailed_timing(descriptor: &[u8]) -> Option<Mode> {
    let pixel_clock_10khz = u32::from(u16::from_le_bytes([descriptor[0], descriptor[1]]));
    if pixel_clock_10khz == 0 {
        return None;
    }
    let horizontal_active = u32::from(descriptor[2]) | (u32::from(descriptor[4] & 0xF0) << 4);
    let horizontal_blank = u32::from(descriptor[3]) | (u32::from(descriptor[4] & 0x0F) << 8);
    let vertical_active = u32::from(descriptor[5]) | (u32::from(descriptor[7] & 0xF0) << 4);
    let vertical_blank = u32::from(descriptor[6]) | (u32::from(descriptor[7] & 0x0F) << 8);
    let horizontal_total = u64::from(horizontal_active) + u64::from(horizontal_blank);
    let vertical_total = u64::from(vertical_active) + u64::from(vertical_blank);
    if horizontal_active == 0 || vertical_active == 0 || horizontal_total == 0 || vertical_total == 0 {
        return None;
    }
    // Pixel clock is in 10 kHz units, and millihertz is a thousandth of a hertz.
    let numerator = u64::from(pixel_clock_10khz) * 10_000 * 1_000;
    let refresh_mhz = numerator / (horizontal_total * vertical_total);
    Some(Mode::new(
        horizontal_active,
        vertical_active,
        u32::try_from(refresh_mhz).unwrap_or(u32::MAX),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manufacturer_code_decodes_to_three_letters() {
        let code = (u16::from(b'D' - 64) << 10) | (u16::from(b'E' - 64) << 5) | u16::from(b'L' - 64);
        assert_eq!(manufacturer_name(code).as_deref(), Some("DEL"));
        assert_eq!(manufacturer_name(0), None);
        assert_eq!(manufacturer_name(0x7FFF), None);
    }

    #[test]
    fn a_preferred_timing_becomes_a_mode_at_its_real_refresh_rate() {
        // 1920x1080 at 148.5 MHz over 2200x1125 totals is exactly 60 Hz.
        let descriptor = [
            0x02, 0x3A, 0x80, 0x18, 0x71, 0x38, 0x2D, 0x40, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(detailed_timing(&descriptor), Some(Mode::new(1920, 1080, 60_000)));
        assert_eq!(detailed_timing(&[0; 18]), None);
    }

    #[test]
    fn a_name_descriptor_is_read_to_its_first_padding_byte() {
        let mut bytes = [0x20_u8; 13];
        bytes[..3].copy_from_slice(b"ABC");
        bytes[12] = 0x00;
        assert_eq!(monitor_name(&bytes).as_deref(), Some("ABC"));
        assert_eq!(monitor_name(&[0x0A; 13]), None);
    }
}
