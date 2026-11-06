//! The values `org.freedesktop.impl.portal.Settings` publishes, and where they come from.
//!
//! D15 makes the settings bridge a one-way publish from Gnoblin configuration: the
//! compositor owns the values, it states them as properties of one `settings` object
//! (`crates/gnoblin-schema/schema/settings.toml`), and this module translates them into
//! the namespaces toolkits read. Nothing here writes anything. There is no method that
//! takes a value and no path back to the configuration, which is what makes "a write
//! from a toolkit is never read back" a fact about the code rather than a promise.
//!
//! # Why a translation table rather than one vocabulary everywhere
//!
//! The schema's property names are the settings' own names with `-` turned into `_`,
//! because a schema property cannot contain a dash. The portal's names are the standard
//! names toolkits look up (`org.freedesktop.appearance` `color-scheme`,
//! `org.gnome.desktop.interface` `font-name`), and a toolkit cannot be renamed. The two
//! must therefore meet somewhere, and [`KEY`] is the one place they do: a property the
//! table does not name is not published, so a new schema property cannot leak into the
//! Settings interface by accident.
//!
//! # Failure modes first
//!
//! The test names below are the tests owed for these rules. None is written yet in this
//! crate, so no entry here is covered by a passing test.
//!
//! 1. **A change is published that nobody asked for.** A property the table does not name
//!    produces no change: `an_unknown_property_is_not_published`.
//! 2. **A value's type changes under a subscription.** A property whose value does not
//!    have the type the schema declares is dropped rather than published as the wrong
//!    D-Bus type: `a_value_of_the_wrong_shape_is_not_published`.
//! 3. **The accent colour arrives one component at a time.** The triple is published
//!    only once all three components are known, and a change to one re-publishes the
//!    whole triple because that is the value toolkits read:
//!    `the_accent_colour_is_one_triple`.
//! 4. **A read-back loop.** The interface has no writable member and the table has no
//!    outbound path: `the_settings_interface_writes_nothing_back`.
//! 5. **A namespace glob matches too much.** `org.*` does not match
//!    `org.freedesktop.appearance`, because the spec's globbing is for trailing
//!    sections only: `a_namespace_glob_matches_only_a_prefix`.
//! 6. **An object's end leaves its values behind.** A closed `settings` object empties
//!    the table rather than keeping the last values: `a_closed_settings_object_publishes_nothing`.

use std::collections::BTreeMap;

use gnoblin_model::{Observation, Value};

/// One published setting.
///
/// The set is the D-Bus types the interface can carry for the values the schema declares:
/// a `u`, a string, a `b`, and the `(ddd)` accent colour. A setting cannot be a type the
/// schema cannot express, which is why there is no map or list here.
#[derive(Clone, Debug, PartialEq)]
pub enum Setting {
    /// A `u`.
    Uint(u32),
    /// A string.
    Text(String),
    /// A `b`.
    Bool(bool),
    /// A `d`, such as the text scaling factor.
    Double(f64),
    /// A `(ddd)`, the accent colour's red, green and blue in sRGB, each in [0, 1].
    Color(f64, f64, f64),
}

/// One thing a subscriber is told.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    /// The namespace, such as `org.freedesktop.appearance`.
    pub namespace: &'static str,
    /// The key inside it, such as `color-scheme`.
    pub key: &'static str,
    /// Its new value.
    pub value: Setting,
}

/// One published key: the schema property it comes from and the name toolkits look up.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Key {
    /// The property name on the `settings` object.
    pub property: &'static str,
    /// The namespace toolkits read.
    pub namespace: &'static str,
    /// The key inside that namespace.
    pub key: &'static str,
}

