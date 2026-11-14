//! Persistent profiles: reading a change request, laying a profile over the default row, and what a profile does
//! while its monitor is not connected.

use gnoblin_output::{
    DisplayMode, LayoutError, OutputDescriptor, OutputProfile, OutputSighting, ProfileChange, ProfileError,
    ProfileStore, compute,
};
use gnoblin_types::{Orientation, OutputId, Rect, Scale};

fn descriptor(id: u64, connector: &str, mode: DisplayMode) -> OutputDescriptor {
    let mut descriptor = OutputDescriptor::new(OutputId::new(id), connector, vec![mode]);
    descriptor.scales = vec![Scale::integral(1), Scale::from_percent(150)];
    descriptor
}

const FHD: DisplayMode = DisplayMode::new(1920, 1080, 60_000);
const QHD: DisplayMode = DisplayMode::new(2560, 1440, 60_000);

fn sighting<'a>(connector: &'a str, identity: Option<&'a str>) -> OutputSighting<'a> {
    OutputSighting { connector, identity }
}

#[test]
fn a_request_is_read_key_by_key() {
    let change = ProfileChange::parse(
        "mode=3440x1440@143.97\nscale=1.25\nposition=-10, 20\nenabled=false\nmirror_of=eDP-1\ntransform=flipped-90",
    )
    .expect("a request that names every setting");
    assert_eq!(change.mode, Some(Some(DisplayMode::new(3440, 1440, 143_970))));
    assert_eq!(change.scale, Some(Some(Scale::from_percent(125))));
    assert_eq!(change.position, Some(Some((-10, 20))));
    assert_eq!(change.enabled, Some(false));
    assert_eq!(change.mirror_of, Some(Some("eDP-1".to_owned())));
    assert_eq!(change.orientation, Some(Orientation::Flipped90));

    let reset = ProfileChange::parse("mode=preferred\nscale=default\nposition=auto\nmirror_of=").expect("defaults");
    assert_eq!(reset.mode, Some(None));
    assert_eq!(reset.scale, Some(None));
    assert_eq!(reset.position, Some(None));
    assert_eq!(reset.mirror_of, Some(None));
}

#[test]
fn a_bad_request_names_the_setting_that_is_wrong() {
    let error = |text: &str| ProfileChange::parse(text).expect_err(text);
    assert!(matches!(error("mode=1080p"), ProfileError::BadValue { key, .. } if key == "mode"));
    assert!(matches!(error("mode=0x0@60"), ProfileError::BadValue { key, .. } if key == "mode"));
    assert!(matches!(error("scale=9"), ProfileError::BadValue { key, .. } if key == "scale"));
    assert!(matches!(error("scale=NaN"), ProfileError::BadValue { key, .. } if key == "scale"));
    assert!(matches!(error("position=1"), ProfileError::BadValue { key, .. } if key == "position"));
    assert!(matches!(error("enabled=yes"), ProfileError::BadValue { key, .. } if key == "enabled"));
    assert!(matches!(error("transform=sideways"), ProfileError::BadValue { key, .. } if key == "transform"));
    assert_eq!(error("colour=red"), ProfileError::UnknownKey("colour".to_owned()));
    assert_eq!(error("scale"), ProfileError::Malformed("scale".to_owned()));
}

#[test]
fn a_profile_changes_the_row_it_is_laid_over() {
    let mut store = ProfileStore::new();
    let change = ProfileChange::parse("mode=2560x1440@60\nscale=1.5").expect("a change");
    store.keep(OutputProfile::new("DP-2", None).with(&change));
    let connected = [sighting("DP-1", None), sighting("DP-2", None)];
    let config = store.layout(&connected);
    let layout =
        compute(&[descriptor(1, "DP-1", FHD), descriptor(2, "DP-2", QHD)], &config).expect("the profile resolves");
    let second = layout.monitor_of(OutputId::new(2)).expect("DP-2 is placed");
    assert_eq!(second.scale, Scale::from_percent(150));
    // 2560 / 1.5 = 1706.67, rounded to a whole logical pixel, flush against DP-1's right edge.
    assert_eq!(second.rect.x, 1920);
    assert_eq!(second.rect.width, 1707);
}

