//! What the host's `zwp_linux_dmabuf_v1` says it can import: the device its renderer is on and the format and modifier
//! pairs it accepts.
//!
//! A host of version 4 sends feedback: the main device, a table of every pair it knows (a file of sixteen-byte entries)
//! and tranches that name which entries are usable for which purpose. A host of version 3 or older sends a `modifier`
//! event per pair, and an older one a `format` event with no modifier. The three ways of saying it end in one list, which
//! [`HostDmabuf::formats`] returns. It is plain data fed by the events, so each awkward case is a unit test.
//!
//! # Failure modes first
//!
//! 1. **A tranche for another device is taken as the host's.** A tranche whose target device is not the main device
//!    (a scan-out tranche on a display GPU) names buffers the main device may not import, so it is ignored:
//!    `a_tranche_for_another_device_is_not_the_hosts_list`.
//! 2. **An index past the table is followed.** A tranche index the table does not have is dropped, not a panic:
//!    `an_index_past_the_table_is_dropped`.
//! 3. **A format the session cannot name is offered.** A code with no [`Fourcc`] is dropped:
//!    `a_format_with_no_name_is_dropped`.
//! 4. **A table cut short is read as whole entries.** A trailing partial entry is ignored:
//!    `a_table_cut_short_keeps_its_whole_entries`.

use smithay::backend::allocator::{Format, Fourcc, Modifier};

/// The size of one entry of the feedback's format table: a `u32` format, four bytes of padding and a `u64` modifier.
const ENTRY: usize = 16;

/// What the host's dmabuf global offers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct HostDmabuf {
    /// The `dev_t` of the host's main device, when it sent feedback.
    main_device: Option<u64>,
    /// The pairs of the feedback table, in the host's order.
    table: Vec<Format>,
    /// The target device of the tranche being read.
    tranche_device: Option<u64>,
    /// The pairs of the tranche being read.
    tranche: Vec<Format>,
    /// The pairs of every finished tranche for the main device, in the order they were sent, without repeats.
    tranched: Vec<Format>,
    /// The pairs of the older events, which carry no device.
    legacy: Vec<Format>,
}

/// A `dev_t` as the wire sends it: an array of the bytes of a native integer.
fn device_of(bytes: &[u8]) -> Option<u64> {
    match bytes.len() {
        8 => Some(u64::from_ne_bytes(bytes.try_into().ok()?)),
        4 => Some(u64::from(u32::from_ne_bytes(bytes.try_into().ok()?))),
        _ => None,
    }
}

fn push_new(list: &mut Vec<Format>, format: Format) {
    if !list.contains(&format) {
        list.push(format);
    }
}

impl HostDmabuf {
    /// The host's main device.
    pub fn main_device(&mut self, bytes: &[u8]) {
        self.main_device = device_of(bytes);
    }

    /// The device the host's feedback names, if it sent any.
    pub fn device(&self) -> Option<u64> {
        self.main_device
    }

    /// The feedback's format table, as the contents of the file the host sent.
    pub fn format_table(&mut self, bytes: &[u8]) {
        self.table = bytes
            .as_chunks::<ENTRY>()
            .0
            .iter()
            .filter_map(|entry| {
                let code = u32::from_ne_bytes(entry[0..4].try_into().ok()?);
                let modifier = u64::from_ne_bytes(entry[8..16].try_into().ok()?);
                Some(Format {
                    code: Fourcc::try_from(code).ok()?,
                    modifier: Modifier::from(modifier),
                })
            })
            .collect();
    }

    /// A new tranche starts, for the device `bytes` names.
    pub fn tranche_target(&mut self, bytes: &[u8]) {
        self.tranche_device = device_of(bytes);
        self.tranche.clear();
    }

    /// The tranche's entries, as indices into the table: an array of native `u16`s.
    pub fn tranche_formats(&mut self, indices: &[u8]) {
        for index in indices.as_chunks::<2>().0 {
            let index = usize::from(u16::from_ne_bytes(*index));
            if let Some(format) = self.table.get(index) {
                push_new(&mut self.tranche, *format);
            }
        }
    }

    /// The tranche is complete: its pairs join the list when it is for the main device.
    pub fn tranche_done(&mut self) {
        let for_main = match (self.tranche_device, self.main_device) {
            (Some(target), Some(main)) => target == main,
            // A host that names no device in a tranche means the main one.
            (None, _) | (_, None) => true,
        };
        if for_main {
            for format in std::mem::take(&mut self.tranche) {
                push_new(&mut self.tranched, format);
            }
        } else {
            self.tranche.clear();
        }
    }

    /// An older host's `modifier` event: a pair, with its modifier as the wire splits it.
    pub fn legacy_modifier(&mut self, code: u32, high: u32, low: u32) {
        if let Ok(code) = Fourcc::try_from(code) {
            let modifier = Modifier::from(u64::from(high) << 32 | u64::from(low));
            push_new(&mut self.legacy, Format { code, modifier });
        }
    }

    /// An older host's `format` event, which states no modifier: the buffer's layout is the driver's to choose.
    pub fn legacy_format(&mut self, code: u32) {
        if let Ok(code) = Fourcc::try_from(code) {
            push_new(
                &mut self.legacy,
                Format {
                    code,
                    modifier: Modifier::Invalid,
                },
            );
        }
    }