/// The keys this backend publishes.
///
/// `org.freedesktop.appearance` is the shared base GTK 4, Qt 6.5 and later, Chromium,
/// Electron and Firefox read; `org.gnome.desktop.interface` is what a GTK 3 application
/// reads when it runs under a desktop that publishes it, which is how GSettings-based
/// software keeps its fonts and themes under Gnoblin without a dconf layer.
pub const KEY: &[Key] = &[
    Key {
        property: "color_scheme",
        namespace: "org.freedesktop.appearance",
        key: "color-scheme",
    },
    Key {
        property: "contrast",
        namespace: "org.freedesktop.appearance",
        key: "contrast",
    },
    Key {
        property: "reduced_motion",
        namespace: "org.freedesktop.appearance",
        key: "reduced-motion",
    },
    Key {
        property: "gtk_theme",
        namespace: "org.gnome.desktop.interface",
        key: "gtk-theme",
    },
    Key {
        property: "icon_theme",
        namespace: "org.gnome.desktop.interface",
        key: "icon-theme",
    },
    Key {
        property: "cursor_theme",
        namespace: "org.gnome.desktop.interface",
        key: "cursor-theme",
    },
    Key {
        property: "cursor_size",
        namespace: "org.gnome.desktop.interface",
        key: "cursor-size",
    },
    Key {
        property: "font_name",
        namespace: "org.gnome.desktop.interface",
        key: "font-name",
    },
    Key {
        property: "document_font_name",
        namespace: "org.gnome.desktop.interface",
        key: "document-font-name",
    },
    Key {
        property: "monospace_font_name",
        namespace: "org.gnome.desktop.interface",
        key: "monospace-font-name",
    },
    Key {
        property: "text_scaling_factor",
        namespace: "org.gnome.desktop.interface",
        key: "text-scaling-factor",
    },
    Key {
        property: "button_layout",
        namespace: "org.gnome.desktop.interface",
        key: "button-layout",
    },
    Key {
        property: "enable_animations",
        namespace: "org.gnome.desktop.interface",
        key: "enable-animations",
    },
];

/// The accent colour's three properties.
///
/// It is one published value built from three properties, so it is not a [`KEY`]: the
/// table would have to say "these three make one key", and three rows that each claim the
/// same key would be three chances to publish half a colour.
const ACCENT: [&str; 3] = ["accent_red", "accent_green", "accent_blue"];

/// The namespace and key of the accent colour.
const ACCENT_NAMESPACE: &str = "org.freedesktop.appearance";
const ACCENT_KEY: &str = "accent-color";

/// The published values.
#[derive(Debug, Default)]
pub struct Table {
    values: BTreeMap<(&'static str, &'static str), Setting>,
    /// The accent's components as they have been seen, so a triple is published only when
    /// all three are known.
    accent: [Option<f64>; 3],
}

impl Table {
    /// An empty table: nothing published, which is what a session with no `settings`
    /// object has.
    #[must_use]
    pub fn new() -> Table {
        Table::default()
    }

