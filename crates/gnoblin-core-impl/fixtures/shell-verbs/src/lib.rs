//! A shell that uses the compositor's verbs and signals (`Gnoblin::call_verb_with`, `Gnoblin::subscribe`), and says what came back.
//!
//! The session (`crates/gnoblin/tests/native_shell_verbs.rs`) reads its lines from the compositor's output and checks the
//! compositor's own state against them. It calls `idle.inhibit` (a hold the compositor counts), a verb on an object that does not
//! exist, and a verb with a wrong argument list; and it subscribes to the session's bell. It never calls a power verb: a session
//! on a machine with a power backend would act on it.

use std::io::Write as _;

use gnoblin_core as gnoblin;
use gnoblin_core::schema::gnoblin::{idle, session};
use gnoblin_core::{Answer, Gnoblin, ShellValue, Value};

fn say(line: &str) {
    let _ = writeln!(std::io::stdout().lock(), "[shell-verbs] {line}");
}

/// What an answer says, in a form the test can match.
fn outcome(answer: &Answer) -> String {
    match &answer.error {
        None => format!("ok result={:?}", answer.text()),
        Some(code) => format!("refused code={code}"),
    }
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Result<()> {
    g.subscribe(session::CLASS, session::signal::BELL, |event| {
        match event.value(session::fields::bell::COUNT) {
            Some(ShellValue::Uint(count)) => say(&format!("bell count={count} from object {}", event.node().0)),
            other => say(&format!("bell with a payload of the wrong shape: {other:?}")),
        }
    })?;

    g.call_verb_with(
        idle::CLASS,
        idle::method::INHIBIT,
        &[Value::Text("shell-verbs".to_owned())],
        |answer| say(&format!("inhibit answered {}", outcome(&answer))),
    )?;
    // An object the compositor does not have: the answer is a refusal, never a hang.
    g.call_verb_on(0x7fff_ffff, session::CLASS, session::function::LOGOUT, &[], |answer| {
        say(&format!("logout on a missing object answered {}", outcome(&answer)));
    })?;
    // A wrong argument list is refused before anything goes out.
    match g.call_verb(idle::CLASS, idle::method::INHIBIT, &[]) {
        Ok(()) => say("inhibit with no holder was sent"),
        Err(error) => say(&format!("inhibit with no holder refused: {error}")),
    }
    say("ready");
    Ok(())
}
