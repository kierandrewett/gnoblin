//! The catalog classes of the built-in layouts, written from `gnoblin_std_abi::layouts::LAYOUT_KINDS`.
//!
//! `LAYOUT_KINDS` is the one table of what a built-in layout takes and answers to: the library validates every option against it and
//! takes its defaults from it. The object protocol's catalog names the same classes, so rather than keep a second list by hand this
//! module renders the table as a schema file (`layouts.toml` of the std catalog directory, which is not on disk) that the catalog
//! loads beside the files that are. A kind added to the table is a class in every language and in the library's own catalog the day
//! the generator runs; the library registers its implementation from the same table (`workspace_objects/stock.rs`).
//!
//! A kind `dwindle` is the class `dwindle_layout`, which extends the interface `layout` (so a workspace takes it where it takes a
//! shell's own layout). Each option is a property, in the table's order, and the constructor takes them all. Each action is a method;
//! an action that takes nothing and answers a value that may be absent (`strip.pin`) is a read-only optional property instead, because
//! only an object result may be none. An enumeration option is the enum `<kind>_<option>`, its members counted from one in the order
//! the table lists them (the table numbers them from zero without a gap, which a test checks).

use std::collections::BTreeSet;
use std::fmt::Write as _;

use gnoblin_std_abi::layouts::{ActionDesc, EnumMember, Initial, LAYOUT_KINDS, OptionDesc, ValueDesc, ValueShape};

/// The name of the schema file the text stands for, in the std catalog directory.
pub const ORIGIN_FILE: &str = "layouts.toml";

/// The class a kind of layout is.
pub fn class_name(kind: &str) -> String {
    format!("{kind}_layout")
}

/// The enum an option, an argument or an answer of `kind` named `name` is.
pub fn enum_name(kind: &str, name: &str) -> String {
    format!("{kind}_{name}")
}

fn type_of(kind: &str, name: &str, desc: &ValueDesc) -> String {
    match desc.ty {
        ValueShape::Bool => "bool".to_owned(),
        ValueShape::Number => "double".to_owned(),
        ValueShape::Int => "int".to_owned(),
        ValueShape::Enum(_) => format!("enum:{}", enum_name(kind, name)),
        ValueShape::Node => "uint".to_owned(),
        ValueShape::Text => "string".to_owned(),
    }
}

fn toml_text(text: &str) -> String {
    format!("{text:?}")
}

/// A number as TOML writes a float: it always has a point.
fn float(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        format!("{value}")
    }
}

fn default_of(option: &OptionDesc, members: Option<&'static [EnumMember]>) -> Option<String> {
    Some(match (&option.default, members) {
        (Initial::None, _) => return None,
        (Initial::Bool(flag), _) => flag.to_string(),
        (Initial::Text(text), _) => toml_text(text),
        (Initial::Number(value), Some(members)) => {
            let member = members.iter().find(|member| f64::from(member.value) == *value)?;
            toml_text(member.name)
        }
        (Initial::Number(value), None) => match option.value.ty {
            ValueShape::Int => format!("{}", *value as i64),
            _ => float(*value),
        },
    })
}

fn comment(out: &mut String, text: &str) {
    let mut line = String::from("#");
    for word in text.split_whitespace() {
        if line.len() + word.len() + 1 > 132 {
            out.push_str(&line);
            out.push('\n');
            line = String::from("#");
        }
        line.push(' ');
        line.push_str(word);
    }
    out.push_str(&line);
    out.push('\n');
}