#[test]
fn a_mirror_is_one_logical_monitor_with_two_members() {
    let mut store = ProfileStore::new();
    store.keep(OutputProfile::new("HDMI-1", None).with(&ProfileChange::parse("mirror_of=DP-1").expect("a change")));
    let connected = [sighting("DP-1", None), sighting("HDMI-1", None)];
    let layout = compute(
        &[descriptor(1, "DP-1", FHD), descriptor(2, "HDMI-1", QHD)],
        &store.layout(&connected),
    )
    .expect("the mirror resolves");
    assert_eq!(
        layout.monitors.len(),
        1,
        "a clone is not a second position on the desktop"
    );
    assert_eq!(layout.monitors[0].rect, Rect::new(0, 0, 1920, 1080));
    assert_eq!(layout.monitors[0].members.len(), 2);
}

#[test]
fn a_mirror_of_an_output_that_is_gone_shows_its_own_picture() {
    let mut store = ProfileStore::new();
    store.keep(OutputProfile::new("HDMI-1", None).with(&ProfileChange::parse("mirror_of=DP-1").expect("a change")));
    // DP-1 was unplugged: the profile cannot be honoured, and the layout must still resolve.
    let connected = [sighting("HDMI-1", None)];
    let config = store.layout(&connected);
    assert_eq!(config.outputs[0].mirror_of, None);
    compute(&[descriptor(2, "HDMI-1", QHD)], &config).expect("the lone output resolves");
    // A mirror source that is itself a mirror or is disabled is refused the same way.
    store.keep(OutputProfile::new("DP-1", None).with(&ProfileChange::parse("enabled=false").expect("a change")));
    let connected = [sighting("DP-1", None), sighting("HDMI-1", None)];
    let config = store.layout(&connected);
    assert_eq!(config.outputs[1].mirror_of, None);
    assert_eq!(
        config.primary.as_deref(),
        Some("HDMI-1"),
        "a disabled output is never the primary"
    );
}

#[test]
fn a_profile_waits_for_its_monitor_and_applies_again_when_it_returns() {
    let mut store = ProfileStore::new();
    let change = ProfileChange::parse("scale=1.5\nposition=1920,0").expect("a change");
    store.keep(OutputProfile::new("DP-2", Some("DEL:a0b1:42".to_owned())).with(&change));

    // Unplugged: nothing is configured for it, the lone output resolves, and the profile is parked.
    let alone = [sighting("DP-1", None)];
    let layout = compute(&[descriptor(1, "DP-1", FHD)], &store.layout(&alone)).expect("a lone output");
    assert_eq!(layout.monitors.len(), 1);
    assert_eq!(store.parked(&alone).len(), 1);

    // Replugged, the same monitor on the same port gets its arrangement back.
    let both = [sighting("DP-1", None), sighting("DP-2", Some("DEL:a0b1:42"))];
    assert!(store.parked(&both).is_empty());
    let layout = compute(
        &[descriptor(1, "DP-1", FHD), descriptor(3, "DP-2", QHD)],
        &store.layout(&both),
    )
    .expect("the stored arrangement resolves");
    let second = layout.monitor_of(OutputId::new(3)).expect("DP-2 is placed");
    assert_eq!(second.scale, Scale::from_percent(150));
    assert_eq!((second.rect.x, second.rect.y), (1920, 0));
}

#[test]
fn another_monitor_on_the_same_port_does_not_inherit_the_profile() {
    let mut store = ProfileStore::new();
    store.keep(
        OutputProfile::new("DP-2", Some("DEL:a0b1:42".to_owned()))
            .with(&ProfileChange::parse("scale=1.5").expect("a change")),
    );
    let stranger = [sighting("DP-1", None), sighting("DP-2", Some("SAM:0001:7"))];
    let config = store.layout(&stranger);
    assert_eq!(config.outputs[1].scale, None, "a different panel has its own defaults");
    assert_eq!(
        store.parked(&stranger).len(),
        1,
        "the old panel's profile is still parked"
    );
}

#[test]
fn keeping_a_profile_twice_replaces_it() {
    let mut store = ProfileStore::new();
    store.keep(OutputProfile::new("DP-1", None));
    store.keep(OutputProfile::new("DP-1", None).with(&ProfileChange::parse("enabled=false").expect("a change")));
    assert_eq!(store.all().len(), 1);
    assert!(!store.all()[0].enabled);
}

#[test]
fn an_overlap_from_a_stored_position_is_still_refused_by_name() {
    let mut store = ProfileStore::new();
    store.keep(OutputProfile::new("DP-2", None).with(&ProfileChange::parse("position=0,0").expect("a change")));
    let connected = [sighting("DP-1", None), sighting("DP-2", None)];
    let error = compute(
        &[descriptor(1, "DP-1", FHD), descriptor(2, "DP-2", FHD)],
        &store.layout(&connected),
    )
    .expect_err("two outputs at the origin overlap");
    assert!(matches!(error, LayoutError::Overlap { .. }));
}
