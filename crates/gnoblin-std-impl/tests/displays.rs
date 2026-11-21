#![allow(unsafe_code)]
//! Output profiles, displays, viewports and the display manager of `libgnoblin-std`, held to what the TypeScript classes did
//! (`data/js/gnoblin-std/display/*`, `display_manager/*`).
//!
//! Each test replays a case of `crates/gnoblin-js/tests/displays.rs` as a shell written over the object protocol: the same screens, the same
//! windows, the same steps, over the real core and the real library and the fake compositor. The TypeScript run asserted these; nothing was
//! recorded as a fixture for displays (`design/native-std-port-tests.md`), so the expectations below are the ones of the TypeScript tests,
//! with the words of the native checks.
//!
//! **Allowances of the replay** (each is where it is made, with its reason):
//!
//! 1. `gnoblin.input.primary.activeDisplay` belongs to the input context (NpCoreInput); the display under a point is the manager's
//!    `display_at`, which is what `activeDisplay` asks, and `the_display_under_a_point_...` asserts it for the pointer positions of the
//!    TypeScript test.
//! 2. A native display is made with `selectors`, and its `id`, `mode` and `viewport` are set after, as the catalog's constructor takes the
//!    selectors only. A `mode` of two outputs left at `single` is refused when the display is added (the TypeScript constructor refused
//!    it at once).
//! 3. A mode given as `{ width, height, refreshRate }` is `format_mode(width, height, refresh_rate)`; a transform is an enum, so a
//!    value outside the eight is refused by the protocol and not by a check of the library.
//! 4. The tests that `Output.apply` and `Output.mirror` of the core's classes answer (supersample of a script's own request) are the
//!    core's (`gnoblin-core-impl`); the profile's supersample is replayed here.

#[path = "ui/rig.rs"]
mod rig;

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core_abi as abi;
use gnoblin_object::{Fail, Val};
use gnoblin_testkit::compositor::text as mirror_text;
use gnoblin_tree_client::tree_mirror::MirrorValue;
use rig::{Rig, W, b as truth, d, en, t};

/// `assert_eq!` for values the protocol carries, which do not compare: they compare as they print.
macro_rules! assert_val {
    ($left:expr, $right:expr $(,)?) => {
        assert_eq!(format!("{:?}", $left), format!("{:?}", $right))
    };
}

const A: u64 = 20;
const B: u64 = 22;

const SINGLE: u32 = 1;
const MIRROR: u32 = 2;
const SPAN: u32 = 3;

// ---- values ------------------------------------------------------------------------------------------------------------------------

fn rec(fields: Vec<Val>) -> Val {
    Val::Record { class: 0, fields }
}

fn none() -> Val {
    Val::None
}

fn matching(connector: &str) -> Val {
    rec(vec![t(connector), none(), none(), none()])
}

fn hardware(make: &str, model: &str, serial: &str) -> Val {
    rec(vec![none(), t(make), t(model), t(serial)])
}

/// What a `profile_settings` value asks for; a field left `None` asks nothing.
#[derive(Default)]
struct Asks {
    mode: Option<&'static str>,
    scale: Option<f64>,
    supersample: Option<f64>,
    transform: Option<u32>,
    vrr: Option<bool>,
    position: Option<Val>,
    enabled: Option<bool>,
}

impl Asks {
    fn val(&self) -> Val {
        rec(vec![
            self.mode.map_or_else(none, t),
            self.scale.map_or_else(none, d),
            self.supersample.map_or_else(none, d),
            self.transform.map_or_else(none, en),
            self.vrr.map_or_else(none, truth),
            self.position.clone().unwrap_or_else(none),
            self.enabled.map_or_else(none, truth),
        ])
    }
}

fn at(x: i64, y: i64) -> Val {
    rec(vec![en(1), Val::Int(x), Val::Int(y), none(), none(), none()])
}

const RIGHT_OF: u32 = 2;
const LEFT_OF: u32 = 3;

fn beside(kind: u32, profile: &W, align: Option<&str>) -> Val {
    rec(vec![
        en(kind),
        none(),
        none(),
        profile.val(),
        none(),
        align.map_or_else(none, t),
    ])
}

fn profile_selector(profile: &W) -> Val {
    rec(vec![profile.val(), none()])
}

fn output_selector(node: u64) -> Val {
    rec(vec![none(), Val::Uint(node)])
}

fn field(value: &Val, at: usize) -> Val {
    match value {
        Val::Record { fields, .. } => fields.get(at).cloned().unwrap_or(Val::None),
        other => panic!("a record, got {other:?}"),
    }
}

// ---- the world ---------------------------------------------------------------------------------------------------------------------

/// A shell over screen `A` (`DP-1`, 1000 by 800, primary) and, when `second`, screen `B` (`HDMI-A-1`, 600 by 400 at x 1000), a seat and one
/// window (id 3), with three workspaces whose layout gives a window the whole area, in the list.
struct World {
    rig: Rig,
    manager: W,
    workspaces: W,
    one: W,
    two: W,
    three: W,
}

fn wrap(rig: &Rig, value: &Val) -> W {
    match value {
        Val::Object { class, obj } => W {
            api: rig.api,
            class: *class,
            obj: *obj,
        },
        other => panic!("{other:?} is not an object"),
    }
}

fn external(rig: &mut Rig, node: u64, key: &str, value: MirrorValue) {
    rig.external(node, key, value);
}

fn world(second: bool) -> World {
    let mut rig = Rig::scratch_policy(1);
    external(&mut rig, A, "connector", mirror_text("DP-1"));
    if second {
        rig.comp
            .create_screen(B, "output-1", (1000.0, 0.0, 600.0, 400.0), false);
        external(&mut rig, B, "connector", mirror_text("HDMI-A-1"));
    }
    rig.settle();
    let workspaces = rig.make("workspaces", &[]);
    let mut made = Vec::new();
    for id in ["one", "two", "three"] {
        let workspace = rig.make("workspace", &[("id", t(id))]);
        workspace.set("layout", rig.make("master_stack_layout", &[]).val());
        workspaces.call("add", &[workspace.val()]);
        made.push(workspace);
    }
    let manager = rig.make("display_manager", &[]);
    rig.settle();
    World {
        rig,
        manager,
        workspaces,
        one: made[0],
        two: made[1],
        three: made[2],
    }
}

