//! The magnifier model: which area of which output is shown, and nothing else.
//!
//! The magnifier is a source rectangle into one output. Every path below ends in
//! [`Rect::magnified`], which clamps the rectangle inside the output, so no
//! reachable state describes a pixel that is not on that output. A locked
//! session pins the output for the same reason: following the pointer onto a
//! neighbour would show that neighbour's area instead of the lock screen.

use crate::geometry::Point;
use crate::{Follow, MagnifierConfig, Output, View};

/// The magnifier's state: the layout, the follow anchor and the view.
pub struct Magnifier {
    config: MagnifierConfig,
    layout: Vec<Output>,
    view: Option<View>,
    /// Where the configured follow source last put the view.
    anchor: Option<Point>,
    /// Whether the session is covered. While it is, the output is pinned and
    /// follow events are ignored, because they describe covered clients.
    locked: bool,
}

impl Magnifier {
    /// A magnifier for this configuration.
    #[must_use]
    pub fn new(config: MagnifierConfig) -> Self {
        Self {
            config,
            layout: Vec::new(),
            view: None,
            anchor: None,
            locked: false,
        }
    }

    /// Applies a configuration change and reports whether the view changed.
    pub fn configure(&mut self, config: MagnifierConfig) -> bool {
        self.config = config;
        self.recompute()
    }

    /// Replaces the output layout and reports whether the view changed.
    pub fn set_layout(&mut self, outputs: &[Output]) -> bool {
        self.layout = outputs.to_vec();
        self.recompute()
    }

    /// Follows a focus, caret or pointer event and reports whether the view
    /// changed. Only the configured source moves the view.
    pub fn follow(&mut self, source: Follow, at: Point) -> bool {
        if !self.config.enabled || self.locked || self.config.follow != source {
            return false;
        }
        self.anchor = Some(at);
        self.recompute()
    }

    /// Pins the view to its current output while the session is covered, and
    /// reports whether the view changed.
    pub fn set_locked(&mut self, locked: bool) -> bool {
        self.locked = locked;
        self.recompute()
    }

    /// The magnifier's view, or `None` when it is off or nothing is known about
    /// the layout.
    #[must_use]
    pub const fn view(&self) -> Option<View> {
        self.view
    }

    fn recompute(&mut self) -> bool {
        let view = self.compute();
        let changed = view != self.view;
        self.view = view;
        changed
    }

    fn compute(&self) -> Option<View> {
        if !self.config.enabled {
            return None;
        }
        let output = self.output()?;
        Some(View {
            output: output.id,
            source: output.bounds.magnified(self.anchor_point(output), self.config.zoom),
            zoom: self.config.zoom,
            mode: self.config.mode,
        })
    }

    /// The output the view belongs to.
    ///
    /// Unlocked, the anchor picks it and the current view is only a fallback, so
    /// following the pointer onto another output moves the magnifier there.
    /// Locked, the current output wins, so nothing can move the view off the
    /// output that is showing the lock screen.
    fn output(&self) -> Option<&Output> {
        let by_anchor = if self.locked {
            None
        } else {
            self.anchor
                .and_then(|anchor| self.layout.iter().find(|output| output.bounds.contains(anchor)))
        };
        let by_view = self
            .view
            .and_then(|view| self.layout.iter().find(|output| output.id == view.output));
        by_anchor.or(by_view).or_else(|| self.layout.first())
    }

    /// Where the view is centred.
    ///
    /// While the session is locked the anchor is moved into the output, so a
    /// pointer on a neighbour cannot describe that neighbour's area.
    fn anchor_point(&self, output: &Output) -> Point {
        let anchor = self.anchor.unwrap_or_else(|| output.bounds.centre());
        if self.locked {
            output.bounds.clamp(anchor)
        } else {
            anchor
        }
    }
}
