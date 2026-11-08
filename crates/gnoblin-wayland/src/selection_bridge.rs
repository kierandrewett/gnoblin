//! The X11 <-> Wayland selection bridge: which side owns a selection target, and what
//! the other side is told when it changes.
//!
//! Smithay moves the bytes. This module decides who may be asked for them. Each target
//! (clipboard, primary) has one owner at a time:
//!
//! - [`SelectionHolder::WaylandOwned`]: a Wayland client set the selection. X clients see its mime types
//!   through the window manager, and an X read is answered by asking the Wayland source.
//! - [`SelectionHolder::X11Owned`]: an X client set the selection. The Wayland side holds a
//!   compositor-owned source, and a Wayland paste is answered by asking the X client.
//! - [`SelectionHolder::Unowned`]: nothing is bridged. Reads and pastes are refused.
//!
//! Setting a selection on one side never echoes back. Smithay reports a client-set
//! Wayland selection only for a client request, so the compositor's own source does not
//! re-enter [`SelectionOwners::wayland_changed`]; and the X11 window manager reports an
//! X client's selection only when the X client sets it.
//!
//! While the session is locked, `allowed` is false and the bridge takes no part in
//! either direction.
//!
//! # Failure modes first
//!
//! Each is a unit test below.
//!
//! 1. **A Wayland copy never reaches X11.** Setting the selection advertises its mime
//!    types to X11, and an X read is answered from the Wayland source.
//!    `a_wayland_copy_is_advertised_to_x11_and_read_from_its_source`.
//! 2. **An X11 copy never reaches Wayland.** Setting it offers the mapped mime types on
//!    the Wayland side, and a Wayland paste is answered by the X client.
//!    `an_x11_copy_is_offered_to_wayland_and_pasted_from_x11`.
//! 3. **Ownership moves without a stale owner.** Each change replaces the previous owner,
//!    and only the current owner's reads and pastes are served.
//!    `ownership_moves_back_and_forth`.
//! 4. **A cleared Wayland selection keeps its X11 offer.** A clear withdraws what was
//!    advertised; an empty list is a clear. `a_wayland_clear_withdraws_the_x11_offer`,
//!    `an_empty_wayland_selection_is_a_clear`.
//! 5. **An X11 clear takes a Wayland selection with it.** Only a selection the X11 side
//!    owns is cleared on the Wayland side. `an_x11_clear_clears_only_what_x11_owned`.
//! 6. **An X11 selection with no pasteable type leaves the old Wayland one in place.**
//!    The X11 selection replaced the Wayland one, so the Wayland one is cleared.
//!    `an_x11_selection_without_pasteable_types_clears_the_wayland_one`.
//! 7. **A locked session takes a copy from either side.**
//!    `a_locked_session_refuses_a_copy_from_either_side`,
//!    `a_lock_withdraws_an_x11_offer_it_refuses`.
//! 8. **A clipboard change moves the primary selection.** The two targets are tracked
//!    apart. `clipboard_and_primary_are_tracked_separately`.
//! 9. **A text alias is not understood.** `UTF8_STRING`, `TEXT` and `STRING` map to the
//!    Wayland text types, and X bookkeeping targets are dropped.
//!    `x11_targets_map_to_wayland_mime_types`.

use smithay::wayland::selection::SelectionTarget;

/// Who owns a selection target, as far as the bridge knows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SelectionHolder {
    /// Nothing is bridged: reads and pastes are refused.
    #[default]
    Unowned,
    /// A Wayland client set the selection.
    WaylandOwned,
    /// An X11 client set the selection.
    X11Owned,
}

/// What the X11 side must be told after a Wayland change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ToX11 {
    /// Advertise these mime types as the X11 selection.
    Advertise(Vec<String>),
    /// Withdraw the selection that was advertised.
    Withdraw,
    /// Nothing to tell.
    Nothing,
}

/// What the Wayland side must be told after an X11 change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ToWayland {
    /// Offer a compositor-owned source with these mime types.
    Offer(Vec<String>),
    /// Clear the Wayland selection, which an X11 change has replaced.
    Clear,
    /// Nothing to tell.
    Nothing,
}

/// Where a Wayland paste from the compositor's own source is answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PasteRoute {
    /// Ask the X11 client that owns the selection.
    X11,
    /// Refuse: the write end is dropped, so the paste reads nothing.
    Refuse,
}

