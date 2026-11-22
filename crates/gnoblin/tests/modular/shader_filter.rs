//! The custom shader pass through the std library, in a real session: a `ShaderFilter` of a Rust, a TypeScript and a Python shell reaches the
//! window's node in the compositor's tree as a `shader` effect, and every way it can be refused is what the user sees.
//!
//! The shell is the shipped binary on a headless backend with a Wayland client that stays connected, and what is asserted is what the compositor's
//! tree holds (read over the socket) and what the session logs, not what the shell believes. `compositor.user-shaders` is on by default and is a
//! setting of the accepted configuration document, read when the compositor starts. A user switches it off the way a shell supplies any document
//! setting: a TypeScript `shell.ts` sets `gnoblin.compositor.userShaders = false`, the shell commits it with its document, the compositor keeps it
//! as the last accepted document, and the next login reads it. The TypeScript tests below do exactly that (a first login that commits, a second
//! login over the same home). A Rust or a Python shell commits no document of its own, so the test of a Rust shell with shaders switched off starts
//! over a last-accepted document that a TypeScript configuration committed earlier, written to the place the binary reads it (`XDG_STATE_HOME`).
//!
//! What this cannot prove is a pixel: a headless session draws with the software renderer, which has no shader compiler and lists the pass as
//! skipped. The renderer lanes prove the drawing; the tests here assert the tree, the listing and the refusals. If the headless renderer comes to
//! draw shaders, the seat says so (`effect_kinds`) and the same tests assert that nothing is listed skipped.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use gnoblin_config::{Config, Store, Value};

use crate::support::native_shell::{
    Gnoblin, Home, Launch, Typist, script_environment, staged_library_path, std_shells,
};
use crate::support::{Mapper, snapshot};
use crate::{repo_root, save, serving};

/// The evdev code of `w`.
const KEY_W: u32 = 17;

/// The tint of the three examples, as their `params` say: orange, a quarter of the way.
const TINT_PARAMS: [f64; 4] = [1.0, 0.55, 0.2, 0.25];

/// A shader the validator refuses, as the body of a JavaScript string: `loop` is not a bounded `for`.
const LOOPING_JS: &str = "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {\\n\
    var a = 0.0;\\n\
    loop {\\n\
        a = a + 1.0;\\n\
        if a > 3.0 { break; }\\n\
    }\\n\
    return c * a;\\n\
}\\n";

/// A shader the validator accepts, as the body of a JavaScript string.
const WORKING_JS: &str = "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {\\n\
    return c * 0.9;\\n\
}\\n";

/// A home whose last accepted configuration document says `compositor.user-shaders = user_shaders`, for the configuration file `config_text`
/// (`shell.ts` of the configuration directory, which the binary discovers). Returns the home and that file's path.
fn home_with(user_shaders: bool, config_text: &str) -> (Home, PathBuf) {
    home_with_document(
        serde_json::json!({"compositor": {"user-shaders": user_shaders}}),
        config_text,
    )
}

/// A home whose last accepted configuration document is `document`, for the configuration file `config_text` (`shell.ts` of the configuration
/// directory, which the binary discovers). Returns the home and that file's path.
pub(crate) fn home_with_document(document: serde_json::Value, config_text: &str) -> (Home, PathBuf) {
    let home = Home::new();
    let source = home.write("config/gnoblin/shell.ts", config_text);
    let state = home.root().join("state");
    fs::create_dir(&state).expect("the state directory");
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).expect("the state mode");
    let document = Value::from_json(&document).expect("a document");
    Store::new(&state)
        .save(&source, &Config::from_value(document).expect("a valid document"))
        .expect("the last accepted document is stored");
    (home, source)
}

/// A session that runs the script at `shell`, in `home`.
fn start_script(home: Home, shell: &Path) -> Gnoblin {
    let mut launch = Launch::compositor(&[]).with_arg(shell);
    for (name, value) in script_environment() {
        launch = launch.with_env(name, &value);
    }
    let mut session = Gnoblin::launch(home, launch);
    serving(&mut session);
    session
}