impl World {
    fn settle(&mut self) {
        self.rig.settle();
    }

    /// Which workspace each output shows, as `name=id` words.
    fn shown(&self) -> String {
        let mut words = Vec::new();
        for (name, node) in [("output-0", A), ("output-1", B)] {
            if !self.rig.comp.exists(node) {
                continue;
            }
            let shown = self.workspaces.call("of_output", &[Val::Uint(node)]);
            words.push(match shown {
                Val::None => format!("{name}=none"),
                held => format!("{name}={}", wrap(&self.rig, &held).text("id")),
            });
        }
        words.join(" ")
    }

    fn displays(&self) -> Vec<W> {
        self.manager
            .get("displays")
            .list()
            .expect("a list")
            .iter()
            .map(|held| wrap(&self.rig, held))
            .collect()
    }

    /// The displays as `id:mode:outputs:workspace` words, sorted.
    fn listed(&self) -> String {
        let mut words: Vec<String> = self.displays().iter().map(|display| self.describe(display)).collect();
        words.sort();
        words.join(" ")
    }

    fn describe(&self, display: &W) -> String {
        let mode = match display.get("mode") {
            Val::Enum { value, .. } => ["single", "mirror", "span"][value as usize - 1],
            other => panic!("a mode, got {other:?}"),
        };
        let outputs = display.get("outputs").list().expect("a list").len();
        format!(
            "{}:{mode}:{outputs}:{}",
            display.text("id"),
            self.content_of(display).unwrap_or_else(|| "none".to_owned())
        )
    }

    /// The id of the workspace a display's viewport shows.
    fn content_of(&self, display: &W) -> Option<String> {
        match display.obj("viewport").get("content") {
            Val::None => None,
            held => Some(wrap(&self.rig, &held).text("id")),
        }
    }

    fn named(&self, id: &str) -> Option<W> {
        match self.manager.call("display_named", &[t(id)]) {
            Val::None => None,
            held => Some(wrap(&self.rig, &held)),
        }
    }

    fn profile(&self, connector: &str) -> W {
        self.rig.make("output_profile", &[("match", matching(connector))])
    }

    /// A display of the shell's own: `selectors`, a name, a mode and the workspace its viewport holds.
    fn display(&self, id: &str, selectors: Vec<Val>, mode: u32, content: Option<&W>) -> W {
        let display = self.rig.make("display", &[("selectors", Val::List(selectors))]);
        display.set("id", t(id));
        display.set("mode", en(mode));
        let viewport = self.rig.make("viewport", &[]);
        if let Some(workspace) = content {
            viewport.set("content", workspace.val());
        }
        display.set("viewport", viewport.val());
        display
    }

    fn add(&self, display: &W) {
        self.manager.call("add_display", &[display.val()]);
    }

    /// The output requests sent for screen `node`, in order.
    fn requests(&self, node: u64) -> Vec<String> {
        self.rig
            .comp
            .applies()
            .into_iter()
            .filter(|(target, ..)| *target == node)
            .map(|(_, text, _)| text)
            .collect()
    }

    /// Answers every request not answered yet with success.
    fn answer_all(&mut self, answered: &mut usize) {
        let applies = self.rig.comp.applies();
        for (_, _, token) in &applies[*answered..] {
            self.rig.comp.answer(*token, "", "");
        }
        *answered = applies.len();
        self.settle();
    }
}

/// The texts of a signal's first field, as they arrive.
fn listen(rig: &Rig, object: &W, signal: &str) -> Rc<RefCell<Vec<Vec<Val>>>> {
    let heard = Rc::new(RefCell::new(Vec::new()));
    let kept = Rc::clone(&heard);
    let member = rig
        .api
        .member(object.class, abi::MEMBER_SIGNAL, signal)
        .expect("a signal");
    rig.api
        .listen(object.obj, member, move |_, args| {
            kept.borrow_mut().push(args.to_vec());
            Ok(Val::None)
        })
        .expect("a listener");
    heard
}

fn notes(heard: &Rc<RefCell<Vec<Vec<Val>>>>) -> Vec<String> {
    heard
        .borrow()
        .iter()
        .map(|args| args[0].text().expect("a sentence").to_owned())
        .collect()
}

/// Makes an object and answers the failure when the library refuses it.
fn try_make(rig: &Rig, class: &str, inits: &[(&str, Val)]) -> Result<W, Fail> {
    let class_id = rig.api.class(class).expect("the class");
    let mut ids = Vec::new();
    for (name, value) in inits {
        let member = rig
            .api
            .member(class_id, abi::MEMBER_PROPERTY, name)
            .unwrap_or_else(|fail| panic!("{class} has no {name}: {fail}"));
        ids.push((member, value.clone()));
    }
    rig.api.create(class_id, &ids).map(|obj| W {
        api: rig.api,
        class: class_id,
        obj,
    })
}

fn refused<T: std::fmt::Debug>(result: Result<T, Fail>) -> Fail {
    result.expect_err("the library refuses it")
}

/// A function of the class `output_profile`, which answers the failure when the library refuses it.
fn try_function(rig: &Rig, name: &str, args: &[Val]) -> Result<Val, Fail> {
    let class = rig.api.class("output_profile").expect("the class");
    let member = rig
        .api
        .member(class, abi::MEMBER_METHOD, name)
        .unwrap_or_else(|fail| panic!("{name}: {fail}"));
    rig.api.call_static(class, member, args)
}

