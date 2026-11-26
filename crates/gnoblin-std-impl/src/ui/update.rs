//! The toolkit's turn: what runs once after something changed, and the update pass of a canvas.
//!
//! A canvas is updated in four steps. It refreshes its fields from live state, arranges its children, updates them, and writes what
//! changed: the node's properties, and the paint list when its bytes differ from the last ones. A step that fails is reported and
//! counted, and the pass goes on. The roots update in the order they were made.

use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gnoblin_core::Value;
use gnoblin_core::objects::Timers;
use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::{Closure, Val};
use gnoblin_paint::PaintOp;

use super::tree::find;
use super::widget::{Cid, Special, Ui, WidgetIds, WidgetKind};
use super::{bar, clock, entry, paint, toast};
use crate::menus;

/// How often a canvas that animates (a toast's fade) asks for a turn, in milliseconds.
const FRAME_MS: u64 = 16;

/// The turn the toolkit posted: the frame hooks run, then every root canvas updates its tree.
pub(crate) fn turn(ctx: GnoblinCtx) {
    let Some(ui) = find(ctx) else {
        return;
    };
    ui.pending.set(false);
    ui.frame();
}

/// Milliseconds on a clock that only moves forward.
pub(crate) fn now_ms() -> f64 {
    static START: std::sync::LazyLock<std::time::Instant> = std::sync::LazyLock::new(std::time::Instant::now);
    START.elapsed().as_secs_f64() * 1000.0
}

/// Milliseconds since the Unix epoch on the system's clock.
pub(crate) fn epoch_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1000.0)
}

impl Ui {
    /// The core's clock in milliseconds (`gnoblin.timers.now`): the clock the core's timers, and so the toolkit's wake timer, run by. A delay
    /// the toolkit times for a core timer to wake is due on this clock, so whatever moves it (a test rig's `ManualClock`) moves the delay. When
    /// the core's clock cannot be read the toolkit's own is used, and that is reported.
    pub(crate) fn core_now(&self) -> f64 {
        match self.core_clock() {
            Ok(timers) => timers.now().unwrap_or_else(|fail| {
                self.warn(&format!("the core's clock could not be read: {fail}"));
                now_ms()
            }),
            Err(fail) => {
                self.warn(&format!("the core's clock is not reachable: {fail}"));
                now_ms()
            }
        }
    }

    /// The core's timers, got once.
    fn core_clock(&self) -> Result<Rc<Timers>, String> {
        if let Some(timers) = self.core_timers.borrow().as_ref() {
            return Ok(Rc::clone(timers));
        }
        let objects = self.g.objects().map_err(|fail| fail.to_string())?;
        let timers = Rc::new(Timers::get(&objects).map_err(|fail| fail.to_string())?);
        *self.core_timers.borrow_mut() = Some(Rc::clone(&timers));
        Ok(timers)
    }

    /// A clock a part is timed by, in milliseconds: the one the shell gave it, or the core's. `what` names the clock in the report when the
    /// shell's throws, and the core's is read then.
    pub(crate) fn clock_ms(&self, source: Option<&Closure>, what: &str) -> f64 {
        match source {
            Some(source) => source
                .call(0, &[])
                .and_then(|value| value.double())
                .unwrap_or_else(|fail| {
                    self.warn(&format!("{what} threw: {}", fail.message));
                    self.core_now()
                }),
            None => self.core_now(),
        }
    }
}

impl Ui {
    /// One frame of the toolkit: the frame hooks run, then each root canvas updates its whole tree, and the next wake is planned.
    pub(crate) fn frame(self: &Rc<Self>) {
        menus::watch(self);
        crate::lock::turn(self);
        let hooks: Vec<_> = self
            .frame_hooks
            .borrow()
            .iter()
            .map(|hook| Rc::clone(&hook.run))
            .collect();
        for hook in hooks {
            if let Err(fail) = hook.call(0, &[]) {
                self.warn(&format!("onUiFrame threw: {fail}"));
            }
        }
        crate::shell::turn(self);
        crate::frame::turn(self);
        crate::policy::turn(self);
        let roots = self.w.borrow().roots();
        for root in roots {
            self.update(root);
        }
        crate::frame::after_update(self);
        crate::lock::after_update(self);
        self.plan_wake();
    }

