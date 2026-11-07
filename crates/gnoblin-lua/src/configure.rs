//! `gnoblin.configure`: the neutral configuration tree, validated at the call.
//!
//! The host owns no document model of its own. A `gnoblin.configure { ... }` table
//! is converted into [`gnoblin_config::Value`], merged into the document the files
//! have built so far, and handed to the configuration layer's validation. That is
//! what keeps one meaning per setting: the schema declares the key, the
//! configuration crate decides what it means, and the Lua host only carries the
//! tree across.
//!
//! Two properties of the conversion matter:
//!
//! - **A problem names its path.** Every refusal carries the dotted path into the
//!   document (`input.touchpad.natural-scroll`), including a table key that is not
//!   a string, a value that is a function, and a list with a hole.
//! - **A refusal changes nothing.** The merge happens on a copy; only a document
//!   that validates becomes the new document, so a rejected call leaves the
//!   accepted settings exactly as they were.
//!
//! An empty table converts to an empty [`Value::Map`]. Lua cannot distinguish
//! `{}` from an empty list, and guessing from the key would put the schema in the
//! host, so a document cannot clear a list-valued setting with `{}`; it clears it
//! by assigning the list it wants.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use gnoblin_client::Transport;
use gnoblin_config::{Config, MAX_DEPTH, Value, merge};
use mlua::{Error, Table, Value as LuaValue};

use crate::host::Shared;

/// Convert a `gnoblin.configure` argument into the document layer's tree.
pub(super) fn value_from_table(table: &Table) -> Result<Value, String> {
    convert_table(table, "", 0)
}

/// Apply one configuration table to the document under construction.
///
/// Called from the Lua binding, so it must not call into Lua while the shared
/// state is borrowed; everything Lua-shaped is done before the borrow.
pub(super) fn declare<T: Transport>(shared: &Rc<RefCell<Shared<T>>>, settings: &Table) -> Result<(), Error> {
    let layer = value_from_table(settings).map_err(|reason| Error::runtime(format!("gnoblin.configure: {reason}")))?;
    let mut state = shared.borrow_mut();
    if !state.configuring {
        return Err(Error::runtime(
            "gnoblin.configure is only available while the configuration loads",
        ));
    }
    let candidate = merge(state.document.clone(), layer);
    accept(&mut state, candidate, "gnoblin.configure")
}

/// Append one rule to the `window-rules` list, the way `gnoblin.window_rule` does.
///
/// Lists replace whole when layers merge, so appending cannot go through
/// [`merge`]: the existing list is read, the rule is pushed onto it, and the
/// whole list is validated. The rule is the last entry, so a problem names
/// `window-rules[N]` for the index the rule was given.
pub(super) fn append_rule<T: Transport>(shared: &Rc<RefCell<Shared<T>>>, rule: &Table) -> Result<(), Error> {
    let rule = value_from_table(rule).map_err(|reason| Error::runtime(format!("gnoblin.window_rule: {reason}")))?;
    let mut state = shared.borrow_mut();
    if !state.configuring {
        return Err(Error::runtime(
            "gnoblin.window_rule is only available while the configuration loads",
        ));
    }
    let mut candidate = state.document.clone();
    let Value::Map(root) = &mut candidate else {
        return Err(Error::runtime("gnoblin.window_rule: the document is not a table"));
    };
    match root
        .entry("window-rules".to_owned())
        .or_insert_with(|| Value::List(Vec::new()))
    {
        Value::List(rules) => rules.push(rule),
        _ => {
            return Err(Error::runtime(
                "gnoblin.window_rule: window-rules is not a list in the document",
            ));
        }
    }
    accept(&mut state, candidate, "gnoblin.window_rule")
}

/// Validate a candidate document and make it the document, or refuse it whole.
fn accept<T: Transport>(state: &mut Shared<T>, candidate: Value, call: &str) -> Result<(), Error> {
    match Config::from_value(candidate) {
        Ok(config) => {
            state.document = config.document().clone();
            Ok(())
        }
        Err(report) => {
            let mut message = format!("{call}: the settings were not accepted:");
            for problem in report.problems() {
                message.push_str("\n  ");
                message.push_str(&problem.to_string());
            }
            Err(Error::runtime(message))
        }
    }
}