/// The owner of each selection target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SelectionOwners {
    clipboard: SelectionHolder,
    primary: SelectionHolder,
}

impl SelectionOwners {
    /// The owner of a target.
    pub(crate) fn owner(&self, target: SelectionTarget) -> SelectionHolder {
        match target {
            SelectionTarget::Clipboard => self.clipboard,
            SelectionTarget::Primary => self.primary,
        }
    }

    fn slot(&mut self, target: SelectionTarget) -> &mut SelectionHolder {
        match target {
            SelectionTarget::Clipboard => &mut self.clipboard,
            SelectionTarget::Primary => &mut self.primary,
        }
    }

    /// A Wayland client set (`Some`) or cleared (`None`) the selection.
    ///
    /// Only a selection the Wayland side owned is withdrawn from X11. A clear while an
    /// X11 client owns the target leaves that owner in place: the compositor's own
    /// source may be replaced by a Wayland client, but the X11 selection still stands.
    pub(crate) fn wayland_changed(
        &mut self,
        target: SelectionTarget,
        mime_types: Option<&[String]>,
        allowed: bool,
    ) -> ToX11 {
        match mime_types.filter(|types| allowed && !types.is_empty()) {
            Some(types) => {
                *self.slot(target) = SelectionHolder::WaylandOwned;
                ToX11::Advertise(types.to_vec())
            }
            None => {
                let slot = self.slot(target);
                if *slot == SelectionHolder::WaylandOwned {
                    *slot = SelectionHolder::Unowned;
                    ToX11::Withdraw
                } else {
                    ToX11::Nothing
                }
            }
        }
    }

    /// An X11 client set the selection to these X11 targets.
    pub(crate) fn x11_changed(
        &mut self,
        target: SelectionTarget,
        targets: Option<&[String]>,
        allowed: bool,
    ) -> ToWayland {
        let previous = std::mem::take(self.slot(target));
        if !allowed {
            return if previous == SelectionHolder::X11Owned {
                ToWayland::Clear
            } else {
                ToWayland::Nothing
            };
        }
        let offered = targets.map(wayland_mime_types).unwrap_or_default();
        if offered.is_empty() {
            // The X11 selection replaced whatever the Wayland side held, even when it
            // has nothing to paste.
            return ToWayland::Clear;
        }
        *self.slot(target) = SelectionHolder::X11Owned;
        ToWayland::Offer(offered)
    }

    /// An X11 client gave up the selection.
    pub(crate) fn x11_cleared(&mut self, target: SelectionTarget) -> ToWayland {
        if self.owner(target) == SelectionHolder::X11Owned {
            *self.slot(target) = SelectionHolder::Unowned;
            ToWayland::Clear
        } else {
            ToWayland::Nothing
        }
    }

    /// Whether an X11 client may read the selection: only a Wayland-owned one, and only
    /// while the session is not locked.
    pub(crate) fn allow_x11_read(&self, target: SelectionTarget, allowed: bool) -> bool {
        allowed && self.owner(target) == SelectionHolder::WaylandOwned
    }

    /// Where a Wayland paste from the compositor's own source is answered.
    pub(crate) fn paste_route(&self, target: SelectionTarget, allowed: bool) -> PasteRoute {
        if allowed && self.owner(target) == SelectionHolder::X11Owned {
            PasteRoute::X11
        } else {
            PasteRoute::Refuse
        }
    }
}