    /// Updates `cid` and everything under it.
    pub(crate) fn update(self: &Rc<Self>, cid: Cid) {
        if !self.alive(cid) {
            return;
        }
        self.step(cid, "refresh", |ui| ui.refresh(cid));
        self.step(cid, "arrange", |ui| ui.arrange(cid));
        let children = self.read(cid, |widget| widget.children.clone()).unwrap_or_default();
        for child in children {
            self.update(child);
        }
        self.step(cid, "draw", |ui| ui.flush(cid));
    }

    /// Runs one step of the update pass. A failure is counted and reported, and the pass goes on. A step that fails the same way every
    /// turn is reported once, until it succeeds or fails another way.
    fn step(self: &Rc<Self>, cid: Cid, what: &'static str, body: impl FnOnce(&Rc<Ui>) -> Result<(), String>) {
        match body(self) {
            Ok(()) => {
                self.tweak(cid, |widget| widget.reported.retain(|(step, _)| *step != what));
            }
            Err(message) => {
                let first = self
                    .tweak(cid, |widget| {
                        widget.failures += 1;
                        let seen = widget
                            .reported
                            .iter()
                            .any(|(step, held)| *step == what && *held == message);
                        if !seen {
                            widget.reported.retain(|(step, _)| *step != what);
                            widget.reported.push((what, message.clone()));
                        }
                        !seen
                    })
                    .unwrap_or(true);
                if first {
                    let title = self.read(cid, |widget| widget.kind.title()).unwrap_or("Canvas");
                    self.warn(&format!("{title}.{what} threw: {message}"));
                }
            }
        }
    }

    /// Asks the shell's implementation of `widget` for `member` of `cid`, when it defined it. `None`: it did not, and the library's own
    /// behaviour runs.
    pub(crate) fn hook_call(
        &self,
        cid: Cid,
        pick: impl FnOnce(&WidgetIds) -> u32,
        args: &[Val],
    ) -> Option<Result<Val, String>> {
        let behaviour = self.read(cid, |widget| widget.behaviour.clone()).flatten()?;
        let member = pick(&self.ids().widget);
        if member == 0 || !behaviour.shell.has(member) {
            return None;
        }
        Some(behaviour.shell.call(member, args).map_err(|fail| fail.message))
    }

    /// Updates the canvas's fields from live state, then lets the shell's own refresh run.
    fn refresh(self: &Rc<Self>, cid: Cid) -> Result<(), String> {
        match self.read(cid, |widget| widget.kind) {
            Some(WidgetKind::Entry) => entry::refresh(self, cid)?,
            Some(WidgetKind::Clock) => clock::refresh(self, cid)?,
            Some(WidgetKind::Bar) => bar::refresh(self, cid)?,
            Some(WidgetKind::Toast) => toast::refresh(self, cid)?,
            Some(kind @ (WidgetKind::MenuItem | WidgetKind::MenuPanel | WidgetKind::Menu)) => {
                menus::refresh(self, cid, kind)
            }
            _ => {}
        }
        crate::frame::refresh(self, cid)?;
        crate::shell::refresh(self, cid)?;
        if let Some(done) = self.hook_call(cid, |ids| ids.refresh, &[]) {
            done?;
        }
        Ok(())
    }

    /// Places the children: the shell's arrangement when it defined one, else the library's.
    fn arrange(self: &Rc<Self>, cid: Cid) -> Result<(), String> {
        match self.hook_call(cid, |ids| ids.arrange, &[]) {
            Some(done) => done.map(drop),
            None => {
                self.arrange_default(cid);
                Ok(())
            }
        }
    }

    /// The width: the assigned one, or the content's natural width.
    pub(crate) fn width_of(&self, cid: Cid) -> f64 {
        match self.read(cid, |widget| widget.width) {
            Some(Some(width)) => width,
            Some(None) => self.natural_width(cid),
            None => 0.0,
        }
    }

    /// The height: the assigned one, or the content's natural height.
    pub(crate) fn height_of(&self, cid: Cid) -> f64 {
        match self.read(cid, |widget| widget.height) {
            Some(Some(height)) => height,
            Some(None) => self.natural_height(cid),
            None => 0.0,
        }
    }

    /// What a width that was never given is: a text's measured width, an icon's size, nothing for the rest, or what the shell's
    /// `natural_width` says.
    pub(crate) fn natural_width(&self, cid: Cid) -> f64 {
        match self.hook_call(cid, |ids| ids.natural_width, &[]) {
            Some(Ok(value)) => {
                if let Ok(width) = value.double() {
                    return width;
                }
            }
            Some(Err(message)) => self.warn(&format!("natural_width threw: {message}")),
            None => {}
        }
        if let Some(width) = crate::frame::natural(self, cid, false) {
            return width;
        }
        match self.read(cid, |widget| (widget.kind, widget.i.size)) {
            Some((kind, _)) if kind.is_text() => self.metrics(cid).width,
            Some((WidgetKind::Icon, size)) => size,
            _ => 0.0,
        }
    }

