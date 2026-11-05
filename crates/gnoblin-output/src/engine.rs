//! The layout engine: a declarative configuration becomes logical monitor rectangles.

use crate::geometry::{Rect, Size};
use crate::ids::OutputId;
use crate::layout::{Align, Edge, Layout, LayoutConfig, LogicalMonitor, OutputConfig, Placement, clamp_to_i32};
use crate::mode::Mode;
use crate::output::{OutputDescriptor, VrrSupport};
use crate::scale::Scale;
use crate::transform::Transform;

/// Why a configuration could not become a layout.
///
/// Each variant names the entry or the pair that failed, because the consumer has
/// to tell a person which output to fix.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LayoutError {
    /// No connected output is enabled, so the session would show nothing.
    NoEnabledOutputs,
    /// The configuration has two entries for one output.
    DuplicateOutput(OutputId),
    /// The output has no modes at all, so no mode can be chosen.
    NoPreferredMode(OutputId),
    /// The requested mode is not one the connector accepts.
    ModeNotSupported {
        /// The output.
        output: OutputId,
        /// The refused mode.
        mode: Mode,
    },
    /// The requested scale is not one the session offers for this output.
    ScaleNotSupported {
        /// The output.
        output: OutputId,
        /// The refused scale.
        scale: Scale,
    },
    /// An enabled, non-mirrored output has no placement.
    MissingPlacement(OutputId),
    /// A relative placement names an output that is not an enabled monitor.
    DanglingAnchor {
        /// The output being placed.
        output: OutputId,
        /// The anchor it names.
        anchor: OutputId,
    },
    /// Relative placements form a cycle, so no output in it has a position.
    PlacementCycle(OutputId),
    /// A mirror names another mirror, which has no size of its own.
    MirrorOfMirror {
        /// The mirrored output.
        output: OutputId,
        /// The mirror it names.
        source: OutputId,
    },
    /// A mirror names an output that is not connected and enabled.
    MirrorSourceUnavailable {
        /// The mirrored output.
        output: OutputId,
        /// The source it names.
        source: OutputId,
    },
    /// A mirror sets its own mode, scale, transform or VRR.
    MirrorOverridesSource(OutputId),
    /// The requested primary is not an enabled member of the layout.
    PrimaryNotEnabled(OutputId),
    /// Two logical monitors share a pixel.
    Overlap {
        /// One of the monitors.
        first: OutputId,
        /// The other monitor.
        second: OutputId,
    },
    /// A logical monitor is not edge-adjacent to the rest of the layout.
    Disconnected(OutputId),
}

/// One enabled, non-mirrored output while the layout is being resolved.
struct Source<'a> {
    entry: &'a OutputConfig,
    descriptor: &'a OutputDescriptor,
    mode: Mode,
    scale: Scale,
    size: Size,
    rect: Option<Rect>,
    mirrors: Vec<OutputId>,
}

