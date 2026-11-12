//! What a running session does when a setting in the accepted document changes.
//!
//! Every setting the schema declares has one rule here: it is applied [`Applies::Live`] by the
//! process that owns it, it is read only when a process starts ([`Applies::Restart`]), or
//! nothing reads it at all ([`Applies::Unused`]). `design/config-reload.md` carries the same
//! table with the evidence for each row, and `tests/apply.rs` fails when a setting has no rule,
//! a rule names a setting the schema lacks, or the design table and the rules disagree.
//!
//! A reload that changes a setting a running process only reads at start must say so: the
//! document is accepted and the old behaviour continues, and a user who is not told would
//! think the reload applied it. [`restart_required`] and [`no_effect`] name those settings.
//!
//! # Which document a process started with
//!
//! A setting read at start is compared with the document the process read, not with the
//! previous reload: a setting changed and changed back is not pending, and one changed twice is
//! reported once. The compositor reads the last accepted file when it starts, which is the
//! previous session's document, so an edit made between two logins is reported on the first
//! commit. The processes the guardian starts after the first commit (the services role, the
//! compatibility host) read what that commit saved.

use crate::document::Config;
use crate::generated::SETTINGS;
use crate::model::Default;
use crate::value::Value;

/// A process that reads a setting once, when it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Process {
    /// The compositor, which reads the last accepted document before the runtime commits.
    Compositor,
    /// The services role: the Secret Service, the polkit agent and the location agent. It
    /// starts after the runtime's first commit.
    Services,
    /// The compatibility host, which serves the GNOME shell introspection interface. It
    /// starts after the runtime's first commit.
    CompatHost,
}

impl Process {
    /// The words a reload report uses for the process.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Process::Compositor => "the compositor",
            Process::Services => "the services role",
            Process::CompatHost => "the compatibility host",
        }
    }

    /// Whether the process reads the document the session's first commit saved, rather than the
    /// one the compositor started with.
    #[must_use]
    pub const fn starts_after_first_commit(self) -> bool {
        !matches!(self, Process::Compositor)
    }
}

/// What happens to a running session when the setting changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applies {
    /// The owning process applies the new value as soon as the document is accepted.
    Live,
    /// The process reads the value once, when it starts; the change waits for the next start.
    Restart(Process),
    /// Nothing reads the setting. The document accepts it and no behaviour follows.
    Unused,
}

/// The rule for one setting, or for a whole section whose keys are entries (`key` is `*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The top-level section.
    pub section: &'static str,
    /// The key inside it, or `*`.
    pub key: &'static str,
    /// What a change does.
    pub applies: Applies,
    /// Why, in the words a user sees when the reload reports it.
    pub detail: &'static str,
}

const fn live(section: &'static str, key: &'static str) -> Rule {
    Rule {
        section,
        key,
        applies: Applies::Live,
        detail: "",
    }
}

const fn restart(section: &'static str, key: &'static str, process: Process, detail: &'static str) -> Rule {
    Rule {
        section,
        key,
        applies: Applies::Restart(process),
        detail,
    }
}

const fn unused(section: &'static str, key: &'static str, detail: &'static str) -> Rule {
    Rule {
        section,
        key,
        applies: Applies::Unused,
        detail,
    }
}