fn enums_of(kind: &str, options: &[OptionDesc], actions: &[ActionDesc]) -> Vec<(String, &'static [EnumMember])> {
    let mut found: Vec<(String, &'static [EnumMember])> = Vec::new();
    let mut add = |name: &str, desc: &ValueDesc| {
        if let ValueShape::Enum(members) = desc.ty {
            let class = enum_name(kind, name);
            if !found.iter().any(|(seen, _)| *seen == class) {
                found.push((class, members));
            }
        }
    };
    for option in options {
        add(option.name, &option.value);
    }
    for action in actions {
        for arg in action.args {
            add(arg.name, &arg.value);
        }
        if let Some(returns) = &action.returns {
            add(action.name, returns);
        }
    }
    found
}

/// The schema of the built-in layouts.
pub fn schema_text() -> String {
    let mut out = String::new();
    out.push_str(
        "# The built-in layouts of `gnoblin.std`, written from `gnoblin_std_abi::layouts::LAYOUT_KINDS` by `gnoblin-sdkgen`.\n\
         namespace = \"gnoblin.std\"\nversion = \"0.2.0\"\nlocal = true\n",
    );
    for kind in LAYOUT_KINDS {
        let mut enum_members: BTreeSet<String> = BTreeSet::new();
        for (name, members) in enums_of(kind.name, kind.options, kind.actions) {
            if !enum_members.insert(name.clone()) {
                continue;
            }
            out.push('\n');
            comment(
                &mut out,
                &format!(
                    "The values of `{name}` (option, argument or answer of the {} layout).",
                    kind.name
                ),
            );
            out.push_str("[[enum]]\n");
            let _ = writeln!(out, "name = {}", toml_text(&name));
            out.push_str("since = \"0.2.0\"\n");
            let names: Vec<String> = members.iter().map(|member| toml_text(member.name)).collect();
            let _ = writeln!(out, "values = [{}]", names.join(", "));
        }
        out.push('\n');
        comment(&mut out, kind.doc);
        out.push_str("[[object]]\n");
        let _ = writeln!(out, "name = {}", toml_text(&class_name(kind.name)));
        out.push_str("since = \"0.2.0\"\nbase = \"layout\"\n");
        for option in kind.options {
            out.push('\n');
            comment(&mut out, option.doc);
            out.push_str("[[object.property]]\n");
            let _ = writeln!(out, "name = {}", toml_text(option.name));
            let _ = writeln!(
                out,
                "type = {}",
                toml_text(&type_of(kind.name, option.name, &option.value))
            );
            out.push_str("since = \"0.2.0\"\n");
            let members = match option.value.ty {
                ValueShape::Enum(members) => Some(members),
                _ => None,
            };
            if let Some(default) = default_of(option, members) {
                let _ = writeln!(out, "default = {default}");
            }
            if option.value.optional {
                out.push_str("optional = true\n");
            }
        }
        for action in kind.actions {
            out.push('\n');
            comment(&mut out, action.doc);
            if action.is_reading() {
                out.push_str("[[object.property]]\n");
                let _ = writeln!(out, "name = {}", toml_text(action.name));
                let returns = action.returns.as_ref().expect("a reading answers");
                let _ = writeln!(out, "type = {}", toml_text(&type_of(kind.name, action.name, returns)));
                out.push_str("since = \"0.2.0\"\nreadonly = true\noptional = true\n");
                continue;
            }
            out.push_str("[[object.method]]\n");
            let _ = writeln!(out, "name = {}", toml_text(action.name));
            out.push_str("since = \"0.2.0\"\n");
            if let Some(returns) = &action.returns {
                let _ = writeln!(
                    out,
                    "returns = {}",
                    toml_text(&type_of(kind.name, action.name, returns))
                );
            }
            for arg in action.args {
                out.push_str("\n[[object.method.param]]\n");
                let _ = writeln!(out, "name = {}", toml_text(arg.name));
                let _ = writeln!(out, "type = {}", toml_text(&type_of(kind.name, arg.name, &arg.value)));
                if arg.value.optional {
                    out.push_str("optional = true\n");
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_enumerations_of_the_table_count_from_zero_without_a_gap() {
        for kind in LAYOUT_KINDS {
            for (name, members) in enums_of(kind.name, kind.options, kind.actions) {
                for (at, member) in members.iter().enumerate() {
                    assert_eq!(member.value as usize, at, "{name}.{}", member.name);
                }
            }
        }
    }

    #[test]
    fn every_kind_is_a_class_that_the_schema_parser_accepts() {
        let text = schema_text();
        let schema = gnoblin_schema::parse_schema("layouts.toml", &text).expect("the layout schema parses");
        let names: Vec<&str> = schema.objects.iter().map(|object| object.name.as_str()).collect();
        for kind in LAYOUT_KINDS {
            assert!(
                names.contains(&class_name(kind.name).as_str()),
                "{} has no class",
                kind.name
            );
        }
    }

    /// A class that extends the interface `layout` is a class in every language, with the interface's methods and its own options, and
    /// the interface itself is what a shell implements.
    #[test]
    fn a_class_that_extends_an_interface_is_rendered_in_every_language() {
        let spec = crate::SdkSpec::load(&crate::workspace_root()).expect("the specification loads");
        let files = crate::files_of(&crate::render(&spec, None).expect("every target renders"));
        let text = |path: &str| {
            files
                .iter()
                .find(|(name, _)| name == path)
                .unwrap_or_else(|| panic!("{path} is not rendered"))
                .1
                .clone()
        };
        let python = text("python/gnoblin/objects/std.py");
        assert!(python.contains("class DwindleLayout(_B.Object):"));
        assert!(python.contains("class Layout(_B.Implementor):"));
        assert!(python.contains("def toggle_split(self)"));
        assert!(python.contains("def compute(self, context: LayoutContext)"));
        let types = text("js/gnoblin-std/generated/objects.d.ts");
        assert!(types.contains("export declare class DwindleLayout {"));
        assert!(types.contains("export declare abstract class Layout {"));
        assert!(types.contains("toggleSplit("));
        let script = text("js/gnoblin-std/generated/objects.js");
        assert!(script.contains("export class DwindleLayout {"));
        let rust = text("rust-objects/std_api.rs");
        assert!(rust.contains("pub struct DwindleLayout"));
        assert!(rust.contains("pub struct LayoutHandlers"));
        assert!(rust.contains("pub fn move_split"));
    }

    #[test]
    fn two_renders_are_equal() {
        assert_eq!(schema_text(), schema_text());
    }
}