/// The Wayland mime types for an X11 selection's targets.
///
/// Only names that are mime types (contain a `/`) and the text aliases are kept, in
/// their first order and without duplicates. X bookkeeping targets such as `TARGETS`,
/// `TIMESTAMP` and `MULTIPLE` are dropped. `STRING` is mapped to `text/plain`, which
/// loses its Latin-1 meaning; that is the recorded limit of this bridge.
pub(crate) fn wayland_mime_types(targets: &[String]) -> Vec<String> {
    let mut mime_types: Vec<String> = Vec::new();
    for target in targets {
        let mapped = match target.as_str() {
            "UTF8_STRING" => "text/plain;charset=utf-8",
            "TEXT" | "STRING" => "text/plain",
            name if name.contains('/') => name,
            _ => continue,
        };
        if !mime_types.iter().any(|known| known == mapped) {
            mime_types.push(mapped.to_owned());
        }
    }
    mime_types
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIPBOARD: SelectionTarget = SelectionTarget::Clipboard;
    const PRIMARY: SelectionTarget = SelectionTarget::Primary;

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_wayland_copy_is_advertised_to_x11_and_read_from_its_source() {
        let mut owners = SelectionOwners::default();
        let types = names(&["text/plain;charset=utf-8"]);

        assert_eq!(
            owners.wayland_changed(CLIPBOARD, Some(&types), true),
            ToX11::Advertise(types.clone())
        );
        assert_eq!(owners.owner(CLIPBOARD), SelectionHolder::WaylandOwned);
        assert!(owners.allow_x11_read(CLIPBOARD, true));
        // A Wayland paste of a Wayland copy is the protocol's own path, not the bridge's.
        assert_eq!(owners.paste_route(CLIPBOARD, true), PasteRoute::Refuse);
    }

    #[test]
    fn an_x11_copy_is_offered_to_wayland_and_pasted_from_x11() {
        let mut owners = SelectionOwners::default();
        let targets = names(&["UTF8_STRING", "TARGETS", "TIMESTAMP"]);

        assert_eq!(
            owners.x11_changed(CLIPBOARD, Some(&targets), true),
            ToWayland::Offer(names(&["text/plain;charset=utf-8"]))
        );
        assert_eq!(owners.owner(CLIPBOARD), SelectionHolder::X11Owned);
        assert_eq!(owners.paste_route(CLIPBOARD, true), PasteRoute::X11);
        assert!(!owners.allow_x11_read(CLIPBOARD, true));
    }

    #[test]
    fn ownership_moves_back_and_forth() {
        let mut owners = SelectionOwners::default();
        let wayland = names(&["text/plain"]);
        let x11 = names(&["UTF8_STRING"]);

        owners.wayland_changed(CLIPBOARD, Some(&wayland), true);
        assert_eq!(owners.owner(CLIPBOARD), SelectionHolder::WaylandOwned);

        assert_eq!(
            owners.x11_changed(CLIPBOARD, Some(&x11), true),
            ToWayland::Offer(names(&["text/plain;charset=utf-8"]))
        );
        assert_eq!(owners.owner(CLIPBOARD), SelectionHolder::X11Owned);
        assert!(!owners.allow_x11_read(CLIPBOARD, true));

        assert_eq!(
            owners.wayland_changed(CLIPBOARD, Some(&wayland), true),
            ToX11::Advertise(wayland.clone())
        );
        assert_eq!(owners.owner(CLIPBOARD), SelectionHolder::WaylandOwned);
        assert_eq!(owners.paste_route(CLIPBOARD, true), PasteRoute::Refuse);
    }

    #[test]
    fn a_wayland_clear_withdraws_the_x11_offer() {
        let mut owners = SelectionOwners::default();
        let types = names(&["text/plain"]);
        owners.wayland_changed(CLIPBOARD, Some(&types), true);

        assert_eq!(owners.wayland_changed(CLIPBOARD, None, true), ToX11::Withdraw);
        assert_eq!(owners.owner(CLIPBOARD), SelectionHolder::Unowned);
        assert!(!owners.allow_x11_read(CLIPBOARD, true));
    }

    #[test]
    fn an_empty_wayland_selection_is_a_clear() {
        let mut owners = SelectionOwners::default();
        let types = names(&["text/plain"]);
        owners.wayland_changed(CLIPBOARD, Some(&types), true);

        assert_eq!(owners.wayland_changed(CLIPBOARD, Some(&[]), true), ToX11::Withdraw);
        assert_eq!(owners.owner(CLIPBOARD), SelectionHolder::Unowned);
    }

    #[test]
    fn a_wayland_clear_keeps_an_x11_owner() {
        let mut owners = SelectionOwners::default();
        let x11 = names(&["UTF8_STRING"]);
        owners.x11_changed(CLIPBOARD, Some(&x11), true);

        // The compositor's source can be replaced by a Wayland client's clear; the X11
        // owner stands, so nothing is withdrawn from X11 and its read stays X11's.
        assert_eq!(owners.wayland_changed(CLIPBOARD, None, true), ToX11::Nothing);
        assert_eq!(owners.owner(CLIPBOARD), SelectionHolder::X11Owned);
        assert!(!owners.allow_x11_read(CLIPBOARD, true));
    }

    #[test]
    fn an_x11_clear_clears_only_what_x11_owned() {
        let mut owners = SelectionOwners::default();
        let wayland = names(&["text/plain"]);
        let x11 = names(&["UTF8_STRING"]);

        // A Wayland selection survives an X11 clear: the X11 side never owned it.
        owners.wayland_changed(CLIPBOARD, Some(&wayland), true);
        assert_eq!(owners.x11_cleared(CLIPBOARD), ToWayland::Nothing);
        assert_eq!(owners.owner(CLIPBOARD), SelectionHolder::WaylandOwned);

        // An X11 selection is cleared once, when its owner gives it up.
        owners.x11_changed(CLIPBOARD, Some(&x11), true);
        assert_eq!(owners.x11_cleared(CLIPBOARD), ToWayland::Clear);
        assert_eq!(owners.owner(CLIPBOARD), SelectionHolder::Unowned);
        assert_eq!(owners.x11_cleared(CLIPBOARD), ToWayland::Nothing);
    }

    #[test]
    fn an_x11_selection_without_pasteable_types_clears_the_wayland_one() {
        let mut owners = SelectionOwners::default();
        let wayland = names(&["text/plain"]);
        owners.wayland_changed(CLIPBOARD, Some(&wayland), true);

        let bookkeeping = names(&["TARGETS", "TIMESTAMP", "MULTIPLE"]);
        assert_eq!(
            owners.x11_changed(CLIPBOARD, Some(&bookkeeping), true),
            ToWayland::Clear
        );
        assert_eq!(owners.owner(CLIPBOARD), SelectionHolder::Unowned);
        assert_eq!(owners.paste_route(CLIPBOARD, true), PasteRoute::Refuse);
    }

    #[test]
    fn a_locked_session_refuses_a_copy_from_either_side() {
        let mut owners = SelectionOwners::default();
        let types = names(&["text/plain"]);
        let x11 = names(&["UTF8_STRING"]);

        assert_eq!(owners.wayland_changed(CLIPBOARD, Some(&types), false), ToX11::Nothing);
        assert_eq!(owners.owner(CLIPBOARD), SelectionHolder::Unowned);
        assert!(!owners.allow_x11_read(CLIPBOARD, false));

        assert_eq!(owners.x11_changed(CLIPBOARD, Some(&x11), false), ToWayland::Nothing);
        assert_eq!(owners.owner(CLIPBOARD), SelectionHolder::Unowned);
        assert_eq!(owners.paste_route(CLIPBOARD, false), PasteRoute::Refuse);
    }

    #[test]
    fn a_lock_withdraws_an_x11_offer_it_refuses() {
        let mut owners = SelectionOwners::default();
        let x11 = names(&["UTF8_STRING"]);
        owners.x11_changed(CLIPBOARD, Some(&x11), true);

        // A new X11 copy while locked clears the offer it replaced, and is not offered.
        assert_eq!(owners.x11_changed(CLIPBOARD, Some(&x11), false), ToWayland::Clear);
        assert_eq!(owners.owner(CLIPBOARD), SelectionHolder::Unowned);
        assert_eq!(owners.paste_route(CLIPBOARD, false), PasteRoute::Refuse);
        assert_eq!(owners.paste_route(CLIPBOARD, true), PasteRoute::Refuse);
    }

    #[test]
    fn clipboard_and_primary_are_tracked_separately() {
        let mut owners = SelectionOwners::default();
        let wayland = names(&["text/plain"]);
        let x11 = names(&["UTF8_STRING"]);

        owners.wayland_changed(CLIPBOARD, Some(&wayland), true);
        owners.x11_changed(PRIMARY, Some(&x11), true);

        assert_eq!(owners.owner(CLIPBOARD), SelectionHolder::WaylandOwned);
        assert_eq!(owners.owner(PRIMARY), SelectionHolder::X11Owned);
        assert_eq!(owners.paste_route(PRIMARY, true), PasteRoute::X11);
        assert_eq!(owners.paste_route(CLIPBOARD, true), PasteRoute::Refuse);
    }

    #[test]
    fn x11_targets_map_to_wayland_mime_types() {
        assert_eq!(
            wayland_mime_types(&names(&[
                "UTF8_STRING",
                "TEXT",
                "STRING",
                "TARGETS",
                "TIMESTAMP",
                "MULTIPLE",
                "image/png",
                "text/plain",
                "text/plain;charset=utf-8",
            ])),
            names(&["text/plain;charset=utf-8", "text/plain", "image/png"])
        );
        assert!(wayland_mime_types(&names(&["TARGETS"])).is_empty());
    }
}