    /// What a height that was never given is.
    pub(crate) fn natural_height(&self, cid: Cid) -> f64 {
        match self.hook_call(cid, |ids| ids.natural_height, &[]) {
            Some(Ok(value)) => {
                if let Ok(height) = value.double() {
                    return height;
                }
            }
            Some(Err(message)) => self.warn(&format!("natural_height threw: {message}")),
            None => {}
        }
        if let Some(height) = crate::frame::natural(self, cid, true) {
            return height;
        }
        match self.read(cid, |widget| (widget.kind, widget.i.size)) {
            Some((kind, _)) if kind.is_text() => self.metrics(cid).height,
            Some((WidgetKind::Icon, size)) => size,
            _ => 0.0,
        }
    }

    /// The operations the canvas paints: the shell's when it defined `paint`, else the library's own.
    pub(crate) fn paint_ops(&self, cid: Cid) -> Result<Vec<PaintOp>, String> {
        match self.hook_call(cid, |ids| ids.paint, &[]) {
            Some(done) => paint::read(&done?),
            None => self.paint_default(cid),
        }
    }

    /// The operations the library paints a canvas with by itself.
    pub(crate) fn paint_default(&self, cid: Cid) -> Result<Vec<PaintOp>, String> {
        if let Some(ops) = crate::frame::paint(self, cid).or_else(|| crate::lock::paint(self, cid)) {
            return ops;
        }
        let Some(kind) = self.read(cid, |widget| widget.kind) else {
            return Ok(Vec::new());
        };
        match kind {
            WidgetKind::Canvas => Ok(Vec::new()),
            WidgetKind::Box
            | WidgetKind::Grid
            | WidgetKind::Bar
            | WidgetKind::Toast
            | WidgetKind::MenuItem
            | WidgetKind::MenuPanel
            | WidgetKind::Menu => self.box_ops(cid),
            WidgetKind::MenuMark | WidgetKind::MenuSeparator => menus::ops(self, cid, kind),
            WidgetKind::Text | WidgetKind::Clock => self.text_ops(cid),
            WidgetKind::Icon => {
                let (size, color, name) = self
                    .read(cid, |widget| {
                        (widget.i.size, widget.i.color.clone(), widget.i.icon.clone())
                    })
                    .unwrap_or_default();
                Ok(vec![paint::icon(size, &color, &name)?])
            }
            WidgetKind::Entry => entry::ops(self, cid),
        }
    }

    /// A box's background: one rectangle over the whole canvas.
    pub(crate) fn box_ops(&self, cid: Cid) -> Result<Vec<PaintOp>, String> {
        let (radius, fill) = self
            .read(cid, |widget| (widget.b.radius, widget.b.fill.clone()))
            .unwrap_or_default();
        Ok(vec![paint::rect(
            0.0,
            0.0,
            self.width_of(cid),
            self.height_of(cid),
            radius,
            &fill,
        )?])
    }

    /// A text's string, centred on the canvas vertically. If the canvas is shorter than the string's block, it is clipped equally at
    /// the top and the bottom.
    fn text_ops(&self, cid: Cid) -> Result<Vec<PaintOp>, String> {
        let y = ((self.height_of(cid) - self.metrics(cid).height) / 2.0).floor();
        let shown = self.shown(cid);
        let (size, color, font) = self
            .read(cid, |widget| {
                (widget.t.size, widget.t.color.clone(), widget.t.font.clone())
            })
            .unwrap_or_default();
        Ok(vec![paint::text(0.0, y, size, &color, &font, &shown)?])
    }

    /// Writes one property of the node: a node the compositor already ended ends the canvas, and nothing more is written.
    fn write(self: &Rc<Self>, cid: Cid, node: u64, key: &str, value: impl Into<Value>) -> Result<bool, String> {
        let target = self.node(node);
        match target.set(key, value) {
            Ok(()) => Ok(true),
            Err(error) => {
                if target.is_alive() {
                    Err(format!("the host refused {key} ({error})"))
                } else {
                    self.node_gone(cid);
                    Ok(false)
                }
            }
        }
    }