/// Convert one Lua value, naming its path in every refusal.
fn convert(value: &LuaValue, path: &str, depth: usize) -> Result<Value, String> {
    if depth > MAX_DEPTH {
        return Err(format!("{path}: nested deeper than {MAX_DEPTH} levels"));
    }
    match value {
        LuaValue::Nil => Ok(Value::Null),
        LuaValue::Boolean(flag) => Ok(Value::Bool(*flag)),
        LuaValue::Integer(number) => Ok(Value::Integer(*number)),
        LuaValue::Number(number) if number.is_finite() => Ok(Value::Float(*number)),
        LuaValue::Number(_) => Err(format!("{path}: expected a finite number")),
        LuaValue::String(text) => match text.to_str() {
            Ok(text) => Ok(Value::String(text.to_string())),
            Err(_) => Err(format!("{path}: expected text, found a string that is not UTF-8")),
        },
        LuaValue::Table(table) => convert_table(table, path, depth),
        other => Err(format!("{path}: expected a value, found {}", other.type_name())),
    }
}

/// Convert one table: a list when its keys are `1..n`, a map when they are names.
fn convert_table(table: &Table, path: &str, depth: usize) -> Result<Value, String> {
    let path = if path.is_empty() { "<document>" } else { path };
    let mut pairs = Vec::new();
    for pair in table.pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair.map_err(|error| format!("{path}: {error}"))?;
        pairs.push((key, value));
    }
    if pairs.is_empty() {
        return Ok(Value::Map(BTreeMap::new()));
    }
    if pairs.iter().all(|(key, _)| matches!(key, LuaValue::Integer(_))) {
        let mut entries: Vec<(i64, &LuaValue)> = pairs
            .iter()
            .map(|(key, value)| match key {
                LuaValue::Integer(index) => (*index, value),
                _ => unreachable!("checked above"),
            })
            .collect();
        entries.sort_by_key(|(index, _)| *index);
        let contiguous = entries
            .iter()
            .enumerate()
            .all(|(position, (index, _))| *index == position as i64 + 1);
        if !contiguous {
            return Err(format!(
                "{path}: a table with integer keys must be a list numbered 1 to its length"
            ));
        }
        let mut items = Vec::with_capacity(entries.len());
        for (index, value) in entries {
            items.push(convert(value, &format!("{path}[{index}]"), depth + 1)?);
        }
        return Ok(Value::List(items));
    }
    let mut entries = BTreeMap::new();
    for (key, value) in pairs {
        let LuaValue::String(name) = key else {
            return Err(format!(
                "{path}: a table key must be a string, found {}",
                key.type_name()
            ));
        };
        let name = match name.to_str() {
            Ok(name) => name.to_string(),
            Err(_) => return Err(format!("{path}: a table key is not UTF-8")),
        };
        let child = format!("{path}.{name}");
        entries.insert(name, convert(&value, &child, depth + 1)?);
    }
    Ok(Value::Map(entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    fn document(lua: &Lua, text: &str) -> Result<Value, String> {
        let table: Table = lua.load(text).eval().expect("the test script builds a table");
        value_from_table(&table)
    }

    #[test]
    fn a_table_becomes_a_map_and_a_sequence_becomes_a_list() {
        let lua = Lua::new();
        let value = document(
            &lua,
            "return {input = {touchpad = {enabled = true}}, workspaces = {'main', 'web'}}",
        )
        .expect("a plain table converts");
        let root = value.as_table().expect("a map");
        assert_eq!(root.len(), 2);
        assert!(matches!(root.get("workspaces"), Some(Value::List(items)) if items.len() == 2));
    }

    #[test]
    fn a_function_is_refused_with_its_path() {
        let lua = Lua::new();
        let error =
            document(&lua, "return {compositor = {blur = function() end}}").expect_err("a function is not data");
        assert!(error.starts_with("<document>.compositor.blur:"), "{error}");
        assert!(error.contains("function"), "{error}");
    }

    #[test]
    fn a_list_with_a_hole_is_refused() {
        let lua = Lua::new();
        let error =
            document(&lua, "return {workspaces = {[1] = 'main', [3] = 'web'}}").expect_err("a hole is not a list");
        assert!(error.contains("numbered 1 to its length"), "{error}");
    }

    #[test]
    fn a_table_key_that_is_not_a_string_is_refused() {
        let lua = Lua::new();
        let error = document(&lua, "return {mixed = {[1] = 'main', name = 'web'}}").expect_err("keys mix");
        assert!(error.contains("a table key must be a string"), "{error}");
    }

    #[test]
    fn a_self_referencing_table_is_refused_rather_than_followed() {
        let lua = Lua::new();
        let error = document(&lua, "local t = {} t.self = t return t").expect_err("a cycle stops at the depth limit");
        assert!(error.contains("nested deeper"), "{error}");
    }
}