/// A transition of 300 milliseconds that runs `run`, which the library stages: what it changes is put back when it fails.
fn request(rig: &Rig, run: Val) -> Result<Val, Fail> {
    let class = rig.api.class("transition").expect("the class");
    let member = rig
        .api
        .member(class, abi::MEMBER_METHOD, "request")
        .expect("the function");
    let options = rec(vec![d(300.0), none(), d(0.0), truth(true)]);
    rig.api.call_static(class, member, &[run, options, none()])
}

// ---- the list of displays and the display under a point ------------------------------------------------------------------------------

#[test]
fn the_library_keeps_one_display_per_output_and_the_display_under_a_point_is_the_one_asked_for() {
    let w = world(true);
    assert_eq!(
        w.listed(),
        "display-DP-1:single:1:one display-HDMI-A-1:single:1:two",
        "each output with a desk has a single display showing what its desk shows"
    );
    let under = |w: &World, x: f64, y: f64| -> String {
        match w.manager.call("display_at", &[d(x), d(y)]) {
            Val::None => "none".to_owned(),
            held => wrap(&w.rig, &held).text("id"),
        }
    };
    assert_eq!(
        under(&w, 10.0, 10.0),
        "display-DP-1",
        "the pointer is on the first output"
    );
    assert_eq!(under(&w, 1100.0, 10.0), "display-HDMI-A-1");
    assert_eq!(
        under(&w, 9000.0, 9000.0),
        "display-DP-1",
        "a point outside every output falls back to the primary output's display"
    );
    let first = w.named("display-DP-1").expect("a display");
    let outputs = first.get("outputs");
    assert_eq!(
        outputs
            .list()
            .expect("a list")
            .iter()
            .map(|node| node.uint().expect("a node"))
            .collect::<Vec<_>>(),
        vec![A],
        "a display's outputs are the nodes of the outputs it presents"
    );
    assert!(first.flag("implicit") && first.flag("listed"));
}

#[test]
fn a_display_is_removed_with_its_output_and_returns_with_the_next_one() {
    let mut w = world(true);
    w.rig.comp.destroy(B);
    w.settle();
    assert_eq!(w.listed(), "display-DP-1:single:1:one");
    w.rig
        .comp
        .create_screen(30, "output-2", (1000.0, 0.0, 640.0, 480.0), false);
    external(&mut w.rig, 30, "connector", mirror_text("DP-2"));
    w.settle();
    assert_eq!(
        w.listed(),
        "display-DP-1:single:1:one display-DP-2:single:1:two",
        "a new output gets a display of its own"
    );
}

#[test]
fn a_display_the_library_keeps_is_one_object_and_stays_when_removed() {
    let mut w = world(true);
    let first = w.named("display-DP-1").expect("a display");
    let again = w.named("display-DP-1").expect("a display");
    assert_eq!(first.obj, again.obj, "the object for a display is the one object");
    let manager = w.manager;
    assert!(
        !manager.call("remove_display", &[first.val()]).bool().expect("a flag"),
        "the display of the library's own stays: removing it answers false"
    );
    w.settle();
    assert_eq!(manager.get("displays").list().expect("a list").len(), 2);
}

// ---- viewports -----------------------------------------------------------------------------------------------------------------------

#[test]
fn assigning_a_viewports_content_shows_the_workspace_on_that_display_as_show_does() {
    let mut w = world(true);
    let left = w.named("display-DP-1").expect("a display");
    let right = w.named("display-HDMI-A-1").expect("a display");
    left.obj("viewport").set("content", w.three.val());
    w.settle();
    assert_eq!(w.shown(), "output-0=three output-1=two");
    assert_eq!(w.content_of(&left).as_deref(), Some("three"));
    // A workspace is on one display at a time: the one that another display shows moves, and that display takes the one this display
    // showed, exactly as `workspaces.show` does.
    right.obj("viewport").set("content", w.three.val());
    w.settle();
    assert_eq!(w.shown(), "output-0=two output-1=three");
    assert_eq!(
        w.content_of(&left).as_deref(),
        Some("two"),
        "a display reads what its desk shows"
    );
    let loose = w.rig.make("workspace", &[("id", t("loose"))]);
    let fail = refused(left.obj("viewport").try_set("content", loose.val()));
    assert_eq!(
        fail.code,
        abi::E_REFUSED,
        "a workspace that is not in the list cannot be shown, as with `show`: {}",
        fail.message
    );
    assert!(
        left.obj("viewport").try_set("content", t("one")).is_err(),
        "a text is not a workspace"
    );
}

#[test]
fn a_viewport_that_no_display_shows_keeps_its_workspace_and_a_camera_of_its_own() {
    let w = world(true);
    let viewport = w.rig.make("viewport", &[]);
    assert_val!(viewport.get("content"), Val::None);
    assert_val!(viewport.get("display"), Val::None);
    viewport.set("content", w.two.val());
    assert_eq!(wrap(&w.rig, &viewport.get("content")).text("id"), "two");
    assert_eq!(viewport.text("clear_color"), "#000000");
    let camera = viewport.obj("camera");
    assert!(
        camera.flag("identity"),
        "a viewport made without a camera has a flat one with no pan, zoom or roll"
    );
    assert_eq!(
        viewport.obj("camera").obj,
        camera.obj,
        "asking again gives the same camera"
    );
    let fail = refused(viewport.try_set("clear_color", t("")));
    assert!(
        fail.message.contains("must be a colour such as #101820"),
        "{}",
        fail.message
    );
    let plain = w.rig.make("scene_node", &[]);
    let fail = refused(viewport.try_set("camera", plain.val()));
    assert!(
        fail.message.contains("must be a Camera2D or a Camera3D"),
        "{}",
        fail.message
    );
}

// ---- displays of the shell's own -----------------------------------------------------------------------------------------------------

