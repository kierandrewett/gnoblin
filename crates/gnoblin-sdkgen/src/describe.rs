//! The text of a member's documentation: its description, then what the schema says about its parameters, its range, unit and default,
//! the error codes it raises and an example, written for each language.
//!
//! The description is the comment above the table ([`NamespaceDocs`]); everything else is in the catalog ([`Notes`]). The three
//! languages write the same facts differently: TSDoc tags (`@param`, `@throws`, `@example`), reStructuredText fields (`:param x:`,
//! `:raises GnoblinError:`) and rustdoc headings (`# Arguments`). The text this module returns goes through the target's own comment
//! writer, which escapes it for the language.

use std::collections::BTreeMap;

use gnoblin_schema::{DefaultValue, Field, Notes, ObjectType, Property, Range, SchemaExample};

use crate::docs::{DocLanguage, NamespaceDocs, base_of};
use crate::text::{camel, python_ident, rust_ident, snake, ts_ident};

/// What a language writes of a member's documentation: the name of a parameter, the info string of a fence and the example it has.
pub(crate) trait DocLanguageExt {
    /// The name a parameter or a field has in the language.
    fn param_name(self, name: &str) -> String;
    /// The word that opens a fenced example of the language in a doc comment.
    fn fence(self) -> &'static str;
    /// The example of the language, when there is one.
    fn example(self, example: &SchemaExample) -> Option<&str>;
}

impl DocLanguageExt for DocLanguage {
    fn param_name(self, name: &str) -> String {
        match self {
            DocLanguage::TypeScript => ts_ident(&camel(name)),
            DocLanguage::Python => python_ident(&snake(name)),
            DocLanguage::Rust => rust_ident(&snake(name)),
        }
    }

    fn fence(self) -> &'static str {
        match self {
            DocLanguage::TypeScript => "typescript",
            DocLanguage::Python => "python",
            DocLanguage::Rust => "rust,ignore",
        }
    }

    fn example(self, example: &SchemaExample) -> Option<&str> {
        match self {
            DocLanguage::TypeScript => example.typescript.as_deref(),
            DocLanguage::Python => example.python.as_deref(),
            DocLanguage::Rust => example.rust.as_deref(),
        }
    }
}