/// One rule per declared setting, in the schema's section order.
pub const RULES: &[Rule] = &[
    unused("compositor", "debug", "nothing reads it yet"),
    unused("compositor", "locate-pointer", "nothing reads it yet"),
    unused("compositor", "locate-pointer-key", "nothing reads it yet"),
    live("compositor", "visual-bell"),
    unused("compositor", "audible-bell", "nothing reads it yet"),
    live("compositor", "visual-bell-type"),
    unused("compositor", "blur", "nothing reads it yet"),
    restart(
        "compositor",
        "renderer",
        Process::Compositor,
        "the renderer is created when the compositor starts",
    ),
    live("window-management", "focus-mode"),
    live("window-management", "focus-new-windows"),
    live("window-management", "raise-on-click"),
    live("window-management", "auto-raise"),
    live("window-management", "auto-raise-delay"),
    live("window-management", "focus-change-on-pointer-rest"),
    restart(
        "window-management",
        "check-alive-timeout",
        Process::Compositor,
        "the liveness checker is created when the compositor starts",
    ),
    live("window-management", "edge-tiling"),
    unused("window-management", "attach-modal-dialogs", "nothing reads it yet"),
    unused("window-management", "disable-workarounds", "nothing reads it yet"),
    live("window-management", "constrain-drag-to-work-area"),
    live("window-management", "action-double-click-titlebar"),
    live("window-management", "action-middle-click-titlebar"),
    live("window-management", "action-right-click-titlebar"),
    live("cursor", "theme"),
    live("cursor", "size"),
    live("cursor", "providers"),
    live("appearance", "color-scheme"),
    live("appearance", "accent"),
    live("appearance", "reduced-motion"),
    live("appearance", "high-contrast"),
    unused("input", "orientation-lock", "no device applies it yet"),
    unused("input", "mouse", "no device applies the libinput settings yet"),
    unused("input", "touchpad", "no device applies the libinput settings yet"),
    unused("input", "trackball", "no device applies the libinput settings yet"),
    unused("input", "pointing-stick", "no device applies the libinput settings yet"),
    live("input", "keyboard"),
    unused("input", "tablets", "no device applies the tablet settings yet"),
    unused("input", "styluses", "no device applies the stylus settings yet"),
    live("input-sources", "sources"),
    live("input-sources", "per-window"),
    live("accessibility", "magnifier"),
    live("accessibility", "magnifier-zoom"),
    live("accessibility", "colour-filter"),
    restart(
        "protocols",
        "*",
        Process::Compositor,
        "the advertised protocols are chosen when the compositor starts, and a protocol that was switched off cannot be advertised again to clients that already hold the registry",
    ),
    unused("layer-shell", "preserve-active-window", "nothing reads it yet"),
    restart(
        "keyring",
        "enabled",
        Process::Services,
        "the services role decides whether to serve the Secret Service when it starts",
    ),
    unused("auth", "polkit-agent", "nothing reads it yet"),
    restart(
        "location",
        "enabled",
        Process::Services,
        "the location agent reads its policy when the services role starts",
    ),
    restart(
        "location",
        "max-accuracy",
        Process::Services,
        "the location agent reads its policy when the services role starts",
    ),
    restart(
        "compat",
        "capture",
        Process::CompatHost,
        "the compatibility host reads the capture mode when it starts",
    ),
    restart(
        "capture",
        "secure-treatment",
        Process::Compositor,
        "the backend's renderer is built with the secure-layer treatment when the compositor starts",
    ),
    unused("monitors", "privacy-screen", "no output applies it yet"),
    live("monitors", "global-scale"),
    live("xwayland", "allow-grabs"),
    live("xwayland", "allow-byte-swapped-clients"),
    live("xwayland", "grab-access-rules"),
    live("xwayland", "disable-extensions"),
    live("xwayland", "scaling-factor"),
    restart(
        "workspaces",
        "*",
        Process::Compositor,
        "the workspaces are published to the shell when the compositor starts, and the stock shell builds its desks from them once",
    ),
    restart(
        "touchpad-gestures",
        "*",
        Process::Compositor,
        "the gesture table is built when the compositor starts",
    ),
    unused("animations", "*", "nothing reads the animations section"),
    restart(
        "logging",
        "console",
        Process::Compositor,
        "the log sinks are opened when the process starts",
    ),
    restart(
        "logging",
        "level",
        Process::Compositor,
        "the log filter is installed when the process starts",
    ),
    restart(
        "logging",
        "file",
        Process::Compositor,
        "the log sinks are opened when the process starts",
    ),
    restart(
        "logging",
        "dir",
        Process::Compositor,
        "the log sinks are opened when the process starts",
    ),
    restart(
        "logging",
        "max-size-mb",
        Process::Compositor,
        "the log sinks are opened when the process starts",
    ),
    live("autostart", "*"),
    unused("keybindings", "*", "nothing reads the keybindings section"),
    unused("portals", "*", "nothing reads the portals section"),
    restart(
        "shortcuts",
        "*",
        Process::Compositor,
        "the shortcut table is built when the compositor starts",
    ),
];

