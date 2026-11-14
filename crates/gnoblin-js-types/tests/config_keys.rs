//! The `Config` interface of the hand-written declarations names every key `defineConfig` takes, and no other.
//!
//! `static/gnoblin-stock.d.ts` is written by hand beside the TypeScript it describes (`data/js/gnoblin/config.ts`), so
//! nothing but this test notices a key added to one and not the other. A key the declarations lack is a false type
//! error in the editor, and one they have in excess is a type that passes and then fails at load.

use std::collections::BTreeSet;

const DECLARATIONS: &str = include_str!("../static/gnoblin-stock.d.ts");
const CONFIG_TS: &str = include_str!("../../../data/js/gnoblin/config.ts");

/// The member names of the interface that opens with `opening`, whose members are indented by `indent` spaces.
fn members(source: &str, opening: &str, indent: usize) -> BTreeSet<String> {
    let pad = " ".repeat(indent);
    let close = format!("{}}}", " ".repeat(indent - 4));
    let mut inside = false;
    let mut names = BTreeSet::new();
    for line in source.lines() {
        if line.trim() == opening {
            inside = true;
        } else if inside && line == close {
            break;
        } else if inside {
            let Some(rest) = line.strip_prefix(&pad) else {
                continue;
            };
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() && rest[name.len()..].starts_with(['?', ':', '(']) {
                names.insert(name);
            }
        }
    }
    names
}

#[test]
fn the_declarations_name_exactly_the_keys_of_the_typescript_config() {
    let declared = members(DECLARATIONS, "export interface Config {", 8);
    let implemented = members(CONFIG_TS, "export interface GnoblinConfig {", 4);
    assert!(implemented.len() > 25, "the parse found the keys: {implemented:?}");
    let missing: Vec<_> = implemented.difference(&declared).collect();
    let extra: Vec<_> = declared.difference(&implemented).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "gnoblin-stock.d.ts lacks {missing:?} and has {extra:?} that config.ts does not"
    );
}