/// A session that runs the native shell at `shell`, in `home`.
fn start_native(home: Home, shell: &Path) -> Gnoblin {
    let launch = Launch::compositor(&[])
        .with_arg(shell)
        .with_env("LD_LIBRARY_PATH", &staged_library_path());
    let mut session = Gnoblin::launch(home, launch);
    session.wait_serving();
    session.wait_log("the native shell serves in process", |line| {
        line.contains("serves in process")
    });
    session
}

/// The last accepted document of the session at `ipc`, as JSON, once it holds one.
fn accepted_document(ipc: &Path) -> Option<serde_json::Value> {
    let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
    let transport = gnoblin_client::unix::UnixTransport::connect(ipc).ok()?;
    let mut client =
        gnoblin_client::Client::connect(transport, gnoblin_proto::Version::new(major, minor, patch)).ok()?;
    let (_, bytes) = client.configuration().ok()??;
    serde_json::from_slice(&bytes).ok()
}

/// A second login over a TypeScript `shell.ts` that switches `compositor.user-shaders` off with `gnoblin.compositor.userShaders = false`: the
/// first login commits the setting, which the compositor accepts and keeps without applying (it reads the setting when it starts), and the
/// second one starts with it. Returns the first session, which has run to the commit, and the path of `shell.ts`; the caller starts the second
/// from `first.into_home()`.
fn first_login_that_switches_user_shaders_off(shell_text: &str) -> (Gnoblin, PathBuf) {
    let home = Home::new();
    let source = home.write("config/gnoblin/shell.ts", shell_text);
    let mut first = start_script(home, &source);
    let ipc = first.ipc();
    first.wait_for("the first login commits compositor.user-shaders = false", |_| {
        accepted_document(&ipc).is_some_and(|document| document["compositor"]["user-shaders"] == false)
    });
    (first, source)
}

/// One entry of a node's `effects` list: `(kind, source, params)`.
type Held = (String, String, Vec<f64>);

