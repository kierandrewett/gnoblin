// Refuse two Lua keys that would be one key of one table.
//
// Included by `build.rs`, which runs it over the schema, and by `src/lib.rs` under
// `cfg(test)`, which is where its unit tests run: `cargo test` does not execute the
// tests of a build script.

use std::collections::BTreeMap;

/// Check the keys one Lua table would take from the object types declared for it.
///
/// `owners` pairs each object type's schema name with the key it takes in `table`;
/// `reserved` are keys `table` already has by hand. A clash names both sides, so the
/// refusal says which declaration to rename.
fn check_lua_names(table: &str, owners: &[(String, String)], reserved: &[&str]) -> Result<(), String> {
    let mut taken: BTreeMap<&str, &str> = BTreeMap::new();
    for (owner, key) in owners {
        if reserved.contains(&key.as_str()) {
            return Err(format!(
                "{owner} is named `{key}`, which `{table}` already has by hand; rename the object type"
            ));
        }
        if let Some(first) = taken.insert(key.as_str(), owner.as_str()) {
            return Err(format!(
                "{first} and {owner} are both `{table}.{key}`; rename one of them"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::check_lua_names;
    use crate::bindings::HAND_WRITTEN;

    fn owners(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(owner, key)| ((*owner).to_owned(), (*key).to_owned()))
            .collect()
    }

    #[test]
    fn distinct_names_outside_the_hand_written_set_are_accepted() {
        let owners = owners(&[("gnoblin.window", "window"), ("gnoblin.session", "session")]);
        assert!(check_lua_names("gnoblin", &owners, HAND_WRITTEN).is_ok());
    }

    #[test]
    fn an_object_named_like_a_hand_written_member_is_refused_naming_both() {
        let owners = owners(&[("gnoblin.timers", "timers")]);
        let error = check_lua_names("gnoblin", &owners, HAND_WRITTEN).expect_err("timers is taken");
        assert!(error.contains("gnoblin.timers"), "{error}");
        assert!(error.contains("`timers`"), "{error}");
        assert!(error.contains("`gnoblin`"), "{error}");
    }

    #[test]
    fn two_object_types_with_one_key_are_refused_naming_both() {
        let owners = owners(&[("gnoblin.window", "window"), ("gnoblin.window_two", "window")]);
        let error = check_lua_names("gnoblin", &owners, HAND_WRITTEN).expect_err("window is taken twice");
        assert!(error.contains("gnoblin.window and gnoblin.window_two"), "{error}");
    }

    #[test]
    fn a_required_module_has_no_hand_written_names_to_clash_with() {
        let owners = owners(&[("bsm.greeter.timers", "timers")]);
        assert!(check_lua_names("require(\"bsm.greeter\")", &owners, &[]).is_ok());
    }
}