#[test]
fn a_script_display_is_listed_and_an_output_is_presented_by_one_display_at_a_time() {
    let mut w = world(true);
    let a = w.profile("DP-1");
    let mine = w.display("left", vec![profile_selector(&a)], SINGLE, Some(&w.three));
    let rival = w.display("also-left", vec![profile_selector(&a)], SINGLE, Some(&w.two));
    w.add(&mine);
    w.settle();
    assert_eq!(
        w.listed(),
        "display-HDMI-A-1:single:1:two left:single:1:three",
        "the display for DP-1 is the shell's, which shows the workspace it asked for"
    );
    let fail = refused(w.manager.try_call("add_display", &[rival.val()]));
    assert_eq!(fail.code, abi::E_REFUSED);
    assert!(
        fail.message.contains("\"also-left\"") && fail.message.contains("\"left\"") && fail.message.contains("DP-1"),
        "the error names both displays and the output: {}",
        fail.message
    );
    // `replace_display` swaps them as one change.
    w.manager.call("replace_display", &[mine.val(), rival.val()]);
    w.settle();
    assert_eq!(
        w.listed(),
        "also-left:single:1:two display-HDMI-A-1:single:1:three",
        "the rival shows its workspace on DP-1, and the output that showed it takes what DP-1 showed"
    );
    assert!(w.named("left").is_none());
    assert!(!mine.flag("listed") && rival.flag("listed"));
}

#[test]
fn an_id_that_is_taken_is_busy_and_the_display_keeps_its_id_while_it_is_listed() {
    let w = world(true);
    let a = w.profile("DP-1");
    let b = w.profile("HDMI-A-1");
    let first = w.display("same", vec![profile_selector(&a)], SINGLE, None);
    let second = w.display("same", vec![profile_selector(&b)], SINGLE, None);
    w.add(&first);
    let fail = refused(w.manager.try_call("add_display", &[second.val()]));
    assert_eq!(fail.code, abi::E_BUSY, "{}", fail.message);
    assert!(fail.message.contains("already a display \"same\""), "{}", fail.message);
    let fail = refused(first.try_set("id", t("other")));
    assert_eq!(fail.code, abi::E_REFUSED);
    assert!(fail.message.contains("cannot change while"), "{}", fail.message);
    second.set("id", t("different"));
    w.add(&second);
}

#[test]
fn removing_a_script_display_gives_its_output_a_display_of_the_librarys_again() {
    let mut w = world(true);
    let a = w.profile("DP-1");
    let mine = w.display("left", vec![profile_selector(&a)], SINGLE, Some(&w.one));
    w.add(&mine);
    w.settle();
    let ids = |w: &World| {
        let mut ids: Vec<String> = w.displays().iter().map(|display| display.text("id")).collect();
        ids.sort();
        ids.join(" ")
    };
    assert_eq!(ids(&w), "display-HDMI-A-1 left");
    assert!(w.manager.call("remove_display", &[mine.val()]).bool().expect("a flag"));
    w.settle();
    assert_eq!(ids(&w), "display-DP-1 display-HDMI-A-1");
    assert!(
        !w.manager.call("remove_display", &[mine.val()]).bool().expect("a flag"),
        "a display that is not in the list is not removed"
    );
}

#[test]
fn a_display_whose_output_is_not_connected_is_parked_with_its_workspace_and_wakes_when_the_output_arrives() {
    let mut w = world(true);
    let hdmi9 = w.profile("HDMI-9");
    let parked = w.display("parked", vec![profile_selector(&hdmi9)], SINGLE, Some(&w.one));
    w.add(&parked);
    w.settle();
    assert_eq!(
        parked.get("outputs").list().expect("a list").len(),
        0,
        "no output matches: the display is parked"
    );
    assert_eq!(
        w.content_of(&parked).as_deref(),
        Some("one"),
        "the workspace it was given is kept while it is parked"
    );
    assert_eq!(
        w.shown(),
        "output-0=one output-1=two",
        "a parked display changes nothing"
    );
    w.rig
        .comp
        .create_screen(30, "output-2", (1600.0, 0.0, 640.0, 480.0), false);
    external(&mut w.rig, 30, "connector", mirror_text("HDMI-9"));
    w.settle();
    assert_eq!(parked.get("outputs").list().expect("a list").len(), 1);
    let shown = w.workspaces.call("of_output", &[Val::Uint(30)]);
    assert_eq!(
        wrap(&w.rig, &shown).text("id"),
        "one",
        "the display shows its workspace on the output that arrived"
    );
}

#[test]
fn display_arguments_are_checked() {
    let w = world(true);
    let a = w.profile("DP-1");
    let b = w.profile("HDMI-A-1");
    let message = |result: Result<W, Fail>| refused(result.map(|_| ())).message;
    assert!(
        message(try_make(&w.rig, "display", &[("selectors", Val::List(Vec::new()))]))
            .contains("Display.selectors: must list at least one"),
    );
    let neither = rec(vec![none(), none()]);
    assert!(
        message(try_make(&w.rig, "display", &[("selectors", Val::List(vec![neither]))]))
            .contains("each entry must be an OutputProfile or an Output")
    );
    assert!(
        message(try_make(
            &w.rig,
            "display",
            &[("selectors", Val::List(vec![output_selector(9999)]))]
        ))
        .contains("each entry must be an OutputProfile or an Output"),
        "an output that is not connected is not one"
    );
    let two = w.rig.make(
        "display",
        &[("selectors", Val::List(vec![profile_selector(&a), profile_selector(&b)]))],
    );
    let fail = refused(w.manager.try_call("add_display", &[two.val()]));
    assert!(
        fail.message.contains("\"single\" presents one output"),
        "{}",
        fail.message
    );
    let fail = refused(two.try_set("id", t("")));
    assert!(
        fail.message.contains("Display.id: must be a non-empty string"),
        "{}",
        fail.message
    );
    two.set("mode", en(MIRROR));
    two.set("mode", en(SPAN));
    let fail = refused(two.try_set("mode", en(SINGLE)));
    assert!(
        fail.message
            .contains("\"single\" presents one output; use \"mirror\" or \"span\" for 2"),
        "{}",
        fail.message
    );
    // A viewport belongs to one display.
    let viewport = w.rig.make("viewport", &[]);
    viewport.set("content", w.one.val());
    let first = w
        .rig
        .make("display", &[("selectors", Val::List(vec![profile_selector(&a)]))]);
    first.set("viewport", viewport.val());
    let second = w
        .rig
        .make("display", &[("selectors", Val::List(vec![profile_selector(&b)]))]);
    let fail = refused(second.try_set("viewport", viewport.val()));
    assert!(fail.message.contains("already shown by display"), "{}", fail.message);
    assert_eq!(wrap(&w.rig, &viewport.get("display")).obj, first.obj);
}

