//! The window-manager provider the test sessions run.
//!
//! This crate holds no stock provider: a session with none runs `NullWm`. A suite that needs a desktop to act on (a
//! window that takes the keyboard focus when it maps, a click that focuses the window under it) starts its session
//! through [`config`]:
//!
//! - with the `std-providers` feature on (the default), the stock window manager of `gnoblin-std-wm`, so the suites run
//!   against the desktop users get, cascade, focus rotation and raise included;
//! - with it off (`--no-default-features`, the CI job `core-only`), [`FocusOnMap`], a test-only provider with the one
//!   policy the protocol tests lean on, so the core is proved without any std code.
//!
//! A test whose subject is the stock desktop's own policy is `#[cfg(feature = "std-providers")]`; every other test must
//! pass under both.

#![allow(dead_code)]

use gnoblin_wayland::{Config, WindowManagerFactory};

/// The provider a test session starts with: the stock window manager with the feature on, [`FocusOnMap`] with it off.
pub fn window_manager() -> Option<WindowManagerFactory> {
    #[cfg(feature = "std-providers")]
    {
        Some(gnoblin_std_wm::factory())
    }
    #[cfg(not(feature = "std-providers"))]
    {
        Some(WindowManagerFactory::new(|| Box::<focus_on_map::FocusOnMap>::default()))
    }
}

/// The default session configuration of the test sessions: [`Config::default`] with [`window_manager`].
pub fn config() -> Config {
    Config {
        window_manager: window_manager(),
        ..Config::default()
    }
}

/// `NullWm` plus the one policy the protocol tests lean on: a window that maps takes the keyboard focus and a click on a
/// window focuses it. Windows still open at the origin, nothing is raised and nothing is refocused when the focused
/// window goes. It is test-only: the core has no focus policy, and the stock one is `gnoblin-std-wm`'s.
#[cfg(not(feature = "std-providers"))]
mod focus_on_map {
    use gnoblin_types::{Insets, OutputId, Point, Rect, Size, WindowId};
    use gnoblin_wm_api::{
        NewWindowFocus, NullWm, Open, OutputRecord, Standing, Window, WindowKind, WindowManager, WmEffect, WmRefusal,
    };

    /// See the module documentation.
    #[derive(Debug, Default)]
    pub struct FocusOnMap(NullWm);

    impl WindowManager for FocusOnMap {
        fn window(&self, id: WindowId) -> Option<&Window> {
            self.0.window(id)
        }

        fn windows(&self) -> Box<dyn Iterator<Item = &Window> + '_> {
            self.0.windows()
        }

        fn output(&self, id: OutputId) -> Option<&OutputRecord> {
            self.0.output(id)
        }

        fn set_role(
            &mut self,
            id: WindowId,
            kind: WindowKind,
            transient_for: Option<WindowId>,
            seat: Standing<'_>,
        ) -> Vec<WmEffect> {
            self.0.set_role(id, kind, transient_for, seat)
        }

        fn may_take_focus(&self, id: WindowId, policy: NewWindowFocus, seat: Standing<'_>) -> bool {
            self.window(id).is_some()
                && match policy {
                    NewWindowFocus::Allow => true,
                    NewWindowFocus::Prevent => seat.focus.is_none(),
                }
        }

        fn open(&mut self, open: Open, seat: Standing<'_>) -> Result<Vec<WmEffect>, WmRefusal> {
            self.0.open(open, seat)
        }

        fn close(&mut self, id: WindowId, seat: Standing<'_>) -> Vec<WmEffect> {
            self.0.close(id, seat)
        }

        fn activate(&self, id: WindowId, _raise: bool, seat: Standing<'_>) -> Vec<WmEffect> {
            if self.is_visible(id) && seat.focus != Some(id) {
                vec![WmEffect::Focus(Some(id))]
            } else {
                Vec::new()
            }
        }

        fn raise(&self, id: WindowId, seat: Standing<'_>) -> Vec<WmEffect> {
            self.0.raise(id, seat)
        }

        fn lower(&self, id: WindowId, seat: Standing<'_>) -> Vec<WmEffect> {
            self.0.lower(id, seat)
        }

        fn set_maximised(&mut self, id: WindowId, on: bool) -> Vec<WmEffect> {
            self.0.set_maximised(id, on)
        }

        fn set_fullscreen(&mut self, id: WindowId, on: bool, seat: Standing<'_>) -> Vec<WmEffect> {
            self.0.set_fullscreen(id, on, seat)
        }

        fn set_minimised(&mut self, id: WindowId, on: bool, seat: Standing<'_>) -> Vec<WmEffect> {
            self.0.set_minimised(id, on, seat)
        }

        fn move_resize(&mut self, id: WindowId, wanted: Rect) -> Result<Vec<WmEffect>, WmRefusal> {
            self.0.move_resize(id, wanted)
        }

        fn place(&mut self, id: WindowId, at: Point) -> Result<Vec<WmEffect>, WmRefusal> {
            self.0.place(id, at)
        }

        fn set_rect(&mut self, id: WindowId, wanted: Rect) -> Result<Vec<WmEffect>, WmRefusal> {
            self.0.set_rect(id, wanted)
        }

        fn adopt_size(&mut self, id: WindowId, size: Size) -> bool {
            self.0.adopt_size(id, size)
        }

        fn move_to_output(&mut self, id: WindowId, target: OutputId) -> Result<Vec<WmEffect>, WmRefusal> {
            self.0.move_to_output(id, target)
        }

        fn output_added(&mut self, id: OutputId, rect: Rect, seat: Standing<'_>) -> Vec<WmEffect> {
            self.0.output_added(id, rect, seat)
        }

        fn output_removed(&mut self, id: OutputId, seat: Standing<'_>) -> Vec<WmEffect> {
            self.0.output_removed(id, seat)
        }

        fn output_moved(&mut self, id: OutputId, rect: Rect) -> Vec<WmEffect> {
            self.0.output_moved(id, rect)
        }

        fn reserve(&mut self, id: OutputId, reserved: Insets) -> Vec<WmEffect> {
            self.0.reserve(id, reserved)
        }
    }
}