/// Turns a configuration into the layout it describes.
///
/// Entries for outputs that are not connected are inert, so one configuration
/// survives a dock going away and coming back. Everything else either resolves to
/// a rectangle or fails by name: nothing here invents a position, because an
/// invented position is what turns a stale profile into a monitor the pointer
/// cannot reach.
pub fn compute(connected: &[OutputDescriptor], config: &LayoutConfig) -> Result<Layout, LayoutError> {
    let mut sources: Vec<Source<'_>> = Vec::new();

    for (position, entry) in config.outputs.iter().enumerate() {
        if config.outputs[..position].iter().any(|earlier| earlier.id == entry.id) {
            return Err(LayoutError::DuplicateOutput(entry.id.clone()));
        }
        if !entry.enabled {
            continue;
        }
        let Some(descriptor) = connected.iter().find(|descriptor| descriptor.id == entry.id) else {
            continue;
        };
        if entry.mirror_of.is_some() {
            continue;
        }
        let mode = match entry.mode {
            Some(mode) => mode,
            None => descriptor
                .preferred_mode()
                .ok_or_else(|| LayoutError::NoPreferredMode(entry.id.clone()))?,
        };
        if !descriptor.supports_mode(mode) {
            return Err(LayoutError::ModeNotSupported {
                output: entry.id.clone(),
                mode,
            });
        }
        let scale = entry.scale.unwrap_or_else(|| descriptor.default_scale());
        if !descriptor.supports_scale(scale) {
            return Err(LayoutError::ScaleNotSupported {
                output: entry.id.clone(),
                scale,
            });
        }
        sources.push(Source {
            entry,
            descriptor,
            mode,
            scale,
            size: mode.logical_size(entry.transform, scale),
            rect: None,
            mirrors: Vec::new(),
        });
    }

    if sources.is_empty() {
        return Err(LayoutError::NoEnabledOutputs);
    }

    attach_mirrors(connected, config, &mut sources)?;
    resolve_placements(&mut sources)?;

    let mut monitors = Vec::with_capacity(sources.len());
    for source in &sources {
        let Some(rect) = source.rect else {
            // `resolve_placements` either places every source or returns an error.
            return Err(LayoutError::MissingPlacement(source.entry.id.clone()));
        };
        let mut members = source.mirrors.clone();
        members.push(source.entry.id.clone());
        members.sort();
        let Some(id) = members.first().cloned() else {
            continue;
        };
        monitors.push(LogicalMonitor {
            id,
            rect,
            scale: source.scale,
            transform: source.entry.transform,
            vrr: source.entry.vrr && source.descriptor.vrr == VrrSupport::Supported,
            mode: source.mode,
            members,
        });
    }

    for (index, monitor) in monitors.iter().enumerate() {
        for other in &monitors[index + 1..] {
            if monitor.rect.overlaps(other.rect) {
                return Err(LayoutError::Overlap {
                    first: monitor.id.clone(),
                    second: other.id.clone(),
                });
            }
        }
    }

    if !config.allow_gaps {
        check_connected(&monitors)?;
    }

    monitors.sort_by(|left, right| (left.rect.x, left.rect.y, &left.id).cmp(&(right.rect.x, right.rect.y, &right.id)));

    let primary = match &config.primary {
        Some(primary) => {
            if !monitors.iter().any(|monitor| monitor.contains(primary)) {
                return Err(LayoutError::PrimaryNotEnabled(primary.clone()));
            }
            Some(primary.clone())
        }
        None => connected
            .iter()
            .find(|descriptor| monitors.iter().any(|monitor| monitor.contains(&descriptor.id)))
            .map(|descriptor| descriptor.id.clone()),
    };

    Ok(Layout { monitors, primary })
}