#[test]
fn the_displays_a_shell_makes_are_numbered_in_the_order_they_are_made() {
    let w = world(true);
    let a = w.profile("DP-1");
    let first = w
        .rig
        .make("display", &[("selectors", Val::List(vec![profile_selector(&a)]))]);
    let second = w
        .rig
        .make("display", &[("selectors", Val::List(vec![profile_selector(&a)]))]);
    assert_eq!(
        (first.text("id"), second.text("id")),
        ("display-1".to_owned(), "display-2".to_owned())
    );
    assert_eq!(wrap(&w.rig, &first.get("viewport")).obj, first.obj("viewport").obj);
    assert_eq!(wrap(&w.rig, &first.obj("viewport").get("display")).obj, first.obj);
}

// ---- mirror and span ------------------------------------------------------------------------------------------------------------------

#[test]
fn a_mirror_display_asks_the_compositor_to_mirror_and_ends_the_mirror_when_it_is_removed() {
    let mut w = world(true);
    let (a, b) = (w.profile("DP-1"), w.profile("HDMI-A-1"));
    let mirror = w.display(
        "mirror",
        vec![profile_selector(&a), profile_selector(&b)],
        MIRROR,
        Some(&w.one),
    );
    w.add(&mirror);
    w.settle();
    assert_eq!(
        w.requests(B),
        vec!["mirror_of=DP-1"],
        "the second output mirrors the first"
    );
    assert!(w.requests(A).is_empty());
    // The compositor takes the mirror: the second output has no desk then, and the display still presents both.
    external(&mut w.rig, B, "mirror_of", mirror_text("DP-1"));
    w.settle();
    assert_eq!(w.listed(), "mirror:mirror:2:one");
    assert!(
        w.manager
            .call("remove_display", &[mirror.val()])
            .bool()
            .expect("a flag")
    );
    w.settle();
    assert_eq!(
        w.requests(B),
        vec!["mirror_of=DP-1", "mirror_of="],
        "removing the display stops the mirror it made"
    );
}

#[test]
fn a_span_display_lays_one_workspace_out_over_the_union_of_its_outputs() {
    let mut w = world(true);
    let (a, b) = (w.profile("DP-1"), w.profile("HDMI-A-1"));
    let span = w.display(
        "span",
        vec![profile_selector(&a), profile_selector(&b)],
        SPAN,
        Some(&w.one),
    );
    w.add(&span);
    w.settle();
    assert_eq!(
        w.shown(),
        "output-0=one output-1=one",
        "both outputs show the span's workspace"
    );
    assert_eq!(
        w.rig.comp.rect(3),
        "0,0,1600,800",
        "the layout area is the bounding box of the outputs' areas: 1000 by 800 and 600 by 400 at x 1000"
    );
    assert_eq!(w.listed(), "span:span:2:one");
    span.obj("viewport").set("content", w.two.val());
    w.settle();
    assert_eq!(
        w.shown(),
        "output-0=two output-1=two",
        "the span shows one workspace at a time"
    );
    assert!(w.manager.call("remove_display", &[span.val()]).bool().expect("a flag"));
    w.settle();
    assert_eq!(
        w.shown(),
        "output-0=two output-1=one",
        "without the span each output shows a workspace of its own again"
    );
}

// ---- profiles --------------------------------------------------------------------------------------------------------------------------

#[test]
fn a_profile_applies_its_settings_as_one_request_and_places_its_neighbour_from_the_answer() {
    let mut w = world(false);
    w.rig
        .comp
        .create_screen(B, "output-1", (1200.0, 0.0, 600.0, 400.0), false);
    external(&mut w.rig, B, "connector", mirror_text("HDMI-A-1"));
    w.settle();
    let main = w.profile("DP-1");
    let mode = w
        .rig
        .function("output_profile", "format_mode", &[d(1000.0), d(800.0), d(60.0)]);
    assert_val!(mode, t("1000x800@60"));
    main.set(
        "settings",
        Asks {
            mode: Some("1000x800@60"),
            scale: Some(1.25),
            transform: Some(1),
            position: Some(at(0, 0)),
            ..Asks::default()
        }
        .val(),
    );
    let second = w.profile("HDMI-A-1");
    second.set(
        "settings",
        Asks {
            mode: Some("preferred"),
            position: Some(beside(RIGHT_OF, &main, Some("top"))),
            ..Asks::default()
        }
        .val(),
    );
    w.manager.call("add_profile", &[main.val()]);
    w.manager.call("add_profile", &[second.val()]);
    w.settle();
    assert_eq!(
        w.requests(A),
        vec!["mode=1000x800@60\nscale=1.25\ntransform=normal\nposition=0,0"],
        "one request carries all of a profile's settings"
    );
    assert_eq!(
        w.requests(B),
        vec!["mode=preferred"],
        "a relative position waits for the neighbour"
    );
    let mut answered = 0;
    w.answer_all(&mut answered);
    assert_eq!(
        w.requests(B),
        vec!["mode=preferred", "position=1000,0"],
        "once the neighbour has answered, the profile is placed against its rectangle"
    );
    w.answer_all(&mut answered);
    let sent = w.rig.comp.applies().len();
    w.settle();
    assert_eq!(w.rig.comp.applies().len(), sent, "a settled profile sends nothing more");
}

