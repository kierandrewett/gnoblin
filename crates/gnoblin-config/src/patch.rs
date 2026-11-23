//! A shell that owns some of the keys of the document, and not the whole of it: laying its keys over the document in force, and taking
//! back the keys an earlier commit of its own set.
//!
//! A commit of the compositor's document carries the whole document, so a shell that writes only a few settings (the device settings, the
//! compositor's own) reads the document in force, takes out the key paths its last commit set ([`remove`]), lays what it sets now over
//! the rest ([`overlay`]) and commits the result. A key it no longer sets is then gone from the document, and the section it leaves
//! empty goes with it; a key some other writer set is not the shell's to take back, because only the paths the shell set are removed.
//! [`leaves`] names those paths. The JavaScript runtime and the core library's own shell host both commit this way for the keys they own.

use crate::value::Value;

/// A key of the document, from the section down: `["dialogs", "style", "radius"]`.
pub type KeyPath = Vec<String>;

/// A document with no keys.
#[must_use]
pub fn empty() -> Value {
    Value::Map(std::collections::BTreeMap::new())
}

/// Every key path `patch` sets: each scalar and each list (a list is one value), and no empty table.
#[must_use]
pub fn leaves(patch: &Value) -> Vec<KeyPath> {
    fn walk(value: &Value, path: &mut KeyPath, out: &mut Vec<KeyPath>) {
        match value {
            Value::Map(entries) => {
                for (key, value) in entries {
                    path.push(key.clone());
                    walk(value, path, out);
                    path.pop();
                }
            }
            _ if path.is_empty() => {}
            _ => out.push(path.clone()),
        }
    }
    let mut out = Vec::new();
    walk(patch, &mut Vec::new(), &mut out);
    out
}

/// Sets every leaf of `patch` in `document`; the tables between are made when they are not there.
pub fn overlay(document: &mut Value, patch: &Value) {
    let (Value::Map(target), Value::Map(entries)) = (document, patch) else {
        return;
    };
    for (key, value) in entries {
        match value {
            Value::Map(inner) if inner.is_empty() => {}
            Value::Map(_) => {
                let slot = target.entry(key.clone()).or_insert_with(empty);
                if !matches!(slot, Value::Map(_)) {
                    *slot = empty();
                }
                overlay(slot, value);
            }
            _ => {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

/// Takes `path` out of `document`, and any table that is left empty above it.
pub fn remove(document: &mut Value, path: &[String]) {
    let (Some((first, rest)), Value::Map(entries)) = (path.split_first(), document) else {
        return;
    };
    if rest.is_empty() {
        entries.remove(first);
        return;
    }
    let emptied = match entries.get_mut(first) {
        Some(inner @ Value::Map(_)) => {
            remove(inner, rest);
            matches!(inner, Value::Map(entries) if entries.is_empty())
        }
        _ => false,
    };
    if emptied {
        entries.remove(first);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(text: &str) -> Value {
        let parsed: serde_json::Value = serde_json::from_str(text).expect("test json");
        Value::from_json(&parsed).expect("a configuration tree")
    }

    #[test]
    fn leaves_are_scalars_and_lists_and_never_empty_tables() {
        let patch = json(r#"{"dialogs":{"style":{"radius":12,"font":{"size":14}}},"input":{"keyboard":{}},"x":[1,2]}"#);
        let mut found = leaves(&patch);
        found.sort();
        assert_eq!(
            found,
            vec![
                vec![
                    "dialogs".to_owned(),
                    "style".to_owned(),
                    "font".to_owned(),
                    "size".to_owned()
                ],
                vec!["dialogs".to_owned(), "style".to_owned(), "radius".to_owned()],
                vec!["x".to_owned()],
            ]
        );
    }

    #[test]
    fn overlay_keeps_what_the_patch_does_not_name() {
        let mut document = json(r#"{"dialogs":{"style":{"radius":3,"scale":2}},"cursor":{"size":24}}"#);
        overlay(&mut document, &json(r#"{"dialogs":{"style":{"radius":12}}}"#));
        assert_eq!(
            document,
            json(r#"{"dialogs":{"style":{"radius":12,"scale":2}},"cursor":{"size":24}}"#)
        );
    }

    #[test]
    fn removing_a_path_prunes_the_tables_it_leaves_empty_and_no_others() {
        let mut document = json(r#"{"dialogs":{"style":{"radius":12}},"cursor":{"size":24}}"#);
        remove(
            &mut document,
            &["dialogs".to_owned(), "style".to_owned(), "radius".to_owned()],
        );
        assert_eq!(document, json(r#"{"cursor":{"size":24}}"#));
        // A path that is not there changes nothing.
        remove(&mut document, &["dialogs".to_owned(), "style".to_owned()]);
        assert_eq!(document, json(r#"{"cursor":{"size":24}}"#));
    }

    #[test]
    fn a_table_the_removal_does_not_empty_stays() {
        let mut document = json(r#"{"dialogs":{"style":{"radius":12,"scale":2}}}"#);
        remove(
            &mut document,
            &["dialogs".to_owned(), "style".to_owned(), "radius".to_owned()],
        );
        assert_eq!(document, json(r#"{"dialogs":{"style":{"scale":2}}}"#));
    }
}
