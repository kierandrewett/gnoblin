//! What the composer keeps to say what a surface's pixels are, and what a commit changed in them.
//!
//! A frame carries no history, so a renderer that wants to keep a surface's texture between frames needs the composer
//! to name the pixels ([`ContentKey`]): which surface, which commit of it, and which rectangles that commit changed.
//! The composer follows each surface that has pixels with a [`ContentTrack`]: one per window, per layer surface, per
//! surface of a window's tree, per canvas and per View picture.
//!
//! # Where the damage comes from
//!
//! Smithay records every damaged commit of a surface in a bag and numbers them. The protocol glue hands the composer
//! the bag's snapshot with the buffer ([`BufferDamage`]), and the track asks it for what changed since the last
//! snapshot the track saw. A caller that has no snapshot (a test, a snapshot of a closing window) says nothing about
//! the damage, which the key reports as unknown and a renderer answers with a whole upload.
//!
//! # Failure modes
//!
//! 1. **A surface's stale texture is shown.** Every change of pixels raises `commit`, so a renderer that sees the same
//!    commit knows nothing changed, and the next commit carries what did:
//!    `tests::a_commit_with_damage_carries_it_in_the_layers_key`.
//! 2. **A change is described as smaller than it was.** A new buffer with no damage recorded, a first commit and damage
//!    older than Smithay keeps are reported as unknown (`None`), never as an empty list:
//!    `tests::a_new_buffer_without_recorded_damage_is_reported_whole`.
//! 3. **Two surfaces share a key.** The surface id comes from one counter for the process, so a destroyed surface's id
//!    is never given to a later one and a renderer's cached texture is never taken for another surface's:
//!    `tests::two_windows_have_two_surfaces_and_a_snapshot_keeps_its_own`.
//! 4. **A snapshot is uploaded again every frame.** A snapshot is drawn under one key and one commit however long it
//!    fades: `tests::a_closing_windows_snapshot_has_a_stable_key`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gnoblin_render_frame::frame::{ContentKey, SurfaceSource};
use gnoblin_types::Rect;
use smithay::backend::renderer::utils::{CommitCounter, DamageSnapshot};
use smithay::utils::Buffer as BufferCoord;

/// Smithay's record of what a surface's commits damaged, in the buffer's own pixels.
pub type BufferDamage = DamageSnapshot<i32, BufferCoord>;

/// The next surface id. One counter for the process, so no two surfaces that ever lived share an id.
static NEXT_SURFACE: AtomicU64 = AtomicU64::new(1);

/// Follows one surface's pixels, so each layer it draws carries a key that says what changed.
#[derive(Clone, Debug)]
pub(super) struct ContentTrack {
    surface: u64,
    commit: u64,
    /// The last snapshot of Smithay's damage this track folded in, to ask it for what is new.
    seen: Option<CommitCounter>,
    /// What the pixels were taken from at the last commit, to notice a new buffer.
    source: Option<SurfaceSource>,
    damage: Option<Arc<[Rect]>>,
    /// The key of the commit the track is at, made when the pixels change so a frame clones a pointer and no more.
    key: Arc<ContentKey>,
}

impl ContentTrack {
    /// A track for a surface that has not committed.
    pub(super) fn new() -> Self {
        let surface = NEXT_SURFACE.fetch_add(1, Ordering::Relaxed);
        Self {
            surface,
            commit: 0,
            seen: None,
            source: None,
            damage: None,
            key: Arc::new(ContentKey {
                surface,
                commit: 0,
                damage: None,
            }),
        }
    }

    /// Folds in the surface's pixels as they are now.
    ///
    /// `damage` is Smithay's snapshot when the caller has one; the track then reports what changed since the last
    /// snapshot it saw, and nothing at all when it is the same one and the buffer is the same. Without a snapshot the
    /// change is unknown, so it is reported whole when `committed` says the surface committed or `source` is not what the
    /// track last saw, and is not a commit otherwise.
    pub(super) fn observe(&mut self, source: &SurfaceSource, damage: Option<&BufferDamage>, committed: bool) {
        let same_source = self.source.as_ref().is_some_and(|held| same_pixels(held, source));
        match damage {
            None => {
                if !committed && same_source {
                    return;
                }
                self.commit += 1;
                self.damage = None;
                self.seen = None;
            }
            Some(snapshot) => {
                let current = snapshot.current_commit();
                if self.seen == Some(current) && same_source {
                    return;
                }
                self.commit += 1;
                // A buffer that changed with no damage recorded, a first commit and damage the bag no longer holds are
                // all unknown. An unchanged commit counter with a new buffer is the first of those.
                self.damage = match snapshot.damage_since(self.seen) {
                    Some(set) if self.seen != Some(current) => Some(set.into_iter().map(rect_of).collect()),
                    _ => None,
                };
                self.seen = Some(current);
            }
        }
        self.source = Some(source.clone());
        self.refresh();
    }

    /// Notes that the pixels were replaced by a new picture the track knows nothing of (a canvas repainted, a View
    /// drawn again): a commit with unknown damage.
    pub(super) fn replaced(&mut self) {
        self.commit += 1;
        self.damage = None;
        self.seen = None;
        self.source = None;
        self.refresh();
    }

    /// Makes the key of the commit the track is at.
    fn refresh(&mut self) {
        self.key = Arc::new(ContentKey {
            surface: self.surface,
            commit: self.commit,
            damage: self.damage.clone(),
        });
    }

    /// The key a layer of these pixels carries.
    pub(super) fn key(&self) -> Arc<ContentKey> {
        Arc::clone(&self.key)
    }

    /// A key for a snapshot of the surface's pixels that never changes again: a surface of its own, at commit one.
    pub(super) fn snapshot_key() -> Arc<ContentKey> {
        Arc::new(ContentKey {
            surface: NEXT_SURFACE.fetch_add(1, Ordering::Relaxed),
            commit: 1,
            damage: None,
        })
    }
}

/// A rectangle of Smithay's damage as the frame states one: clamped to the buffer's positive quadrant.
fn rect_of(rectangle: smithay::utils::Rectangle<i32, BufferCoord>) -> Rect {
    Rect::new(
        rectangle.loc.x,
        rectangle.loc.y,
        rectangle.size.w.max(0).unsigned_abs(),
        rectangle.size.h.max(0).unsigned_abs(),
    )
}

/// Whether two sources are the same pixels: the same buffer, the same snapshot or the same texture.
fn same_pixels(left: &SurfaceSource, right: &SurfaceSource) -> bool {
    match (left, right) {
        (SurfaceSource::Buffer(left), SurfaceSource::Buffer(right)) => left == right,
        (SurfaceSource::Snapshot(left), SurfaceSource::Snapshot(right)) => Arc::ptr_eq(left, right),
        (SurfaceSource::Dmabuf(left), SurfaceSource::Dmabuf(right)) => left == right,
        (SurfaceSource::View(left), SurfaceSource::View(right)) => left == right,
        _ => false,
    }
}