#[test]
fn a_profile_applies_again_when_its_monitor_returns_and_a_refusal_is_reported_by_key() {
    let mut w = world(true);
    let noted = listen(&w.rig, &w.manager, "noted");
    let p = w.profile("HDMI-A-1");
    p.set(
        "settings",
        Asks {
            scale: Some(2.0),
            ..Asks::default()
        }
        .val(),
    );
    w.manager.call("add_profile", &[p.val()]);
    w.settle();
    let first = w.rig.comp.applies();
    assert_eq!(first.len(), 1);
    w.rig
        .comp
        .answer(first[0].2, "", "scale: the backend does not take 2 here");
    w.settle();
    assert!(
        notes(&noted)
            .iter()
            .any(|line| line == "outputs.profiles.HDMI-A-1.scale: the backend does not take 2 here"),
        "{:?}",
        notes(&noted)
    );
    w.rig.comp.destroy(B);
    w.settle();
    w.rig
        .comp
        .create_screen(B + 10, "output-3", (1000.0, 0.0, 600.0, 400.0), false);
    external(&mut w.rig, B + 10, "connector", mirror_text("HDMI-A-1"));
    w.settle();
    assert_eq!(
        w.requests(B + 10),
        vec!["scale=2"],
        "the monitor is back: the profile applies to it again"
    );
}

#[test]
fn the_compositors_answer_to_a_profiles_request_reaches_the_managers_listeners() {
    let mut w = world(false);
    let applied = listen(&w.rig, &w.manager, "applied");
    let p = w.profile("DP-1");
    p.set(
        "settings",
        Asks {
            scale: Some(2.0),
            ..Asks::default()
        }
        .val(),
    );
    w.manager.call("add_profile", &[p.val()]);
    w.settle();
    let sent = w.rig.comp.applies();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(applied.borrow().is_empty(), "nothing is answered yet");
    w.rig.comp.answer(sent[0].2, "", "");
    w.settle();
    let words = |heard: &Rc<RefCell<Vec<Vec<Val>>>>| -> Vec<(u64, String)> {
        heard
            .borrow()
            .iter()
            .map(|args| {
                (
                    args[0].uint().expect("a node"),
                    args[1].text().expect("a text").to_owned(),
                )
            })
            .collect()
    };
    assert_eq!(
        words(&applied),
        vec![(A, String::new())],
        "a taken request is announced with no error"
    );
    p.set(
        "settings",
        Asks {
            scale: Some(3.0),
            ..Asks::default()
        }
        .val(),
    );
    w.settle();
    let again = w.rig.comp.applies();
    assert_eq!(again.len(), 2, "{again:?}");
    w.rig
        .comp
        .answer(again[1].2, "", "scale: the backend does not take 3 here");
    w.settle();
    assert_eq!(
        words(&applied),
        vec![
            (A, String::new()),
            (A, "scale: the backend does not take 3 here".to_owned())
        ],
        "a refused one carries the compositor's words"
    );
}

#[test]
fn a_profile_waits_for_the_answer_to_its_own_request_before_it_sends_the_next() {
    let mut w = world(false);
    let p = w.profile("DP-1");
    p.set(
        "settings",
        Asks {
            scale: Some(2.0),
            ..Asks::default()
        }
        .val(),
    );
    w.manager.call("add_profile", &[p.val()]);
    w.settle();
    let sent = w.rig.comp.applies();
    assert_eq!(sent.len(), 1, "{sent:?}");
    p.set(
        "settings",
        Asks {
            scale: Some(3.0),
            ..Asks::default()
        }
        .val(),
    );
    w.settle();
    assert_eq!(
        w.rig.comp.applies().len(),
        1,
        "the output has a request in flight: the new scale waits for its answer"
    );
    w.rig.comp.answer(sent[0].2, "", "");
    w.settle();
    let after = w.rig.comp.applies();
    assert_eq!(after.len(), 2, "{after:?}");
    assert_eq!(after[1].1, "scale=3");
}

#[test]
fn profiles_match_by_make_model_and_serial_and_report_vrr_as_not_applied() {
    let mut w = world(true);
    let noted = listen(&w.rig, &w.manager, "noted");
    external(&mut w.rig, B, "make", mirror_text("BOE"));
    external(&mut w.rig, B, "model", mirror_text("Panel"));
    external(&mut w.rig, B, "identity", mirror_text("BOE:0a1b:SN42"));
    w.settle();
    let panel = w
        .rig
        .make("output_profile", &[("match", hardware("BOE", "Panel", "SN42"))]);
    assert_eq!(
        panel.text("id"),
        "BOE/Panel/SN42",
        "the default name is the match written out"
    );
    panel.set(
        "settings",
        Asks {
            scale: Some(1.5),
            vrr: Some(true),
            ..Asks::default()
        }
        .val(),
    );
    w.manager.call("add_profile", &[panel.val()]);
    w.settle();
    assert_eq!(w.requests(B), vec!["scale=1.5"]);
    assert!(
        w.requests(A).is_empty(),
        "a profile for one monitor leaves the other alone"
    );
    let vrr: Vec<String> = notes(&noted).into_iter().filter(|line| line.contains("vrr")).collect();
    assert_eq!(vrr.len(), 1, "{vrr:?}");
    assert!(
        vrr[0].starts_with("not applied: OutputProfile \"BOE/Panel/SN42\" vrr"),
        "{vrr:?}"
    );
}

