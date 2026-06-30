use crate::{QsMenuRow, QsTile, QsTileRow, TopBar};
use gnoblin_shell_ui::find_icon;
use std::rc::Rc;

/// Push a quick-settings snapshot into the control-centre popout + the status
/// cluster (network glyph, mute). Shared by the one-shot read and the poller.
fn apply_cluster(p: &TopBar, st: &gnoblin_shell_ui::quicksettings::QuickState) {
    // Cluster network glyph: wired wins when both are up (the active route, as
    // GNOME does); else wifi; else disconnected. GNOBLIN_NET_MODE overrides for
    // headless validation of the wifi/disconnected variants.
    let net_mode = std::env::var("GNOBLIN_NET_MODE")
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(if st.wired {
            1
        } else if st.wifi {
            2
        } else {
            0
        });
    p.set_network_mode(net_mode);

    // GNOBLIN_AUDIO_MUTED overrides for headless validation of the mute glyph.
    let muted = std::env::var("GNOBLIN_AUDIO_MUTED")
        .map(|v| v == "1")
        .unwrap_or(st.muted);
    p.set_audio_muted(muted);
}

/// Greedily pack tiles into rows of a 4-wide span grid, wrapping when the next
/// tile would overflow the row's 4 columns.
fn pack_rows(tiles: Vec<QsTile>) -> Vec<QsTileRow> {
    let mut rows: Vec<Vec<QsTile>> = Vec::new();
    let mut cur: Vec<QsTile> = Vec::new();
    let mut used = 0;
    for t in tiles {
        let span = t.span.clamp(1, 4);
        if used + span > 4 && !cur.is_empty() {
            rows.push(std::mem::take(&mut cur));
            used = 0;
        }
        used += span;
        cur.push(t);
        if used >= 4 {
            rows.push(std::mem::take(&mut cur));
            used = 0;
        }
    }
    if !cur.is_empty() {
        rows.push(cur);
    }
    rows.into_iter()
        .map(|tiles| QsTileRow {
            tiles: Rc::new(slint::VecModel::from(tiles)).into(),
        })
        .collect()
}

/// Build the unified tile list from every ready plugin tile. The host preserves
/// declared config order; this module only maps snapshots to Slint rows.
fn build_tiles(plugins: &[gnoblin_shell_ui::qsplugin::PluginState]) -> Vec<QsTile> {
    let mut tiles = Vec::new();
    for pl in plugins {
        let spec = &pl.update.tile;
        let layout = if spec.layout.is_empty() {
            "toggle"
        } else {
            spec.layout.as_str()
        };
        let span = if spec.span == 0 {
            if layout == "toggle" {
                2
            } else {
                4
            }
        } else {
            spec.span
        };
        let rows: Vec<QsMenuRow> = pl
            .update
            .menu
            .as_ref()
            .map(|m| {
                m.rows
                    .iter()
                    .map(|r| QsMenuRow {
                        id: r.id.clone().into(),
                        kind: r.kind.clone().into(),
                        label: r.label.clone().into(),
                        sublabel: r.sublabel.clone().into(),
                        icon: find_icon(&r.icon, "").unwrap_or_default(),
                        value: r.value,
                        on: r.on,
                    })
                    .collect()
            })
            .unwrap_or_default();
        tiles.push(QsTile {
            id: pl.id.clone().into(),
            span,
            layout: layout.into(),
            icon_name: String::new().into(),
            icon: find_icon(&spec.icon, "").unwrap_or_default(),
            title: spec.title.clone().into(),
            subtitle: spec.subtitle.clone().into(),
            active: spec.active,
            chevron: spec.chevron || !rows.is_empty(),
            // Clamp to the track range: a slider plugin (e.g. gnoblin-qs-output)
            // can report >1.0 for an over-amplified PipeWire volume, which would
            // overdraw the fill past the track.
            value: spec.value.clamp(0.0, 1.0),
            rows: Rc::new(slint::VecModel::from(rows)).into(),
        });
    }

    tiles
}

/// Push the unified control-centre tile model + status cluster from a known
/// state snapshot.
pub(crate) fn push(
    p: &TopBar,
    st: &gnoblin_shell_ui::quicksettings::QuickState,
    plugins: &[gnoblin_shell_ui::qsplugin::PluginState],
) {
    apply_cluster(p, st);
    let tiles = build_tiles(plugins);
    // If a slide-out submenu is open, refresh its rows from the freshly-built
    // model so a plugin update mid-open doesn't leave the page showing a stale
    // snapshot (the rows were captured when the chevron was tapped).
    let open = p.get_cc_open_submenu().to_string();
    if !open.is_empty() {
        if let Some(t) = tiles.iter().find(|t| t.id == open) {
            p.set_cc_submenu_rows(t.rows.clone());
        }
    }
    p.set_cc_tiles(Rc::new(slint::VecModel::from(pack_rows(tiles))).into());
}

/// Re-read live built-in state and rebuild the control-centre tile grid.
pub(crate) fn refresh(p: &TopBar, plugins: &[gnoblin_shell_ui::qsplugin::PluginState]) {
    let st = gnoblin_shell_ui::quicksettings::read();
    push(p, &st, plugins);
}
