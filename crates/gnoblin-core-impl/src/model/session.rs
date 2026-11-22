//! `session` and `runtime`: the compositor's session (logout, power, suspend, lock), the configuration reload, and the build identity.
//!
//! Each verb is one call on the compositor's `session` object, made on the owner connection ([`crate::verbs`]), and returns an
//! `operation` that settles when the compositor answers: nothing in a turn waits for logind. `lock` and `reload_config` are not methods of
//! that object, so the connection makes each on a connection of its own. The session's signals (`ready`, `bell`, `reload_requested`,
//! `lock_state_changed`) and the configuration's (`reloaded`, `reload_failed`, which the shell host raises) arrive as events of the host's
//! mailbox and are told from the turn after.
//!
//! `status` is read from the properties the connection keeps, so it answers at once, as of the last thing the compositor said.

use std::path::{Path, PathBuf};

use gnoblin_core_abi::{self as abi, GnoblinCtx};
use gnoblin_model::Fields;
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Fail, Obj, Res, Val};

use super::glue::{self, PerGeneration};
use super::listener::{self, Registered};
use super::operation;
use crate::catalog;
use crate::events::{ShellEvent, SignalDelivery};
use crate::session as host_session;
use crate::values::OwnedValue;
use crate::verbs::OwnerRequest;

/// The build identity field when the sidecar does not supply it.
const UNKNOWN: &str = "unknown";

/// What a reload gave.
struct ReloadedConfig {
    revision: u64,
    not_applied: Vec<String>,
}

/// What the services keep for a generation.
#[derive(Default)]
struct SessionTable {
    session: Obj,
    runtime: Obj,
    hooked: bool,
    session_listeners: Registered,
    runtime_listeners: Registered,
    last_reload: Option<ReloadedConfig>,
}

thread_local! {
    static SESSION: PerGeneration<SessionTable> = PerGeneration::new();
}

/// The code a compositor refusal is reported under: the wire's kebab-case, spelled as the docs spell it.
fn script_code(wire: &str) -> String {
    wire.replace('-', "_")
}

/// Settles a session operation with the answer the compositor gave.
fn settle(cx: &Cx<'_>, op: Obj, outcome: Result<(), String>) {
    match outcome {
        Ok(()) => operation::finish(cx, op, Ok(())),
        Err(code) => {
            let message = format!("the compositor refused the request: {code}");
            operation::finish(cx, op, Err((&script_code(&code), &message)));
        }
    }
}

/// Starts the verb `method` of the session object and returns the operation its answer settles.
fn verb(cx: &Cx<'_>, method: &'static str) -> Res<Val> {
    let (ctx, service) = (cx.ctx(), cx.me());
    hook(cx)?;
    let op = operation::begin(cx)?;
    let sent = glue::owner_call(ctx, "session verb", "session", method, Fields::new(), move |outcome| {
        let _ = crate::object::run_in(ctx, service, |cx| settle(cx, op, outcome));
    });
    if let Err(fail) = sent {
        operation::finish(cx, op, Err(("unavailable", &fail.message)));
    }
    cx.handout(op)
}

/// `lock`: the connection attaches to the lock node and asks for the lock.
fn lock(cx: &Cx<'_>) -> Res<Val> {
    let (ctx, service) = (cx.ctx(), cx.me());
    hook(cx)?;
    let op = operation::begin(cx)?;
    let sent = glue::owner_request(ctx, "session lock", OwnerRequest::Lock, move |outcome| {
        let _ = crate::object::run_in(ctx, service, |cx| match outcome {
            Ok(_) => operation::finish(cx, op, Ok(())),
            Err((code, message)) => operation::finish(cx, op, Err((&script_code(&code), &message))),
        });
    });
    if let Err(fail) = sent {
        operation::finish(cx, op, Err(("unavailable", &fail.message)));
    }
    cx.handout(op)
}

/// `reload_config`.
fn reload_config(cx: &Cx<'_>) -> Res<Val> {
    let (ctx, service) = (cx.ctx(), cx.me());
    if glue::loading(ctx) {
        return Err(Fail::new(
            abi::E_BUSY,
            "gnoblin.runtime.reload_config: the configuration is loading, and each reload would ask for another",
        ));
    }
    hook(cx)?;
    let op = operation::begin(cx)?;
    let sent = glue::owner_request(ctx, "config reload", OwnerRequest::ReloadConfig, move |outcome| {
        let _ = crate::object::run_in(ctx, service, |cx| match outcome {
            Ok(text) => {
                let mut lines = text.lines();
                let revision = lines.next().and_then(|line| line.parse().ok()).unwrap_or(0);
                let reloaded = ReloadedConfig {
                    revision,
                    not_applied: lines.map(str::to_owned).collect(),
                };
                SESSION.with(|session| session.with(ctx, |table| table.last_reload = Some(reloaded)));
                operation::finish(cx, op, Ok(()));
            }
            Err((code, message)) => operation::finish(cx, op, Err((&script_code(&code), &message))),
        });
    });
    if let Err(fail) = sent {
        operation::finish(cx, op, Err(("unavailable", &fail.message)));
    }
    cx.handout(op)
}