    /// Every published value, ordered by namespace and key.
    pub fn values(&self) -> impl Iterator<Item = (&'static str, &'static str, &Setting)> + '_ {
        self.values
            .iter()
            .map(|((namespace, key), value)| (*namespace, *key, value))
    }

    /// One value, or nothing when this backend publishes no such key.
    #[must_use]
    pub fn read(&self, namespace: &str, key: &str) -> Option<&Setting> {
        self.values.get(&(find(namespace, key)?))
    }

    /// Whether a namespace pattern from `ReadAll` matches a namespace.
    ///
    /// The interface's own rule: an empty pattern or an empty list matches everything,
    /// and globbing is supported for trailing sections only, so `org.example.*` matches
    /// `org.example.a` and `org.example.a.b`, while `org.*` does not match
    /// `org.freedesktop.appearance` — the pattern names one section, not a prefix of the
    /// whole name.
    #[must_use]
    pub fn matches(pattern: &str, namespace: &str) -> bool {
        if pattern.is_empty() {
            return true;
        }
        match pattern.strip_suffix('*') {
            // A trailing `*` stands for the rest of one dotted name, so the pattern must
            // end at a separator for the match to be a section boundary.
            Some(prefix) => {
                let prefix = prefix.strip_suffix('.').unwrap_or(prefix);
                namespace == prefix || namespace.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('.'))
            }
            None => pattern == namespace,
        }
    }

    /// Apply one observation and return the changes a subscriber must be told.
    pub fn apply(&mut self, observation: &Observation) -> Vec<Change> {
        match observation {
            Observation::Opened { view, .. } => {
                // The snapshot is the whole state, and a subscriber that starts from it is
                // current: a property the object does not carry is not published, so the
                // table starts empty rather than keeping what a previous object said.
                self.values.clear();
                self.accent = [None; 3];
                let mut changes = Vec::new();
                for (property, value) in view.properties() {
                    changes.extend(self.publish(property, value));
                }
                changes
            }
            Observation::Changed { property, value, .. } => self.publish(property, value),
            Observation::Closed { .. } => {
                let had_values = !self.values.is_empty();
                self.values.clear();
                self.accent = [None; 3];
                if had_values {
                    gnoblin_log::info!("the settings object ended; nothing is published until it returns");
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Publish one property, returning what changed.
    fn publish(&mut self, property: &str, value: &Value) -> Vec<Change> {
        if let Some(index) = ACCENT.iter().position(|name| *name == property) {
            return self.publish_accent(index, value);
        }
        let Some(entry) = KEY.iter().find(|entry| entry.property == property) else {
            // A schema property this backend does not publish. Nothing is wrong: a new
            // property in the graph is not a new key in the portal.
            return Vec::new();
        };
        let Some(published) = convert(property, value) else {
            return Vec::new();
        };
        let slot = (entry.namespace, entry.key);
        if self.values.get(&slot) == Some(&published) {
            return Vec::new();
        }
        self.values.insert(slot, published.clone());
        vec![Change {
            namespace: entry.namespace,
            key: entry.key,
            value: published,
        }]
    }

    /// Publish one component of the accent colour.
    fn publish_accent(&mut self, index: usize, value: &Value) -> Vec<Change> {
        let Value::Double(component) = value else {
            return Vec::new();
        };
        let component = *component;
        if !component.is_finite() || !(0.0..=1.0).contains(&component) {
            // The interface's own rule: an out-of-range component means the accent colour
            // is unset, so it is not published rather than clamped to something the user
            // did not choose.
            return Vec::new();
        }
        self.accent[index] = Some(component);
        let [Some(red), Some(green), Some(blue)] = self.accent else {
            return Vec::new();
        };
        let published = Setting::Color(red, green, blue);
        let slot = (ACCENT_NAMESPACE, ACCENT_KEY);
        if self.values.get(&slot) == Some(&published) {
            return Vec::new();
        }
        self.values.insert(slot, published.clone());
        vec![Change {
            namespace: ACCENT_NAMESPACE,
            key: ACCENT_KEY,
            value: published,
        }]
    }
}

/// The published names for a namespace and key, if this backend publishes it.
fn find(namespace: &str, key: &str) -> Option<(&'static str, &'static str)> {
    if namespace == ACCENT_NAMESPACE && key == ACCENT_KEY {
        return Some((ACCENT_NAMESPACE, ACCENT_KEY));
    }
    KEY.iter()
        .find(|entry| entry.namespace == namespace && entry.key == key)
        .map(|entry| (entry.namespace, entry.key))
}

/// Convert a graph value into the setting the schema declares for it.
///
/// The schema's type is the converter: an `uint` becomes a `u`, a `double` an `f64`, a
/// `string` a `s` and a `bool` a `b`. A value of another shape is dropped, because a
/// toolkit that reads a `u` and receives a `s` fails in a way the user cannot report.
fn convert(property: &str, value: &Value) -> Option<Setting> {
    match value {
        Value::Uint(raw) => u32::try_from(*raw)
            .inspect_err(|_| {
                gnoblin_log::warn!(
                    property,
                    value = raw,
                    "a settings value does not fit the `u` its key is published as"
                );
            })
            .ok()
            .map(Setting::Uint),
        Value::Double(raw) => Some(Setting::Double(*raw)),
        Value::Text(raw) => Some(Setting::Text(raw.clone())),
        Value::Bool(raw) => Some(Setting::Bool(*raw)),
        other => {
            gnoblin_log::warn!(
                property,
                kind = other.kind(),
                "a settings property has a shape its key is not published as"
            );
            None
        }
    }
}