#[test]
fn the_profile_list_and_profile_arguments_are_checked() {
    let w = world(true);
    let a = w.profile("DP-1");
    let b = w.profile("HDMI-A-1");
    assert_eq!(w.manager.get("profiles").list().expect("a list").len(), 0);
    w.manager.call("add_profile", &[a.val()]);
    assert!(a.flag("listed"));
    let found = w.manager.call("profile_named", &[t("DP-1")]);
    assert_eq!(
        wrap(&w.rig, &found).obj,
        a.obj,
        "the profile of an id is the one object"
    );
    let twin = w.profile("DP-1");
    let fail = refused(w.manager.try_call("add_profile", &[twin.val()]));
    assert_eq!(fail.code, abi::E_BUSY);
    assert!(
        fail.message.contains("there is already a profile \"DP-1\""),
        "{}",
        fail.message
    );
    assert!(w.manager.call("remove_profile", &[a.val()]).bool().expect("a flag"));
    assert_eq!(w.manager.get("profiles").list().expect("a list").len(), 0);
    let message = |result: Result<W, Fail>| refused(result.map(|_| ())).message;
    assert!(
        message(try_make(&w.rig, "output_profile", &[("match", rec(vec![none(); 4]))]))
            .contains("OutputProfile.match: needs at least one")
    );
    assert!(
        message(try_make(&w.rig, "output_profile", &[("match", matching(""))]))
            .contains("OutputProfile.match.connector: must be a non-empty string")
    );
    let set = |p: &W, asks: Asks| refused(p.try_set("settings", asks.val())).message;
    assert!(
        set(
            &a,
            Asks {
                scale: Some(9.0),
                ..Asks::default()
            }
        )
        .contains("OutputProfile \"DP-1\".scale must be a number from 0.5 to 4")
    );
    assert!(
        set(
            &a,
            Asks {
                supersample: Some(17.0),
                ..Asks::default()
            }
        )
        .contains("OutputProfile \"DP-1\".supersample must be a number from 1 to 16")
    );
    assert!(
        set(
            &a,
            Asks {
                mode: Some("big"),
                ..Asks::default()
            }
        )
        .contains("OutputProfile \"DP-1\".mode must be \"preferred\" or \"WIDTHxHEIGHT@HERTZ\"")
    );
    let width_only = refused(try_function(&w.rig, "format_mode", &[d(1.0), d(1.0), d(0.0)]));
    assert!(
        width_only.message.contains("OutputProfile.mode.refresh_rate"),
        "{}",
        width_only.message
    );
    let fraction = refused(try_function(&w.rig, "format_mode", &[d(1.5), d(1.0), d(60.0)]));
    assert!(
        fraction.message.contains("whole numbers of pixels"),
        "{}",
        fraction.message
    );
    assert!(
        set(
            &a,
            Asks {
                position: Some(beside(RIGHT_OF, &b, Some("left"))),
                ..Asks::default()
            }
        )
        .contains("along right_of it must be one of top, center, bottom")
    );
    let side = rec(vec![en(RIGHT_OF), none(), none(), none(), none(), none()]);
    assert!(
        set(
            &a,
            Asks {
                position: Some(side),
                ..Asks::default()
            }
        )
        .contains("position.right_of: must be an OutputProfile or an Output")
    );
    let lonely = rec(vec![en(1), none(), none(), none(), none(), none()]);
    assert!(
        set(
            &a,
            Asks {
                position: Some(lonely),
                ..Asks::default()
            }
        )
        .contains("with whole numbers"),
    );
    // A placement that depends on itself is refused.
    a.set(
        "settings",
        Asks {
            position: Some(beside(RIGHT_OF, &b, None)),
            ..Asks::default()
        }
        .val(),
    );
    let fail = refused(
        b.try_set(
            "settings",
            Asks {
                position: Some(beside(LEFT_OF, &a, None)),
                ..Asks::default()
            }
            .val(),
        ),
    );
    assert!(
        fail.message.contains("placed relative to itself through"),
        "{}",
        fail.message
    );
    assert!(
        matches!(field(&b.get("settings"), 5), Val::None),
        "the refused change left the settings as they were"
    );
}

#[test]
fn a_profile_reads_back_what_it_was_given() {
    let w = world(true);
    let a = w.profile("DP-1");
    let b = w.profile("HDMI-A-1");
    assert_val!(field(&a.get("match"), 0), t("DP-1"));
    assert_val!(field(&a.get("match"), 1), Val::None);
    let asks = Asks {
        mode: Some("preferred"),
        scale: Some(1.25),
        supersample: Some(2.0),
        transform: Some(2),
        vrr: Some(false),
        position: Some(beside(RIGHT_OF, &a, Some("bottom"))),
        enabled: Some(true),
    };
    b.set("settings", asks.val());
    let back = b.get("settings");
    assert_val!(field(&back, 0), t("preferred"));
    assert_val!(field(&back, 1), d(1.25));
    assert_val!(field(&back, 2), d(2.0));
    assert_val!(field(&back, 3), en(2));
    assert_val!(field(&back, 4), truth(false));
    assert_val!(field(&back, 6), truth(true));
    let position = field(&back, 5);
    assert_val!(field(&position, 0), en(RIGHT_OF));
    assert_eq!(
        wrap(&w.rig, &field(&position, 3)).obj,
        a.obj,
        "the neighbour is the profile that was named"
    );
    assert_val!(field(&position, 5), t("bottom"));
    b.set(
        "settings",
        Asks {
            position: Some(at(10, 20)),
            ..Asks::default()
        }
        .val(),
    );
    let position = field(&b.get("settings"), 5);
    assert_val!((field(&position, 1), field(&position, 2)), (Val::Int(10), Val::Int(20)));
    b.set("id", t("second"));
    assert_eq!(b.text("id"), "second");
}