/// The session as the compositor last published it.
fn status(cx: &Cx<'_>) -> Res<Val> {
    let opened = host_session::with(|session| session.verbs().is_ok());
    let text = |name: &str| match glue::owner_property("session", name) {
        Some(gnoblin_model::Value::Text(text)) => Some(text),
        _ => None,
    };
    let lock_available = matches!(
        glue::owner_property("session", "lock_available"),
        Some(gnoblin_model::Value::Bool(true))
    );
    let revision = match glue::owner_property("session", "lock_revision") {
        Some(gnoblin_model::Value::Uint(revision)) => revision,
        _ => 0,
    };
    let lock_state = match text("lock_state").filter(|_| lock_available) {
        Some(state) => cx.enum_val("lock_state", &state).unwrap_or(Val::None),
        None => Val::None,
    };
    let state = cx.enum_val("session_state", if opened { "running" } else { "unavailable" })?;
    glue::record(
        cx,
        "session_status",
        vec![
            state,
            Val::Bool(opened && lock_available),
            Val::Uint(if opened { revision } else { 0 }),
            if opened { lock_state } else { Val::None },
        ],
    )
}

// ---- the compositor's signals ------------------------------------------------------------------------------------------------

/// A value the ABI carries as a number.
fn uint(value: Option<&OwnedValue>) -> u64 {
    match value {
        Some(OwnedValue::Uint(value)) => *value,
        Some(OwnedValue::Int(value)) => u64::try_from(*value).unwrap_or(0),
        Some(OwnedValue::Double(value)) => *value as u64,
        _ => 0,
    }
}

fn texts(value: Option<&OwnedValue>) -> Val {
    match value {
        Some(OwnedValue::Texts(items)) => Val::List(items.iter().cloned().map(Val::Text).collect()),
        _ => Val::List(Vec::new()),
    }
}

fn text_of(value: Option<&OwnedValue>) -> Val {
    match value {
        Some(OwnedValue::Text(text)) => Val::Text(text.clone()),
        _ => Val::Text(String::new()),
    }
}

/// What a signal of the session or of the configuration is told as.
fn tell(cx: &Cx<'_>, delivery: &SignalDelivery) {
    let session_class = catalog::class_id("gnoblin.session");
    let config_class = catalog::class_id("gnoblin.config");
    let Some(name) = catalog::signal_name(delivery.class, delivery.signal) else {
        return;
    };
    let values = &delivery.values;
    // Both services exist once a signal is told, so that a listener added to either hears it.
    let (Ok((_, service)), Ok((_, runtime))) = (glue::service(cx.ctx(), "session"), glue::service(cx.ctx(), "runtime"))
    else {
        return;
    };
    cx.unhold(service);
    cx.unhold(runtime);
    if Some(delivery.class) == session_class {
        let me = cx.object_value(service);
        match name.as_str() {
            "ready" => cx.emit_on(service, "ready", vec![me]),
            "bell" => cx.emit_on(service, "bell", vec![me, Val::Uint(uint(values.first()))]),
            "reload_requested" => cx.emit_on(service, "reload_requested", vec![me, Val::Uint(uint(values.first()))]),
            "lock_state_changed" => {
                let state = match values.first() {
                    Some(OwnedValue::Text(state)) => cx.enum_val("lock_state", state),
                    _ => return,
                };
                if let Ok(state) = state {
                    cx.emit_on(
                        service,
                        "lock_state_changed",
                        vec![me, state, Val::Uint(uint(values.get(1)))],
                    );
                }
            }
            _ => {}
        }
    } else if Some(delivery.class) == config_class {
        let me = cx.object_value(runtime);
        match name.as_str() {
            "reloaded" => {
                if let Ok(reload) = glue::record(
                    cx,
                    "config_reloaded",
                    vec![
                        text_of(values.first()),
                        Val::Uint(uint(values.get(1))),
                        texts(values.get(2)),
                        texts(values.get(3)),
                        Val::Uint(uint(values.get(4))),
                        Val::Uint(uint(values.get(5))),
                    ],
                ) {
                    cx.emit_on(runtime, "config_reloaded", vec![me, reload]);
                }
            }
            "reload_failed" => {
                if let Ok(failure) = glue::record(
                    cx,
                    "config_reload_failed",
                    vec![
                        text_of(values.first()),
                        text_of(values.get(1)),
                        Val::Uint(uint(values.get(2))),
                        Val::Uint(uint(values.get(3))),
                    ],
                ) {
                    cx.emit_on(runtime, "config_reload_failed", vec![me, failure]);
                }
            }
            _ => {}
        }
    }
}