    /// Every pair the host accepts: the feedback's when it sent some, the older events' otherwise.
    pub fn formats(&self) -> Vec<Format> {
        if self.tranched.is_empty() {
            self.legacy.clone()
        } else {
            self.tranched.clone()
        }
    }

    /// Whether the host said anything about formats at all.
    pub fn offers_formats(&self) -> bool {
        !self.tranched.is_empty() || !self.legacy.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(code: Fourcc, modifier: Modifier) -> Vec<u8> {
        let mut bytes = (code as u32).to_ne_bytes().to_vec();
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&u64::from(modifier).to_ne_bytes());
        bytes
    }

    fn indices(of: &[u16]) -> Vec<u8> {
        of.iter().flat_map(|index| index.to_ne_bytes()).collect()
    }

    fn table(entries: &[(Fourcc, Modifier)]) -> Vec<u8> {
        entries
            .iter()
            .flat_map(|(code, modifier)| entry(*code, *modifier))
            .collect()
    }

    #[test]
    fn a_tranche_lists_the_table_entries_it_names() {
        let mut host = HostDmabuf::default();
        host.main_device(&0x8001u64.to_ne_bytes());
        host.format_table(&table(&[
            (Fourcc::Argb8888, Modifier::Linear),
            (Fourcc::Xrgb8888, Modifier::Linear),
            (Fourcc::Argb8888, Modifier::Invalid),
        ]));
        host.tranche_target(&0x8001u64.to_ne_bytes());
        host.tranche_formats(&indices(&[2, 0]));
        host.tranche_done();
        assert_eq!(host.device(), Some(0x8001));
        assert_eq!(
            host.formats(),
            vec![
                Format {
                    code: Fourcc::Argb8888,
                    modifier: Modifier::Invalid
                },
                Format {
                    code: Fourcc::Argb8888,
                    modifier: Modifier::Linear
                },
            ],
            "the tranche's order is the host's preference"
        );
    }

    #[test]
    fn a_tranche_for_another_device_is_not_the_hosts_list() {
        let mut host = HostDmabuf::default();
        host.main_device(&1u64.to_ne_bytes());
        host.format_table(&table(&[
            (Fourcc::Argb8888, Modifier::Linear),
            (Fourcc::Xrgb8888, Modifier::Linear),
        ]));
        host.tranche_target(&2u64.to_ne_bytes());
        host.tranche_formats(&indices(&[0]));
        host.tranche_done();
        host.tranche_target(&1u64.to_ne_bytes());
        host.tranche_formats(&indices(&[1]));
        host.tranche_done();
        assert_eq!(
            host.formats(),
            vec![Format {
                code: Fourcc::Xrgb8888,
                modifier: Modifier::Linear
            }]
        );
    }

    #[test]
    fn an_index_past_the_table_is_dropped() {
        let mut host = HostDmabuf::default();
        host.format_table(&table(&[(Fourcc::Argb8888, Modifier::Linear)]));
        host.tranche_target(&[]);
        host.tranche_formats(&indices(&[0, 9, 65535]));
        host.tranche_done();
        assert_eq!(host.formats().len(), 1);
    }

    #[test]
    fn a_format_with_no_name_is_dropped() {
        let mut host = HostDmabuf::default();
        let mut bytes = table(&[(Fourcc::Argb8888, Modifier::Linear)]);
        bytes.extend_from_slice(&0xdead_beefu32.to_ne_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&0u64.to_ne_bytes());
        host.format_table(&bytes);
        host.tranche_target(&[]);
        host.tranche_formats(&indices(&[0, 1]));
        host.tranche_done();
        assert_eq!(host.formats().len(), 1);
    }

    #[test]
    fn a_table_cut_short_keeps_its_whole_entries() {
        let mut host = HostDmabuf::default();
        let mut bytes = table(&[(Fourcc::Argb8888, Modifier::Linear)]);
        bytes.extend_from_slice(&[1, 2, 3]);
        host.format_table(&bytes);
        host.tranche_target(&[]);
        host.tranche_formats(&indices(&[0, 1]));
        host.tranche_done();
        assert_eq!(host.formats().len(), 1);
    }

    #[test]
    fn an_older_host_states_pairs_one_at_a_time() {
        let mut host = HostDmabuf::default();
        host.legacy_modifier(Fourcc::Argb8888 as u32, 0, 0);
        host.legacy_modifier(Fourcc::Argb8888 as u32, 0, 0);
        host.legacy_format(Fourcc::Xrgb8888 as u32);
        assert_eq!(
            host.formats(),
            vec![
                Format {
                    code: Fourcc::Argb8888,
                    modifier: Modifier::Linear
                },
                Format {
                    code: Fourcc::Xrgb8888,
                    modifier: Modifier::Invalid
                },
            ],
            "a repeated pair is listed once, and a format with no modifier has the driver's choice"
        );
        assert_eq!(host.device(), None, "a host of this age names no device");
    }

    #[test]
    fn feedback_wins_over_the_older_events() {
        let mut host = HostDmabuf::default();
        host.legacy_format(Fourcc::Xrgb8888 as u32);
        host.format_table(&table(&[(Fourcc::Argb8888, Modifier::Linear)]));
        host.tranche_target(&[]);
        host.tranche_formats(&indices(&[0]));
        host.tranche_done();
        assert_eq!(host.formats().len(), 1);
        assert_eq!(host.formats()[0].code, Fourcc::Argb8888);
    }
}