/// A number as the schema wrote it: `-1`, `0.5`, `10000`.
pub(crate) fn number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// A default as a reader sees it, in the language's spelling of a boolean.
pub(crate) fn default_text(default: &DefaultValue, flavor: Option<DocLanguage>) -> String {
    match default {
        DefaultValue::Bool(true) if flavor == Some(DocLanguage::Python) => "True".to_owned(),
        DefaultValue::Bool(false) if flavor == Some(DocLanguage::Python) => "False".to_owned(),
        DefaultValue::Bool(flag) => flag.to_string(),
        DefaultValue::Int(value) => value.to_string(),
        DefaultValue::Uint(value) => value.to_string(),
        DefaultValue::Double(value) => number(*value),
        DefaultValue::Text(text) => format!("\"{text}\""),
        DefaultValue::List(items) => format!(
            "[{}]",
            items
                .iter()
                .map(|item| default_text(item, flavor))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// What the structured keys say, as sentences: `Range: -1 to 1. Unit: ms. Default: 0.` Empty when there is none.
pub(crate) fn facts(
    range: Option<Range>,
    unit: Option<&str>,
    default: Option<&DefaultValue>,
    flavor: DocLanguage,
) -> String {
    let mut parts = Vec::new();
    if let Some(range) = range {
        parts.push(format!("Range: {} to {}.", number(range.min), number(range.max)));
    }
    if let Some(unit) = unit {
        parts.push(format!("Unit: {unit}."));
    }
    if let Some(default) = default {
        parts.push(format!("Default: `{}`.", default_text(default, Some(flavor))));
    }
    parts.join(" ")
}

/// A description and the facts after it, as one paragraph.
fn described(description: &str, facts: &str) -> String {
    match (description.is_empty(), facts.is_empty()) {
        (_, true) => description.to_owned(),
        (true, false) => facts.to_owned(),
        (false, false) => format!("{description} {facts}"),
    }
}

/// The text of one parameter or field: its description and its facts.
pub(crate) fn param_text(description: &str, field: &Field, flavor: DocLanguage) -> String {
    described(
        description,
        &facts(
            field.notes.range,
            field.notes.unit.as_deref(),
            field.default.as_ref(),
            flavor,
        ),
    )
}

/// Everything the schema says about a member of a class, written for `flavor`: the text a language's comment carries. Empty when the
/// schema says nothing.
pub(crate) fn member_text(
    docs: &NamespaceDocs,
    objects: &BTreeMap<&str, &ObjectType>,
    flavor: DocLanguage,
    class: &str,
    member: &str,
) -> String {
    let bases = |name: &str| base_of(objects, name);
    let description = docs.member(class, member, bases);
    let Some(object) = objects.get(class) else {
        return description.to_owned();
    };
    let params = |fields: &[Field], parameters: &mut Vec<(String, String)>| {
        for field in fields {
            let text = param_text(docs.child(class, member, &field.name, bases), field, flavor);
            if !text.is_empty() {
                parameters.push((flavor.param_name(&field.name), text));
            }
        }
    };
    let mut parameters: Vec<(String, String)> = Vec::new();
    let notes: Option<&Notes>;
    let own_facts: String;
    if let Some(method) = object.methods.iter().find(|method| method.name == member) {
        params(&method.params, &mut parameters);
        notes = Some(&method.notes);
        own_facts = String::new();
    } else if let Some(signal) = object.signals.iter().find(|signal| signal.name == member) {
        params(&signal.fields, &mut parameters);
        notes = Some(&signal.notes);
        own_facts = String::new();
    } else if let Some(property) = object.properties.iter().find(|property| property.name == member) {
        notes = Some(&property.notes);
        own_facts = property_facts(property, flavor);
    } else {
        return description.to_owned();
    }
    let notes = notes.map_or_else(Notes::default, Clone::clone);
    render(flavor, &described(description, &own_facts), &parameters, &notes)
}

/// The facts of a property: its range, unit and default.
pub(crate) fn property_facts(property: &Property, flavor: DocLanguage) -> String {
    facts(
        property.notes.range,
        property.notes.unit.as_deref(),
        property.default.as_ref(),
        flavor,
    )
}

/// The description, then the parameters, the codes raised and the example, in the language's tags.
fn render(flavor: DocLanguage, description: &str, params: &[(String, String)], notes: &Notes) -> String {
    let mut blocks: Vec<String> = Vec::new();
    if !description.is_empty() {
        blocks.push(description.to_owned());
    }
    let raises: Vec<String> = notes.raises.iter().map(|code| format!("`{code}`")).collect();
    let example = notes
        .example
        .as_ref()
        .and_then(|example| flavor.example(example))
        .map(|code| format!("```{}\n{}\n```", flavor.fence(), code.trim_end()));
    match flavor {
        DocLanguage::TypeScript => {
            let mut tags: Vec<String> = params
                .iter()
                .map(|(name, text)| format!("@param {name} {text}"))
                .collect();
            if !raises.is_empty() {
                tags.push(format!("@throws GnoblinError with the code {}.", or_list(&raises)));
            }
            if !tags.is_empty() {
                blocks.push(tags.join("\n"));
            }
            if let Some(example) = example {
                blocks.push(format!("@example\n{example}"));
            }
        }
        DocLanguage::Python => {
            let mut fields: Vec<String> = params
                .iter()
                .map(|(name, text)| format!(":param {name}: {text}"))
                .collect();
            if !raises.is_empty() {
                fields.push(format!(":raises GnoblinError: with the code {}.", or_list(&raises)));
            }
            if !fields.is_empty() {
                blocks.push(fields.join("\n"));
            }
            if let Some(example) = example {
                blocks.push(format!("Example:\n\n{example}"));
            }
        }
        DocLanguage::Rust => {
            if !params.is_empty() {
                let items: Vec<String> = params
                    .iter()
                    .map(|(name, text)| format!("* `{name}`: {text}"))
                    .collect();
                blocks.push(format!("# Arguments\n\n{}", items.join("\n")));
            }
            if !raises.is_empty() {
                blocks.push(format!("Fails with the code {}.", or_list(&raises)));
            }
            if let Some(example) = example {
                blocks.push(format!("# Examples\n\n{example}"));
            }
        }
    }
    blocks.join("\n\n")
}

/// `a`, `a` or `b`, `a`, `b` or `c`.
fn or_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [first @ .., last] => format!("{} or {last}", first.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_drop_a_needless_decimal_point() {
        assert_eq!(number(-1.0), "-1");
        assert_eq!(number(0.5), "0.5");
        assert_eq!(number(10000.0), "10000");
    }

    #[test]
    fn facts_read_as_sentences() {
        let facts = facts(
            Some(Range { min: -1.0, max: 1.0 }),
            Some("ms"),
            Some(&DefaultValue::Bool(true)),
            DocLanguage::Python,
        );
        assert_eq!(facts, "Range: -1 to 1. Unit: ms. Default: `True`.");
        assert_eq!(super::facts(None, None, None, DocLanguage::Rust), "");
    }

    #[test]
    fn each_language_writes_its_own_tags() {
        let notes = Notes {
            raises: vec!["denied".to_owned(), "not_found".to_owned()],
            example: Some(SchemaExample {
                typescript: Some("m.reset(\"speed\");".to_owned()),
                ..SchemaExample::default()
            }),
            ..Notes::default()
        };
        let params = [("name".to_owned(), "A setting.".to_owned())];
        let ts = render(DocLanguage::TypeScript, "Resets.", &params, &notes);
        assert_eq!(
            ts,
            "Resets.\n\n@param name A setting.\n@throws GnoblinError with the code `denied` or `not_found`.\n\n@example\n```typescript\nm.reset(\"speed\");\n```"
        );
        let py = render(DocLanguage::Python, "Resets.", &params, &notes);
        assert_eq!(
            py,
            "Resets.\n\n:param name: A setting.\n:raises GnoblinError: with the code `denied` or `not_found`."
        );
        let rust = render(DocLanguage::Rust, "Resets.", &params, &notes);
        assert_eq!(
            rust,
            "Resets.\n\n# Arguments\n\n* `name`: A setting.\n\nFails with the code `denied` or `not_found`."
        );
    }
}