/// The rule for `section.key`.
#[must_use]
pub fn rule_of(section: &str, key: &str) -> Option<&'static Rule> {
    RULES.iter().find(|rule| rule.section == section && rule.key == key)
}

/// Whether a reported setting waits for a restart or has no effect at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingKind {
    /// The setting is read when `Process` starts, so it takes effect at the next start.
    RestartRequired(Process),
    /// Nothing reads the setting.
    NoEffect,
}

/// One setting a reload accepted without applying it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    /// The setting, spelled as in the document: `renderer` of `compositor` is
    /// `compositor.renderer`, and an entry of a section of entries is `protocols.wlr-layer-shell`.
    pub setting: String,
    /// Why it is not applied.
    pub kind: PendingKind,
    /// The reason, in a sentence a user can read.
    pub detail: &'static str,
}

impl Pending {
    /// The line a user sees, for example
    /// `compositor.renderer: restart required, the renderer is created when the compositor starts`.
    #[must_use]
    pub fn describe(&self) -> String {
        match self.kind {
            PendingKind::RestartRequired(process) => format!(
                "{}: restart required ({} reads it when it starts): {}",
                self.setting,
                process.name(),
                self.detail
            ),
            PendingKind::NoEffect => format!("{}: no effect, {}", self.setting, self.detail),
        }
    }
}

/// The settings of `process` that `now` sets differently from `started_with`, the document the
/// process read when it started.
#[must_use]
pub fn restart_required(started_with: &Config, now: &Config, process: Process) -> Vec<Pending> {
    changes(started_with, now, |rule| match rule.applies {
        Applies::Restart(owner) if owner == process => Some(PendingKind::RestartRequired(owner)),
        _ => None,
    })
}

/// The settings nothing reads that `now` sets differently from `previous`, the document
/// accepted before it.
#[must_use]
pub fn no_effect(previous: &Config, now: &Config) -> Vec<Pending> {
    changes(previous, now, |rule| {
        matches!(rule.applies, Applies::Unused).then_some(PendingKind::NoEffect)
    })
}

/// Every setting the selected rules cover that differs between the two documents, in rule order.
fn changes(from: &Config, to: &Config, select: impl Fn(&Rule) -> Option<PendingKind>) -> Vec<Pending> {
    let mut found = Vec::new();
    for rule in RULES {
        let Some(kind) = select(rule) else {
            continue;
        };
        let default = default_of(rule);
        let before = lookup(from.document(), rule);
        let after = lookup(to.document(), rule);
        for setting in differing(rule, before, after, default.as_ref()) {
            found.push(Pending {
                setting,
                kind,
                detail: rule.detail,
            });
        }
    }
    found
}

/// The value `section.key` has in `document`, if the document sets it. For `*` it is the whole
/// section.
fn lookup<'a>(document: &'a Value, rule: &Rule) -> Option<&'a Value> {
    let section = document.as_table()?.get(rule.section)?;
    if rule.key == "*" {
        return Some(section);
    }
    section.as_table()?.get(rule.key)
}

/// The value an unset setting behaves as, from the schema, so an explicit default and an absent
/// key are the same thing.
fn default_of(rule: &Rule) -> Option<Value> {
    let setting = SETTINGS
        .iter()
        .find(|setting| setting.section == rule.section && setting.key == rule.key)?;
    match setting.default {
        Default::None => None,
        Default::Bool(value) => Some(Value::Bool(value)),
        Default::Integer(value) => Some(Value::Integer(value)),
        Default::Float(value) => Some(Value::Float(value)),
        Default::Text(text) | Default::Enum(text) => Some(Value::String(text.to_owned())),
        Default::List(items) => Some(Value::List(
            items.iter().map(|item| Value::String((*item).to_owned())).collect(),
        )),
    }
}

