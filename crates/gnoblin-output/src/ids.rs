//! Output identity.

use std::fmt;

/// A stable identity for one rendering target.
///
/// The connector name (`"DP-1"`, `"eDP-1"`) is what the public output record
/// carries as `id` and what window `monitor_id` and `window:move_to_monitor()`
/// name, so it is the identity everywhere. A second numbering scheme would need
/// a mapping back to the connector and could drift from it after a hotplug, so
/// none exists.
///
/// Ordering is the byte order of the connector name. That is what makes an
/// iteration order exist at all: a logical monitor takes the lowest connector
/// name it contains, and layouts are emitted in a fixed order.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct OutputId(String);

impl OutputId {
    /// The identity of the output reachable through `connector`.
    #[must_use]
    pub fn new(connector: impl Into<String>) -> Self {
        Self(connector.into())
    }

    /// The connector name this identity stands for.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OutputId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connector_order_is_the_order_the_public_reference_documents() {
        let mut ids = vec![OutputId::new("HDMI-1"), OutputId::new("DP-2"), OutputId::new("DP-1")];
        ids.sort();
        assert_eq!(
            ids,
            vec![OutputId::new("DP-1"), OutputId::new("DP-2"), OutputId::new("HDMI-1")]
        );
    }
}
