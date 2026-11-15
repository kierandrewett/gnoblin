//! Tests of this module.

use std::time::{Duration, Instant};

use super::{JsProcessCode, JsProcessError, JsProcesses, JsSpawnOptions, JsStream};

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

fn options(json: &str) -> JsSpawnOptions {
    serde_json::from_str(json).expect("the options parse")
}

/// Waits for the end of process `id`, with a deadline and no sleeping beyond the poll.
fn end_of(processes: &mut JsProcesses, id: u64) -> super::JsExit {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some((_, exit)) = processes.drain().into_iter().find(|(ended, _)| *ended == id) {
            return exit;
        }
        assert!(Instant::now() < deadline, "process {id} never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_program_ends_with_its_exit_code_and_its_captured_output() {
    let mut processes = JsProcesses::default();
    let started = processes
        .spawn(
            &argv(&["sh", "-c", "printf out; printf err >&2; exit 3"]),
            &options(r#"{"stdout":"capture","stderr":"capture"}"#),
        )
        .expect("sh starts");
    assert_eq!(started.id, 1);
    let exit = end_of(&mut processes, started.id);
    assert_eq!(exit.exit_code, Some(3));
    assert_eq!(exit.signal, None);
    assert_eq!(exit.stdout.as_deref(), Some("out"));
    assert_eq!(exit.stderr.as_deref(), Some("err"));
    assert_eq!(processes.running(), 0, "an ended process is forgotten");
}

#[test]
fn env_cwd_and_stdin_reach_the_program() {
    let mut processes = JsProcesses::default();
    let started = processes
        .spawn(
            &argv(&["sh", "-c", "printf '%s|%s|' \"$GNOBLIN_TEST\" \"$(pwd)\"; cat"]),
            &options(r#"{"env":{"GNOBLIN_TEST":"v"},"cwd":"/","stdin":"fed","stdout":"capture"}"#),
        )
        .expect("sh starts");
    assert_eq!(end_of(&mut processes, started.id).stdout.as_deref(), Some("v|/|fed"));
}

#[test]
fn output_past_the_limit_is_dropped_and_flagged() {
    let mut processes = JsProcesses::default();
    let started = processes
        .spawn(
            &argv(&["sh", "-c", "printf 0123456789"]),
            &options(r#"{"stdout":"capture","outputLimit":4}"#),
        )
        .expect("sh starts");
    let exit = end_of(&mut processes, started.id);
    assert_eq!(exit.stdout.as_deref(), Some("0123"));
    assert!(exit.stdout_truncated);
    assert!(!exit.stderr_truncated);
}

#[test]
fn kill_ends_the_process_group_and_names_the_signal() {
    let mut processes = JsProcesses::default();
    let started = processes
        .spawn(&argv(&["sh", "-c", "sleep 60 & wait"]), &JsSpawnOptions::default())
        .expect("sh starts");
    processes.kill(started.id, "kill").expect("a running process is killed");
    let exit = end_of(&mut processes, started.id);
    assert_eq!(exit.exit_code, None);
    assert_eq!(exit.signal.as_deref(), Some("kill"));
    let error = processes
        .kill(started.id, "term")
        .expect_err("an ended process has nothing to kill");
    assert_eq!(error.code, JsProcessCode::NotFound);
}

#[test]
fn a_timeout_terminates_the_program() {
    let mut processes = JsProcesses::default();
    let started = processes
        .spawn(&argv(&["sleep", "60"]), &options(r#"{"timeout":50}"#))
        .expect("sleep starts");
    let exit = end_of(&mut processes, started.id);
    assert!(exit.timed_out);
    assert_eq!(exit.signal.as_deref(), Some("term"));
}

#[test]
fn a_missing_program_is_not_found_and_limits_are_refused_by_name() {
    let mut processes = JsProcesses::default();
    let error = processes
        .spawn(&argv(&["/nonexistent/gnoblin-test"]), &JsSpawnOptions::default())
        .expect_err("no such program");
    assert_eq!(error.code, JsProcessCode::NotFound);
    for (parts, json, needle) in [
        (argv(&[]), "{}", "argv holds"),
        (argv(&["true"]), r#"{"cwd":"relative"}"#, "cwd"),
        (argv(&["true"]), r#"{"timeout":0}"#, "timeout"),
        (argv(&["true"]), r#"{"outputLimit":99999999}"#, "outputLimit"),
        (argv(&["true"]), r#"{"env":{"A=B":"x"}}"#, "env names"),
    ] {
        let JsProcessError { code, message } = processes
            .spawn(&parts, &options(json))
            .expect_err("the options are refused");
        assert_eq!(code, JsProcessCode::InvalidArgument);
        assert!(message.contains(needle), "{message} names {needle}");
    }
    assert!(processes.kill(99, "term").is_err());
    assert_eq!(
        processes.kill(1, "nope").expect_err("a bad signal").code,
        JsProcessCode::InvalidArgument
    );
    assert_eq!(JsStream::default(), JsStream::Log);
}