/// The value a setting behaves as: an empty list or table is the same as nothing, and nothing
/// is the schema's default.
fn unset<'a>(value: Option<&'a Value>, default: Option<&'a Value>) -> Option<&'a Value> {
    match value {
        Some(Value::List(items)) if items.is_empty() => default,
        Some(Value::Map(entries)) if entries.is_empty() => default,
        other => other.or(default),
    }
}

/// The names of the settings under `rule` that differ between `before` and `after`.
///
/// A section of entries reports each entry that was added, removed or changed, so a change to
/// one protocol names that protocol and not the whole table. A list, such as the workspaces, is
/// one setting.
fn differing(rule: &Rule, before: Option<&Value>, after: Option<&Value>, default: Option<&Value>) -> Vec<String> {
    if rule.key == "*"
        && let (None | Some(Value::Map(_)), None | Some(Value::Map(_))) = (before, after)
    {
        let empty = std::collections::BTreeMap::new();
        let old = before.and_then(Value::as_table).unwrap_or(&empty);
        let new = after.and_then(Value::as_table).unwrap_or(&empty);
        let names: std::collections::BTreeSet<&String> = old.keys().chain(new.keys()).collect();
        return names
            .into_iter()
            .filter(|name| old.get(*name) != new.get(*name))
            .map(|name| format!("{}.{name}", rule.section))
            .collect();
    }
    let setting = if rule.key == "*" {
        rule.section.to_owned()
    } else {
        format!("{}.{}", rule.section, rule.key)
    };
    if unset(before, default) == unset(after, default) {
        Vec::new()
    } else {
        vec![setting]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(json: &str) -> Config {
        Config::from_json_bytes(json.as_bytes()).expect("the document is valid")
    }

    #[test]
    fn an_entry_of_a_section_of_entries_names_the_entry() {
        let before = config(r#"{"protocols":{"wlr-layer-shell":true}}"#);
        let after = config(r#"{"protocols":{"wlr-layer-shell":false,"ext-idle-notify":true}}"#);
        let settings: Vec<String> = restart_required(&before, &after, Process::Compositor)
            .into_iter()
            .map(|pending| pending.setting)
            .collect();
        assert_eq!(settings, ["protocols.ext-idle-notify", "protocols.wlr-layer-shell"]);
    }

    #[test]
    fn an_explicit_default_is_the_same_as_an_absent_key() {
        let before = config("{}");
        let after = config(r#"{"compositor":{"renderer":"auto"}}"#);
        assert!(restart_required(&before, &after, Process::Compositor).is_empty());
        let changed = config(r#"{"compositor":{"renderer":"software"}}"#);
        assert_eq!(
            restart_required(&before, &changed, Process::Compositor)
                .into_iter()
                .map(|pending| pending.setting)
                .collect::<Vec<_>>(),
            ["compositor.renderer"]
        );
    }

    #[test]
    fn a_setting_changed_back_is_not_pending() {
        let started = config(r#"{"compositor":{"renderer":"software"}}"#);
        assert!(restart_required(&started, &started.clone(), Process::Compositor).is_empty());
    }

    #[test]
    fn only_the_named_process_is_reported() {
        let before = config("{}");
        let after = config(r#"{"keyring":{"enabled":false},"compositor":{"renderer":"software"}}"#);
        let services: Vec<String> = restart_required(&before, &after, Process::Services)
            .into_iter()
            .map(|pending| pending.setting)
            .collect();
        assert_eq!(services, ["keyring.enabled"]);
    }

    #[test]
    fn an_unused_setting_is_reported_when_it_changes() {
        let before = config("{}");
        let after = config(r#"{"compositor":{"blur":"on"}}"#);
        let reported = no_effect(&before, &after);
        assert_eq!(reported.len(), 1);
        assert_eq!(reported[0].setting, "compositor.blur");
        assert_eq!(reported[0].kind, PendingKind::NoEffect);
        assert!(no_effect(&after, &after).is_empty());
    }
}