    /// Writes the changed properties of the canvas's node, and its paint list when the bytes differ from the last ones.
    fn flush(self: &Rc<Self>, cid: Cid) -> Result<(), String> {
        let Some((node, now, written, previous)) = self.read(cid, |widget| {
            (
                widget.node,
                (
                    widget.x,
                    widget.y,
                    widget.opacity,
                    widget.visible,
                    widget.secure,
                    widget.input,
                    widget.placement,
                ),
                widget.written.clone(),
                widget.content.clone(),
            )
        }) else {
            return Ok(());
        };
        if !self.alive(cid) {
            return Ok(());
        }
        let (x, y, opacity, visible, secure, input, placement) = now;
        let width = self.width_of(cid);
        let height = self.height_of(cid);
        for (key, value, was) in [
            ("x", x, written.x),
            ("y", y, written.y),
            ("width", width, written.width),
            ("height", height, written.height),
            ("opacity", opacity, written.opacity),
        ] {
            if value != was {
                if !value.is_finite() {
                    return Err(format!("the host refused {key} (invalid-value)"));
                }
                if !self.write(cid, node, key, value)? {
                    return Ok(());
                }
                self.tweak(cid, |widget| match key {
                    "x" => widget.written.x = value,
                    "y" => widget.written.y = value,
                    "width" => widget.written.width = value,
                    "height" => widget.written.height = value,
                    _ => widget.written.opacity = value,
                });
            }
        }
        if visible != written.visible {
            if !self.write(cid, node, "visible", visible)? {
                return Ok(());
            }
            self.tweak(cid, |widget| widget.written.visible = visible);
        }
        if secure != written.secure {
            if !self.write(cid, node, "secure", secure)? {
                return Ok(());
            }
            self.tweak(cid, |widget| widget.written.secure = secure);
        }
        if input != written.input {
            if !self.write(cid, node, "input", input)? {
                return Ok(());
            }
            self.tweak(cid, |widget| widget.written.input = input);
        }
        if placement != written.placement {
            let word = super::widget::en::placement_word(placement);
            if !self.write(cid, node, "placement", word)? {
                return Ok(());
            }
            self.tweak(cid, |widget| widget.written.placement = placement);
        }
        let bytes = paint::bytes(self.paint_ops(cid)?);
        if previous.as_ref() != Some(&bytes) {
            if !self.write(cid, node, "content", Value::Bytes(bytes.clone()))? {
                return Ok(());
            }
            self.tweak(cid, |widget| widget.content = Some(bytes));
        }
        Ok(())
    }

    /// Asks for a turn when a canvas that keeps time needs one: a toast's fade follows its age, so a turn runs while one is up, and a
    /// clock changes its text on the system clock's minute or second.
    fn plan_wake(self: &Rc<Self>) {
        let mut wake: Option<u64> = None;
        let ids: Vec<Cid> = self.w.borrow().widgets.keys().copied().collect();
        for cid in ids {
            let due = self.read(cid, |widget| {
                if !widget.alive {
                    return None;
                }
                match &widget.sp {
                    Special::Toast(_) => Some(FRAME_MS),
                    Special::Clock(part) => Some(clock::changes_in(&part.format)),
                    _ => None,
                }
            });
            if let Some(Some(delay)) = due {
                wake = Some(wake.map_or(delay, |held| held.min(delay)));
            }
        }
        for delay in [
            crate::frame::wake_in(self),
            crate::shell::wake_in(self),
            crate::lock::wake_in(self),
            crate::policy::wake_in(self),
        ]
        .into_iter()
        .flatten()
        {
            wake = Some(wake.map_or(delay, |held| held.min(delay)));
        }
        if menus::polls(self) || crate::policy::polls(self) {
            wake = Some(wake.map_or(FRAME_MS, |held| held.min(FRAME_MS)));
        }
        if let Some(old) = self.timer.borrow_mut().take() {
            // A timer that already fired is not an error.
            let _ = old.unsubscribe();
        }
        let Some(delay) = wake else {
            return;
        };
        let ctx = self.ctx;
        let delay = u32::try_from(delay.max(1)).unwrap_or(u32::MAX);
        match self.g.after(delay, move || {
            if let Some(ui) = find(ctx) {
                ui.schedule();
            }
        }) {
            Ok(timer) => *self.timer.borrow_mut() = Some(timer),
            Err(error) => self.warn(&format!("the toolkit could not set its timer: {error}")),
        }
    }
}