/// Listens for the signals of the session and of the configuration, once for the generation.
fn hook(cx: &Cx<'_>) -> Res<()> {
    let ctx = cx.ctx();
    let fresh = SESSION.with(|session| {
        session.with(ctx, |table| {
            let fresh = !table.hooked;
            table.hooked = true;
            fresh
        })
    });
    if !fresh {
        return Ok(());
    }
    let service = cx.me();
    let classes = [
        catalog::class_id("gnoblin.session"),
        catalog::class_id("gnoblin.config"),
    ];
    glue::hear(
        ctx,
        "session signals",
        move |event| matches!(event, ShellEvent::Signal(delivery) if classes.contains(&Some(delivery.class))),
        move |event| {
            if let ShellEvent::Signal(delivery) = event {
                let delivery = delivery.clone();
                let _ = crate::object::run_in(ctx, service, |cx| tell(cx, &delivery));
            }
        },
    )?;
    Ok(())
}

/// Adds or removes a listener of the session or of the runtime.
fn listen(cx: &Cx<'_>, runtime: bool, listener_val: &Val, add: bool) -> Res<Val> {
    let (ctx, me) = (cx.ctx(), cx.me());
    let mut regs = SESSION.with(|session| {
        session.with(ctx, |table| {
            std::mem::take(if runtime {
                &mut table.runtime_listeners
            } else {
                &mut table.session_listeners
            })
        })
    });
    let done = if add {
        listener::add(cx, me, &mut regs, listener_val).map(|_| ())
    } else {
        listener::remove(cx, &mut regs, listener_val)
    };
    SESSION.with(|session| {
        session.with(ctx, |table| {
            if runtime {
                table.runtime_listeners = regs;
            } else {
                table.session_listeners = regs;
            }
        });
    });
    done?;
    if add {
        hook(cx)?;
    }
    Ok(Val::None)
}

// ---- the build identity ------------------------------------------------------------------------------------------------------

/// The `[version]` values of a `version.ini` text. Other sections, blank lines and `#` or `;` comments are ignored.
fn version_values(text: &str) -> Vec<(String, String)> {
    let mut inside = false;
    let mut values = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            inside = section.trim() == "version";
        } else if inside && let Some((key, value)) = line.split_once('=') {
            values.push((key.trim().to_owned(), value.trim().to_owned()));
        }
    }
    values
}

/// A git remote with its credentials, query and fragment removed. A URL keeps its scheme, host, port and path; the scp-like spelling has
/// no scheme, so what precedes its first `@` goes.
fn public_remote(remote: &str) -> String {
    if remote.is_empty() {
        return UNKNOWN.to_owned();
    }
    let Some((scheme, rest)) = remote.split_once("://") else {
        return remote.split_once('@').map_or(remote, |(_, host)| host).to_owned();
    };
    let (authority, path) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let path = path.split(['?', '#']).next().unwrap_or("");
    format!("{scheme}://{host}{path}")
}

/// How many directories above the library the prefix may be: `<prefix>/<libdir>/gnoblin/<library>` is two above its directory, and a
/// multiarch libdir (`lib/x86_64-linux-gnu`) makes it three.
const PREFIX_SEARCH_DEPTH: usize = 4;

/// Where the sidecar may be, best first: the override, then beside the prefix of the library this code runs in (every ancestor that holds
/// the file is a candidate, so a relocated prefix and a multiarch libdir both work), then beside the executable's prefix, then the data
/// directories.
fn sidecar_paths(
    override_path: Option<&str>,
    library: Option<&Path>,
    executable: Option<&Path>,
    data_dirs: Option<&str>,
) -> Vec<PathBuf> {
    const SIDECAR: &str = "share/gnoblin/version.ini";
    let mut paths: Vec<PathBuf> = override_path
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .into_iter()
        .collect();
    if let Some(library) = library {
        paths.extend(
            library
                .ancestors()
                .skip(1)
                .take(PREFIX_SEARCH_DEPTH)
                .filter(|ancestor| ancestor.parent().is_some())
                .map(|ancestor| ancestor.join(SIDECAR)),
        );
    }
    if let Some(prefix) = executable.and_then(Path::parent).and_then(Path::parent) {
        paths.push(prefix.join(SIDECAR));
    }
    let dirs = data_dirs
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or("/usr/local/share/:/usr/share/");
    paths.extend(
        dirs.split(':')
            .filter(|dir| !dir.is_empty())
            .map(|dir| Path::new(dir).join("gnoblin/version.ini")),
    );
    paths
}