#[test]
fn a_profile_with_a_supersample_sends_it_and_reports_a_factor_that_is_not_recommended_once() {
    let mut w = world(false);
    let noted = listen(&w.rig, &w.manager, "noted");
    let hd = w.profile("DP-1");
    hd.set(
        "settings",
        Asks {
            supersample: Some(3.0),
            ..Asks::default()
        }
        .val(),
    );
    w.manager.call("add_profile", &[hd.val()]);
    w.settle();
    assert_eq!(w.requests(A), vec!["supersample=3"]);
    let sentence = "supersample 3 is not recommended: use 1, 2 or 4";
    assert_eq!(notes(&noted).iter().filter(|line| line.as_str() == sentence).count(), 1);
    // A request is not repeated while the compositor has not answered the one before.
    let mut answered = 0;
    w.answer_all(&mut answered);
    w.settle();
    hd.set(
        "settings",
        Asks {
            supersample: Some(2.0),
            ..Asks::default()
        }
        .val(),
    );
    w.settle();
    assert_eq!(
        w.requests(A).last().map(String::as_str),
        Some("supersample=2"),
        "changing the profile applies the new factor"
    );
    assert_eq!(notes(&noted).iter().filter(|line| line.as_str() == sentence).count(), 1);
}

// ---- cameras ---------------------------------------------------------------------------------------------------------------------------

#[test]
fn a_camera3d_on_a_viewport_is_kept_and_reported_once_as_not_applied_and_a_flat_camera_is_read_back() {
    let mut w = world(true);
    let noted = listen(&w.rig, &w.manager, "noted");
    let display = w.named("display-DP-1").expect("a display");
    let viewport = display.obj("viewport");
    let flat = viewport.obj("camera");
    let cam = w.rig.make("camera3d", &[("fov", d(65.0))]);
    viewport.set("camera", cam.val());
    w.settle();
    let told = |noted: &Rc<RefCell<Vec<Vec<Val>>>>, word: &str| {
        notes(noted)
            .into_iter()
            .filter(|line| line.contains(word))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        told(&noted, "camera3d"),
        vec!["not applied: camera3d (needs a projected workspace)"]
    );
    assert_eq!(viewport.obj("camera").obj, cam.obj, "the value stays readable");
    assert_eq!(viewport.obj("camera").num("fov"), 65.0);
    viewport.set("camera", flat.val());
    viewport.set("camera", cam.val());
    w.settle();
    assert_eq!(told(&noted, "camera3d").len(), 1, "once per generation");
    // A flat camera: the identity is applied by construction and says nothing; the rest is kept and reported.
    assert!(flat.flag("identity"));
    let panned = w.rig.make("camera2d", &[("zoom", d(2.0)), ("roll", d(0.5))]);
    panned.set("x", d(10.0));
    panned.set("y", d(20.0));
    viewport.set("camera", panned.val());
    w.settle();
    assert_eq!(told(&noted, "not applied: camera2d").len(), 1, "{:?}", notes(&noted));
    let now = viewport.obj("camera");
    assert_eq!(
        (now.num("zoom"), now.num("x"), now.num("y"), now.num("roll")),
        (2.0, 10.0, 20.0, 0.5)
    );
    viewport.set("clear_color", t("#101820"));
    w.settle();
    assert_eq!(told(&noted, "clear_color").len(), 1, "{:?}", notes(&noted));
    assert_eq!(viewport.text("clear_color"), "#101820");
}

#[test]
fn a_transition_may_assign_a_display_its_workspace_and_camera_and_a_failed_one_puts_them_back() {
    let mut w = world(true);
    let display = w.named("display-DP-1").expect("a display");
    let viewport = display.obj("viewport");
    let flat = viewport.obj("camera");
    let cam = w.rig.make("camera3d", &[("fov", d(50.0))]);
    let assign = {
        let (viewport, workspace, camera) = (viewport, w.three, cam);
        rig::callback(move |_| {
            viewport.try_set("content", workspace.val())?;
            viewport.try_set("camera", camera.val())?;
            Ok(Val::None)
        })
    };
    request(&w.rig, assign).expect("the transition runs");
    w.settle();
    assert_eq!(w.shown(), "output-0=three output-1=two");
    assert_eq!(viewport.obj("camera").obj, cam.obj);
    assert_eq!(cam.num("fov"), 50.0);
    let failing = {
        let (viewport, workspace, camera) = (viewport, w.one, flat);
        rig::callback(move |_| {
            viewport.try_set("content", workspace.val())?;
            viewport.try_set("camera", camera.val())?;
            Err(Fail::err("no"))
        })
    };
    let fail = refused(request(&w.rig, failing));
    assert!(
        fail.message.contains("the callback failed"),
        "a closure that fails is reported by the library as `the callback failed`: {}",
        fail.message
    );
    w.settle();
    assert_eq!(
        w.shown(),
        "output-0=three output-1=two",
        "the failed transition put the desks back"
    );
    assert_eq!(viewport.obj("camera").obj, cam.obj, "and the camera");
}

// ---- snapshots and what the library tells the manager ---------------------------------------------------------------------------------

#[test]
fn the_manager_says_when_the_lists_change_and_a_snapshot_puts_the_waiting_workspaces_back() {
    let mut w = world(true);
    let changed = listen(&w.rig, &w.manager, "changed");
    let hdmi9 = w.profile("HDMI-9");
    let parked = w.display("parked", vec![profile_selector(&hdmi9)], SINGLE, Some(&w.one));
    w.add(&parked);
    w.settle();
    assert!(!changed.borrow().is_empty(), "adding a display is told");
    let token = w.manager.call("snapshot", &[]).uint().expect("a token");
    parked.obj("viewport").set("content", w.two.val());
    assert_eq!(
        w.content_of(&parked).as_deref(),
        Some("two"),
        "a parked display holds the workspace it was given"
    );
    w.manager.call("restore", &[Val::Uint(token)]);
    w.settle();
    assert_eq!(w.content_of(&parked).as_deref(), Some("one"));
    let before = changed.borrow().len();
    w.manager.call("clear_displays", &[]);
    w.settle();
    assert!(changed.borrow().len() > before, "clearing the displays is told");
    assert!(!parked.flag("listed"));
}
