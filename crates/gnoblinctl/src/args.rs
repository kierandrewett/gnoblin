//! Method arguments: `NAME=VALUE` tokens, typed by the method's declared parameters.
//!
//! The schema is the authority on which parameters exist and what type each one
//! has, so an argument the method does not declare, a missing parameter and a
//! value that does not fit its type are all usage errors (exit 2) here rather than
//! a frame the compositor has to refuse. That is what "typed argument parsing from
//! the schema types" buys: the compositor never sees a call this build could have
//! rejected itself.

use gnoblin_model::Fields;

use crate::tree::CliMethod;
use crate::value;

/// Parse every token into the method's arguments.
pub(crate) fn parse(method: &CliMethod, tokens: &[String]) -> Result<Fields, String> {
    let mut fields = Fields::new();
    for token in tokens {
        let (name, text) = token
            .split_once('=')
            .ok_or_else(|| format!("`{token}` is not `NAME=VALUE`"))?;
        let parameter = method
            .params
            .iter()
            .find(|parameter| parameter.name == name)
            .ok_or_else(|| {
                format!(
                    "`{}` takes {}, not `{name}`",
                    method.name,
                    declared(method, "parameter")
                )
            })?;
        if fields.contains_key(name) {
            return Err(format!("`{name}` was given twice"));
        }
        let ty = parameter.ty()?;
        // A secret parameter's value never reaches a message: the schema says the
        // field is secret, and an error printed on a terminal is a place it would
        // otherwise end up.
        let value = value::parse(&ty, text).map_err(|reason| {
            if parameter.secret {
                format!("`{name}` does not fit `{}`", parameter.ty)
            } else {
                format!("`{name}`: {reason}")
            }
        })?;
        fields.insert(name.to_owned(), value);
    }
    let missing: Vec<&str> = method
        .params
        .iter()
        .filter(|parameter| !fields.contains_key(parameter.name))
        .map(|parameter| parameter.name)
        .collect();
    if !missing.is_empty() {
        return Err(format!("`{}` needs {}", method.name, quoted(&missing)));
    }
    Ok(fields)
}

/// `nothing`, `a parameter` or `parameters`.
fn declared(method: &CliMethod, what: &str) -> String {
    if method.params.is_empty() {
        return "nothing".to_owned();
    }
    format!(
        "{} {what}s",
        quoted(&method.params.iter().map(|parameter| parameter.name).collect::<Vec<_>>())
    )
}

/// `a`, `a` and `b`, or `a`, `b` and `c`, each in backticks.
fn quoted(names: &[&str]) -> String {
    let names: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match names.split_last() {
        None => "nothing".to_owned(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{CliMethod, CliParam};

    const PARAMS: &[CliParam] = &[
        CliParam {
            name: "user",
            ty: "string",
            secret: false,
        },
        CliParam {
            name: "attempts",
            ty: "uint",
            secret: false,
        },
    ];

    const METHOD: CliMethod = CliMethod {
        name: "begin_auth",
        params: PARAMS,
    };

    fn tokens(pairs: &[&str]) -> Vec<String> {
        pairs.iter().map(|pair| (*pair).to_owned()).collect()
    }

    #[test]
    fn every_declared_parameter_is_typed_and_collected() {
        let fields = parse(&METHOD, &tokens(&["user=alice", "attempts=2"])).expect("both are declared");
        assert_eq!(fields["user"], gnoblin_model::Value::Text("alice".to_owned()));
        assert_eq!(fields["attempts"], gnoblin_model::Value::Uint(2));
    }

    #[test]
    fn a_token_without_a_value_is_refused() {
        let error = parse(&METHOD, &tokens(&["user"])).expect_err("not `NAME=VALUE`");
        assert!(error.contains("is not `NAME=VALUE`"), "{error}");
    }

    #[test]
    fn an_undeclared_parameter_names_the_declared_ones() {
        let error = parse(&METHOD, &tokens(&["user=alice", "attempts=1", "extra=1"])).expect_err("extra is undeclared");
        assert!(error.contains("`user` and `attempts`"), "{error}");
    }

    #[test]
    fn a_missing_parameter_is_named() {
        let error = parse(&METHOD, &tokens(&["user=alice"])).expect_err("attempts is missing");
        assert!(error.contains("needs `attempts`"), "{error}");
    }

    #[test]
    fn a_repeated_parameter_is_refused() {
        let error = parse(&METHOD, &tokens(&["user=a", "user=b", "attempts=1"])).expect_err("twice");
        assert!(error.contains("`user` was given twice"), "{error}");
    }

    #[test]
    fn a_value_that_does_not_fit_its_type_is_a_usage_error() {
        let error = parse(&METHOD, &tokens(&["user=alice", "attempts=many"])).expect_err("not a uint");
        assert!(error.contains("`attempts`") && error.contains("uint"), "{error}");
    }

    #[test]
    fn a_secret_parameter_value_never_reaches_the_error() {
        const SECRET: &[CliParam] = &[CliParam {
            name: "response",
            ty: "uint",
            secret: true,
        }];
        const AUTH: CliMethod = CliMethod {
            name: "auth_response",
            params: SECRET,
        };
        let error = parse(&AUTH, &tokens(&["response=hunter2"])).expect_err("not a uint");
        assert!(!error.contains("hunter2"), "the value was echoed: {error}");
        assert!(error.contains("`response`"), "{error}");
    }

    #[test]
    fn a_method_without_parameters_takes_nothing() {
        const BARE: CliMethod = CliMethod {
            name: "focus",
            params: &[],
        };
        assert!(parse(&BARE, &[]).expect("no arguments").is_empty());
        let error = parse(&BARE, &tokens(&["title=x"])).expect_err("focus takes nothing");
        assert!(error.contains("takes nothing"), "{error}");
    }
}