/// The build identity from the sidecar's text: a field it does not give is `unknown`.
fn version_fields(sidecar: &str, engine: &str) -> Vec<String> {
    let values = version_values(sidecar);
    let field = |key: &str| {
        values
            .iter()
            .find(|(name, value)| name == key && !value.is_empty())
            .map_or_else(|| UNKNOWN.to_owned(), |(_, value)| value.clone())
    };
    let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
    vec![
        field("gnoblin"),
        field("gnome"),
        field("mutter"),
        engine.to_owned(),
        format!("{major}.{minor}.{patch}"),
        public_remote(
            values
                .iter()
                .find(|(name, _)| name == "git_remote")
                .map_or("", |(_, value)| value.as_str()),
        ),
        field("git_sha"),
        field("build_id"),
    ]
}

fn read_sidecar() -> String {
    let env = |name: &str| std::env::var(name).ok();
    let executable = std::env::current_exe().ok();
    let library = gnoblin_shell_host::runtime::library_containing(read_sidecar as fn() -> String as *const ());
    let paths = sidecar_paths(
        env("GNOBLIN_VERSION_METADATA_FILE").as_deref(),
        library.as_deref(),
        executable.as_deref(),
        env("XDG_DATA_DIRS").as_deref(),
    );
    paths
        .iter()
        .find_map(|path| std::fs::read(path).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

/// The `gnoblin.version` record of the generation `cx` is in.
pub(super) fn version(cx: &Cx<'_>) -> Res<Val> {
    let engine = match crate::registry::ctx_info(cx.ctx()) {
        Ok(info) if info.language == abi::LANG_JS || info.language == abi::LANG_TS => "quickjs-ng",
        _ => UNKNOWN,
    };
    glue::record(
        cx,
        "build_version",
        version_fields(&read_sidecar(), engine)
            .into_iter()
            .map(Val::Text)
            .collect(),
    )
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<()>("compositor_settings")
        .ctor(|_, _| Ok(()))
        .get("user_shaders", |_, cx| Ok(Val::Bool(super::input::user_shaders(cx))));
    let _ = builder
        .class::<()>("session")
        .ctor(|cx, _| {
            let me = cx.me();
            SESSION.with(|session| session.with(cx.ctx(), |table| table.session = me));
            hook(cx)?;
            Ok(())
        })
        .get("status", |_, cx| status(cx))
        .method("logout", |_, cx, _| verb(cx, "logout"))
        .method("poweroff", |_, cx, _| verb(cx, "poweroff"))
        .method("reboot", |_, cx, _| verb(cx, "reboot"))
        .method("suspend", |_, cx, _| verb(cx, "suspend"))
        .method("hibernate", |_, cx, _| verb(cx, "hibernate"))
        .method("lock", |_, cx, _| lock(cx))
        .method("add_listener", |_, cx, args| listen(cx, false, &args[0], true))
        .method("remove_listener", |_, cx, args| listen(cx, false, &args[0], false));
    let _ = builder
        .class::<()>("runtime")
        .ctor(|cx, _| {
            let me = cx.me();
            SESSION.with(|session| session.with(cx.ctx(), |table| table.runtime = me));
            hook(cx)?;
            Ok(())
        })
        .get("last_reload", |_, cx| {
            let last = SESSION.with(|session| {
                session.with(cx.ctx(), |table| {
                    table
                        .last_reload
                        .as_ref()
                        .map(|reloaded| (reloaded.revision, reloaded.not_applied.clone()))
                })
            });
            match last {
                Some((revision, not_applied)) => glue::record(
                    cx,
                    "reload_result",
                    vec![
                        Val::Uint(revision),
                        Val::List(Vec::new()),
                        Val::List(not_applied.into_iter().map(Val::Text).collect()),
                    ],
                ),
                None => Ok(Val::None),
            }
        })
        .method("reload_config", |_, cx, _| reload_config(cx))
        .method("add_listener", |_, cx, args| listen(cx, true, &args[0], true))
        .method("remove_listener", |_, cx, args| listen(cx, true, &args[0], false));
}

/// The generation ended: its table goes.
pub(super) fn forget(ctx: GnoblinCtx) {
    SESSION.with(|session| session.take(ctx));
}