/// Attaches every mirror to the group it clones.
///
/// A mirror has no size or position of its own: it presents the source's mode at
/// the source's scale on its own panel, so anything it declares for itself is a
/// contradiction rather than an override. Its placement is simply ignored, since
/// `OutputConfig::enabled` sets one by default and the source decides where the
/// group sits.
fn attach_mirrors(
    connected: &[OutputDescriptor],
    config: &LayoutConfig,
    sources: &mut [Source<'_>],
) -> Result<(), LayoutError> {
    for entry in &config.outputs {
        if !entry.enabled {
            continue;
        }
        let Some(source_id) = &entry.mirror_of else {
            continue;
        };
        if !connected.iter().any(|descriptor| descriptor.id == entry.id) {
            continue;
        }
        if entry.mode.is_some() || entry.scale.is_some() || entry.transform != Transform::Normal || entry.vrr {
            return Err(LayoutError::MirrorOverridesSource(entry.id.clone()));
        }
        let Some(index) = sources.iter().position(|source| &source.entry.id == source_id) else {
            let names_a_mirror = config
                .outputs
                .iter()
                .any(|other| &other.id == source_id && other.mirror_of.is_some());
            return Err(if names_a_mirror {
                LayoutError::MirrorOfMirror {
                    output: entry.id.clone(),
                    source: source_id.clone(),
                }
            } else {
                LayoutError::MirrorSourceUnavailable {
                    output: entry.id.clone(),
                    source: source_id.clone(),
                }
            });
        };
        sources[index].mirrors.push(entry.id.clone());
    }
    Ok(())
}

/// Resolves every source's rectangle, waiting for anchors that are still unresolved.
///
/// A single pass would only work for a configuration written in dependency order,
/// and a configuration is data from a file: it may place the left monitor after
/// the right one. Only a pass that stops making progress is a real failure, and
/// then the entry is a cycle or a dangling anchor.
fn resolve_placements(sources: &mut [Source<'_>]) -> Result<(), LayoutError> {
    let mut pending: Vec<usize> = (0..sources.len()).collect();
    while !pending.is_empty() {
        let mut resolved: Vec<usize> = Vec::new();
        for (position, index) in pending.iter().enumerate() {
            let source = &sources[*index];
            let size = source.size;
            let rect = match &source.entry.placement {
                None => return Err(LayoutError::MissingPlacement(source.entry.id.clone())),
                Some(Placement::Origin) => Some(Rect::new(0, 0, size.width, size.height)),
                Some(Placement::Absolute { x, y }) => Some(Rect::new(*x, *y, size.width, size.height)),
                Some(Placement::Relative { of, edge, align }) => source_index(sources, of)
                    .and_then(|anchor| sources[anchor].rect)
                    .map(|anchor| place(anchor, size, *edge, *align)),
            };
            if let Some(rect) = rect {
                sources[*index].rect = Some(rect);
                resolved.push(position);
            }
        }
        if resolved.is_empty() {
            let source = &sources[pending[0]];
            return Err(match &source.entry.placement {
                Some(Placement::Relative { of, .. }) if source_index(sources, of).is_some() => {
                    LayoutError::PlacementCycle(source.entry.id.clone())
                }
                Some(Placement::Relative { of, .. }) => LayoutError::DanglingAnchor {
                    output: source.entry.id.clone(),
                    anchor: of.clone(),
                },
                // Every other placement resolves in the first pass.
                _ => LayoutError::MissingPlacement(source.entry.id.clone()),
            });
        }
        for position in resolved.into_iter().rev() {
            pending.remove(position);
        }
    }
    Ok(())
}

/// The group index that `id` names, following one mirror to its source.
fn source_index(sources: &[Source<'_>], id: &OutputId) -> Option<usize> {
    if let Some(index) = sources.iter().position(|source| &source.entry.id == id) {
        return Some(index);
    }
    sources.iter().position(|source| source.mirrors.contains(id))
}

/// Places `size` against one side of `anchor`.
///
/// Alignment only slides the output along the shared edge, so the two are always
/// flush where they join: the layout cannot develop a one-pixel seam that makes
/// the pointer jump.
fn place(anchor: Rect, size: Size, edge: Edge, align: Align) -> Rect {
    let anchor_width = i64::from(anchor.width);
    let anchor_height = i64::from(anchor.height);
    let width = i64::from(size.width);
    let height = i64::from(size.height);
    let left = i64::from(anchor.x);
    let top = i64::from(anchor.y);
    let along = |start: i64, extent: i64, own: i64| match align {
        Align::Start => start,
        Align::Center => start + (extent - own) / 2,
        Align::End => start + extent - own,
    };
    let (x, y) = match edge {
        Edge::Right => (left + anchor_width, along(top, anchor_height, height)),
        Edge::Left => (left - width, along(top, anchor_height, height)),
        Edge::Below => (along(left, anchor_width, width), top + anchor_height),
        Edge::Above => (along(left, anchor_width, width), top - height),
    };
    Rect::new(clamp_to_i32(x), clamp_to_i32(y), size.width, size.height)
}

/// Requires every monitor to be reachable from the first by shared edges.
fn check_connected(monitors: &[LogicalMonitor]) -> Result<(), LayoutError> {
    if monitors.len() < 2 {
        return Ok(());
    }
    let mut reached = vec![false; monitors.len()];
    reached[0] = true;
    let mut frontier = vec![0_usize];
    while let Some(index) = frontier.pop() {
        for (other, monitor) in monitors.iter().enumerate() {
            if reached[other] || !monitors[index].rect.touches(monitor.rect) {
                continue;
            }
            reached[other] = true;
            frontier.push(other);
        }
    }
    match reached.iter().position(|reached| !reached) {
        Some(index) => Err(LayoutError::Disconnected(monitors[index].id.clone())),
        None => Ok(()),
    }
}
