//! Still pictures: one frame of an output, encoded as PNG for a caller that asked for
//! a picture rather than a stream.
//!
//! A still is a session of one frame. The same embargo that stops a stream applies to
//! it: while the session lock is up, or the lock says capture is not allowed, no
//! pixel leaves the compositor. The choice of output and the encoding are pure, so
//! the refusals can be tested without a session, and the compositor's handler does
//! nothing but ask the lock, read the frame and send what this module returns.
//!
//! # Failure modes
//!
//! - a still taken while the lock is up, or while capture is embargoed, is refused
//!   with [`Refusal::Locked`] and no pixel is read;
//! - a session with no output has nothing to show: [`Refusal::NoOutput`];
//! - a frame whose rows do not fit its declared stride is refused rather than read
//!   past its end: [`Refusal::Encode`];
//! - a picture larger than the frame the wire can carry is refused rather than
//!   truncated: [`Refusal::TooLarge`].

use gnoblin_types::{OutputId, Size};

/// Why a still picture was not produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The session is locked, or the lock embargoes capture.
    Locked,
    /// There is no output to take a picture of.
    NoOutput,
    /// The output has no frame to read back.
    Unavailable,
    /// The encoded picture is larger than the limit it was given.
    TooLarge,
    /// The frame does not describe valid RGBA8 pixels, or the encoder failed.
    Encode,
}

/// Chooses the output a still is taken of: the first one, when capture is allowed.
///
/// The lock is asked first. A locked session refuses even when the lock's embargo
/// allows capture, because the lock scene is not the desktop the caller asked about.
///
/// # Errors
///
/// [`Refusal::Locked`] while locked or embargoed, [`Refusal::NoOutput`] when `outputs`
/// is empty.
pub fn choose_output(locked: bool, capture_allowed: bool, outputs: &[OutputId]) -> Result<OutputId, Refusal> {
    if locked || !capture_allowed {
        return Err(Refusal::Locked);
    }
    outputs.first().copied().ok_or(Refusal::NoOutput)
}

/// Encodes one RGBA8 frame as PNG, no larger than `max_len` bytes.
///
/// `stride` is the byte distance between rows, which may be larger than `width * 4`;
/// the padding is dropped rather than encoded.
///
/// # Errors
///
/// [`Refusal::Encode`] when the frame does not hold `size` pixels at `stride`, or the
/// encoder fails; [`Refusal::TooLarge`] when the picture is over `max_len`.
pub fn encode_png(size: Size, stride: usize, data: &[u8], max_len: usize) -> Result<Vec<u8>, Refusal> {
    let row = (size.width as usize).checked_mul(4).ok_or(Refusal::Encode)?;
    let height = size.height as usize;
    if size.width == 0 || size.height == 0 || stride < row {
        return Err(Refusal::Encode);
    }
    let needed = stride
        .checked_mul(height - 1)
        .and_then(|rows| rows.checked_add(row))
        .ok_or(Refusal::Encode)?;
    if data.len() < needed {
        return Err(Refusal::Encode);
    }
    let mut packed = Vec::with_capacity(row * height);
    for line in 0..height {
        let start = line * stride;
        packed.extend_from_slice(&data[start..start + row]);
    }
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, size.width, size.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|_| Refusal::Encode)?;
        writer.write_image_data(&packed).map_err(|_| Refusal::Encode)?;
        writer.finish().map_err(|_| Refusal::Encode)?;
    }
    if encoded.len() > max_len {
        return Err(Refusal::TooLarge);
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(raw: u64) -> OutputId {
        OutputId::new(raw)
    }

    fn decode(bytes: &[u8]) -> (png::OutputInfo, Vec<u8>) {
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().expect("a PNG header");
        let mut buffer = vec![0; reader.output_buffer_size().expect("a size")];
        let info = reader.next_frame(&mut buffer).expect("a PNG frame");
        buffer.truncate(info.buffer_size());
        (info, buffer)
    }

    #[test]
    fn a_locked_session_refuses_even_when_the_embargo_allows_capture() {
        assert_eq!(choose_output(true, true, &[output(1)]), Err(Refusal::Locked));
    }

    #[test]
    fn an_embargoed_session_refuses_even_when_unlocked() {
        assert_eq!(choose_output(false, false, &[output(1)]), Err(Refusal::Locked));
    }

    #[test]
    fn no_output_is_refused_rather_than_guessed() {
        assert_eq!(choose_output(false, true, &[]), Err(Refusal::NoOutput));
    }

    #[test]
    fn the_first_output_is_chosen_when_capture_is_allowed() {
        assert_eq!(choose_output(false, true, &[output(7), output(9)]), Ok(output(7)));
    }

    #[test]
    fn a_frame_with_too_short_rows_is_refused_not_read_past_its_end() {
        let data = vec![0; 15];
        assert_eq!(
            encode_png(Size::new(2, 2), 8, &data, usize::MAX),
            Err(Refusal::Encode),
            "two rows of 8 bytes need 16 bytes; a 15-byte frame must not be read"
        );
    }

    #[test]
    fn a_stride_narrower_than_a_row_is_refused() {
        assert_eq!(
            encode_png(Size::new(2, 1), 4, &[0; 8], usize::MAX),
            Err(Refusal::Encode)
        );
    }

    #[test]
    fn a_picture_over_the_limit_is_refused_rather_than_truncated() {
        let data = vec![0x80; 64 * 64 * 4];
        assert_eq!(encode_png(Size::new(64, 64), 64 * 4, &data, 16), Err(Refusal::TooLarge));
    }

    #[test]
    fn padding_is_dropped_and_the_colour_and_size_survive_the_round_trip() {
        // Two pixels per row, four padding bytes per row: the picture must not carry them.
        let mut data = Vec::new();
        for _ in 0..2 {
            data.extend_from_slice(&[10, 20, 30, 255, 40, 50, 60, 255]);
            data.extend_from_slice(&[0xee; 4]);
        }
        let bytes = encode_png(Size::new(2, 2), 12, &data, usize::MAX).expect("a PNG");
        let (info, pixels) = decode(&bytes);
        assert_eq!((info.width, info.height), (2, 2));
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(pixels.len(), 16);
        assert_eq!(&pixels[0..4], &[10, 20, 30, 255]);
        assert_eq!(&pixels[4..8], &[40, 50, 60, 255]);
        assert_eq!(&pixels[8..12], &[10, 20, 30, 255]);
    }
}