/// The effects the window node of `app` holds, or `None` while the window has no node.
fn effects_of(ipc: &Path, app: &str) -> Option<Vec<Held>> {
    let node = snapshot(ipc)
        .into_iter()
        .find(|node| node.class == "window" && node.app_id == app)?;
    Some(
        node.props
            .iter()
            .find(|prop| prop.key == "effects")
            .map(|prop| {
                prop.value_effects
                    .iter()
                    .map(|effect| (effect.kind.clone(), effect.source.clone(), effect.params.clone()))
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// What the compositor lists as undrawn on the window node of `app`.
fn skipped_of(ipc: &Path, app: &str) -> Vec<String> {
    snapshot(ipc)
        .into_iter()
        .find(|node| node.class == "window" && node.app_id == app)
        .and_then(|node| {
            node.props
                .iter()
                .find(|prop| prop.key == "skipped")
                .map(|prop| prop.value_texts.clone())
        })
        .unwrap_or_default()
}

/// Whether the active renderer draws the shader kind: the seat lists the kinds it draws.
fn renderer_draws_shaders(ipc: &Path) -> bool {
    snapshot(ipc).iter().any(|node| {
        node.class == "seat"
            && node
                .props
                .iter()
                .any(|prop| prop.key == "effect_kinds" && prop.value_texts.iter().any(|kind| kind == "shader"))
    })
}

fn shaders(held: &[Held]) -> Vec<&Held> {
    held.iter().filter(|(kind, _, _)| kind == "shader").collect()
}

/// A window of `app` has been mapped, the library has written its style, and the node holds one shader effect that is the example's tint.
#[track_caller]
fn assert_tinted(session: &mut Gnoblin, mapper: &mut Mapper, app: &str) {
    let ipc = session.ipc();
    mapper.map_window(app, "Tinted");
    session.wait_for("the shader reaches the window's node", |_| {
        effects_of(&ipc, app).is_some_and(|held| shaders(&held).len() == 1)
    });
    let on_node = effects_of(&ipc, app).expect("the node");
    let (_, source, params) = shaders(&on_node)[0].clone();
    assert!(
        source.contains("fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32)"),
        "the node holds the shell's source: {source}"
    );
    assert!(
        params.len() == TINT_PARAMS.len()
            && params
                .iter()
                .zip(TINT_PARAMS)
                .all(|(got, wanted)| (got - wanted).abs() < 1e-6),
        "the node holds the shell's numbers: {params:?}"
    );
    // A renderer that draws shaders lists nothing; one that does not (the software renderer of a headless session) says so on the node.
    if renderer_draws_shaders(&ipc) {
        assert!(skipped_of(&ipc, app).is_empty(), "{:?}", skipped_of(&ipc, app));
    } else {
        session.wait_for("the node lists the shader as undrawn by this renderer", |_| {
            skipped_of(&ipc, app) == ["shader: unsupported-by-renderer"]
        });
    }
    assert!(
        session.log_text().lines().all(|line| !line.contains("ERROR")),
        "{}",
        session.log_text()
    );
}

#[test]
fn a_typescript_shell_tints_a_window_with_a_shader_filter_by_default() {
    let shell = repo_root().join("examples/19-shader-tint/shell.ts");
    let mut session = start_script(Home::new(), &shell);
    let mut mapper = Mapper::connect(&session.wayland());
    assert_tinted(&mut session, &mut mapper, "tint.ts");
}

#[test]
fn a_rust_shell_tints_a_window_with_a_shader_filter_by_default() {
    let mut session = start_native(Home::new(), &std_shells().shader_tint);
    let mut mapper = Mapper::connect(&session.wayland());
    assert_tinted(&mut session, &mut mapper, "tint.rs");
}

#[test]
fn a_python_shell_tints_a_window_with_a_shader_filter_by_default() {
    let shell = repo_root().join("examples/19-shader-tint/shell.py");
    let mut session = start_script(Home::new(), &shell);
    let mut mapper = Mapper::connect(&session.wayland());
    assert_tinted(&mut session, &mut mapper, "tint.py");
}

#[test]
fn a_typescript_config_that_switches_user_shaders_off_has_them_refused_from_the_next_login() {
    let example = fs::read_to_string(repo_root().join("examples/19-shader-tint/shell.ts")).expect("the example");
    let text = format!("{example}\ngnoblin.compositor.userShaders = false;\n");
    let (mut first, source) = first_login_that_switches_user_shaders_off(&text);
    // The first login: this compositor started with the default, so the shader is drawn while the setting is committed and kept.
    let mut mapper = Mapper::connect(&first.wayland());
    assert_tinted(&mut first, &mut mapper, "first.ts");
    drop(mapper);

    // The second login reads it: the shader is not written, the library says why, and the compositor was never asked.
    let mut second = start_script(first.into_home(), &source);
    let ipc = second.ipc();
    let mut mapper = Mapper::connect(&second.wayland());
    mapper.map_window("second.ts", "Second");
    let said = second.wait_log("the library says why the shader is not drawn", |line| {
        line.contains("not applied: style.filter: a ShaderFilter is not drawn")
    });
    assert!(said.text.contains("compositor.user-shaders"), "{}", said.text);
    mapper.roundtrip();
    assert!(effects_of(&ipc, "second.ts").is_none_or(|held| shaders(&held).is_empty()));
    assert!(
        !second.log_text().contains("an effect was refused"),
        "nothing was sent for the compositor to refuse:\n{}",
        second.log_text()
    );
}

/// A TypeScript shell that gives every window a shadow and a shader (`shader`, WGSL as a JavaScript string).
fn shadow_and_shader_shell(shader: &str) -> String {
    format!(
        "import {{ gnoblin }} from \"gnoblin-core\";\n\
         import {{ ShaderFilter }} from \"gnoblin-std\";\n\
         \n\
         gnoblin.clients.style.shadows = [{{ y: 8, blur: 24, color: \"#00000066\" }}];\n\
         gnoblin.clients.style.filter = [new ShaderFilter({{ source: \"{shader}\", params: [0.5] }})];\n"
    )
}

#[test]
fn with_the_setting_switched_off_a_shader_filter_is_not_written_and_the_rest_of_the_style_is() {
    let off = format!(
        "{}\ngnoblin.compositor.userShaders = false;\n",
        shadow_and_shader_shell(WORKING_JS)
    );
    let (first, source) = first_login_that_switches_user_shaders_off(&off);
    let mut session = start_script(first.into_home(), &source);
    let ipc = session.ipc();
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("off.ts", "Off");
    // The shadow is the style's other half: it arrives, and the shader never does.
    session.wait_for("the shadow reaches the window's node", |_| {
        effects_of(&ipc, "off.ts").is_some_and(|held| held.iter().any(|(kind, _, _)| kind == "shadow"))
    });
    assert!(
        shaders(&effects_of(&ipc, "off.ts").expect("the node")).is_empty(),
        "the library does not ask for a shader the compositor refuses"
    );
    let said = session.wait_log("the library says why the shader is not drawn", |line| {
        line.contains("not applied: style.filter: a ShaderFilter is not drawn")
    });
    assert!(
        said.text.contains("compositor.user-shaders"),
        "the message names the setting: {}",
        said.text
    );
    assert!(
        !session.log_text().contains("an effect was refused"),
        "nothing was sent for the compositor to refuse:\n{}",
        session.log_text()
    );
}

#[test]
fn with_the_setting_switched_off_a_rust_shell_reads_why_from_the_style_warnings() {
    // What a TypeScript configuration committed earlier: `compositor.user-shaders = false`.
    let (home, _) = home_with(false, "// the settings file; the shell is the library\n");
    let mut session = start_native(home, &std_shells().shader_tint);
    let ipc = session.ipc();
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("off.rs", "Off");
    let mut typist = Typist::connect(&session.wayland());
    // The chord reports the windows' warnings; a window that is not yet styled reports none, so it is pressed until it does.
    let said = {
        let mut found = None;
        session.wait_for("the style warnings name the shader filter", |session| {
            typist.super_plus(KEY_W);
            found = session.log().into_iter().find(|line| {
                line.text
                    .contains("shader tint: not applied: style.filter: a ShaderFilter is not drawn")
            });
            found.is_some()
        });
        found.expect("the wait ended with the line")
    };
    assert!(said.text.contains("compositor.user-shaders"), "{}", said.text);
    assert!(
        effects_of(&ipc, "off.rs").is_some_and(|held| shaders(&held).is_empty()),
        "no shader reaches the node"
    );
}

#[test]
fn a_shader_the_compositor_refuses_leaves_the_windows_effects_as_they_were_and_the_log_says_where() {
    let home = Home::new();
    let source = home.write("config/gnoblin/shell.ts", &shadow_and_shader_shell(LOOPING_JS));
    let mut session = start_script(home, &source);
    let ipc = session.ipc();
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("bad.ts", "Bad");
    let refused = session.wait_log("the compositor refuses the shader and says where", |line| {
        line.contains("an effect was refused") && line.contains("shader pass line")
    });
    assert!(refused.text.contains("column"), "{}", refused.text);
    // The refusal is the whole list's: the shadow written beside the shader is not on the node either, which is what a shell is told to expect.
    assert_eq!(
        effects_of(&ipc, "bad.ts"),
        Some(Vec::new()),
        "a refused list applies none of its effects"
    );

    // A source that validates is accepted by the next load, and the shadow and the shader arrive together.
    save(&source, &shadow_and_shader_shell(WORKING_JS));
    session.wait_for("the corrected shell's style reaches the node", |_| {
        effects_of(&ipc, "bad.ts")
            .is_some_and(|held| shaders(&held).len() == 1 && held.iter().any(|(kind, _, _)| kind == "shadow"))
    });
}
