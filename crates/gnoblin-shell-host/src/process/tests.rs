//! Tests of this module.

use std::time::{Duration, Instant};

use super::{ProcessCode, ProcessError, Processes, SpawnOptions, StreamMode};

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

fn options(json: &str) -> SpawnOptions {
    serde_json::from_str(json).expect("the options parse")
}

/// Waits for the end of process `id`, with a deadline and no sleeping beyond the poll.
fn end_of(processes: &mut Processes, id: u64) -> super::ProcessExit {
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
    let mut processes = Processes::default();
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
    let mut processes = Processes::default();
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
    let mut processes = Processes::default();
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
    let mut processes = Processes::default();
    let started = processes
        .spawn(&argv(&["sh", "-c", "sleep 60 & wait"]), &SpawnOptions::default())
        .expect("sh starts");
    processes.kill(started.id, "kill").expect("a running process is killed");
    let exit = end_of(&mut processes, started.id);
    assert_eq!(exit.exit_code, None);
    assert_eq!(exit.signal.as_deref(), Some("kill"));
    let error = processes
        .kill(started.id, "term")
        .expect_err("an ended process has nothing to kill");
    assert_eq!(error.code, ProcessCode::NotFound);
}

#[test]
fn a_timeout_terminates_the_program() {
    let mut processes = Processes::default();
    let started = processes
        .spawn(&argv(&["sleep", "60"]), &options(r#"{"timeout":50}"#))
        .expect("sleep starts");
    let exit = end_of(&mut processes, started.id);
    assert!(exit.timed_out);
    assert_eq!(exit.signal.as_deref(), Some("term"));
}

#[test]
fn a_missing_program_is_not_found_and_limits_are_refused_by_name() {
    let mut processes = Processes::default();
    let error = processes
        .spawn(&argv(&["/nonexistent/gnoblin-test"]), &SpawnOptions::default())
        .expect_err("no such program");
    assert_eq!(error.code, ProcessCode::NotFound);
    for (parts, json, needle) in [
        (argv(&[]), "{}", "argv holds"),
        (argv(&["true"]), r#"{"cwd":"relative"}"#, "cwd"),
        (argv(&["true"]), r#"{"timeout":0}"#, "timeout"),
        (argv(&["true"]), r#"{"outputLimit":99999999}"#, "outputLimit"),
        (argv(&["true"]), r#"{"env":{"A=B":"x"}}"#, "env names"),
    ] {
        let ProcessError { code, message } = processes
            .spawn(&parts, &options(json))
            .expect_err("the options are refused");
        assert_eq!(code, ProcessCode::InvalidArgument);
        assert!(message.contains(needle), "{message} names {needle}");
    }
    assert!(processes.kill(99, "term").is_err());
    assert_eq!(
        processes.kill(1, "nope").expect_err("a bad signal").code,
        ProcessCode::InvalidArgument
    );
    assert_eq!(StreamMode::default(), StreamMode::Log);
}

// ---- stages -----------------------------------------------------------------------------------------------------------

/// Whether the process with this operating system id exists and has not ended (a zombie has).
fn alive(pid: u32) -> bool {
    std::fs::metadata(format!("/proc/{pid}")).is_ok()
        && std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| !stat.contains(") Z "))
}

fn sleeping(tag: &str) -> Vec<String> {
    argv(&["sleep", tag])
}

#[test]
fn a_staged_spawn_starts_nothing_until_the_generation_takes_over() {
    let mut processes = Processes::default();
    processes.begin_stage();
    let started = processes
        .spawn(&sleeping("32.5001"), &SpawnOptions::default())
        .expect("a spawn is kept");
    assert_eq!((started.id, started.pid), (1, 0), "it has its id and no pid");
    assert_eq!(processes.running(), 0, "nothing runs");
    assert_eq!(processes.pid(started.id), None);

    let failed = processes.end_stage(1);
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(processes.running(), 1);
    let pid = processes.pid(started.id).expect("it runs under the id it was given");
    assert!(pid > 0);
    assert_eq!(
        processes.owner(started.id),
        Some(1),
        "the generation that took over owns it"
    );
    processes.kill(started.id, "kill").expect("it is killed");
    let _ = end_of(&mut processes, started.id);
}

#[test]
fn an_abandoned_stage_starts_nothing_and_a_handler_spawn_after_it_runs_at_once() {
    let mut processes = Processes::default();
    processes.begin_stage();
    processes
        .spawn(&sleeping("32.5002"), &SpawnOptions::default())
        .expect("a spawn is kept");
    processes.abandon_stage();
    assert_eq!(processes.running(), 0);
    assert!(processes.end_stage(1).is_empty(), "there is no stage left to take over");

    let started = processes
        .spawn(&sleeping("32.5003"), &SpawnOptions::default())
        .expect("a handler's spawn starts at once");
    assert!(started.pid > 0);
    processes.kill(started.id, "kill").expect("it is killed");
    let _ = end_of(&mut processes, started.id);
}

#[test]
fn a_stage_that_spawns_what_the_running_config_spawned_takes_that_process_over() {
    let mut processes = Processes::default();
    processes.begin_stage();
    let first = processes
        .spawn(&sleeping("32.5004"), &SpawnOptions::default())
        .expect("kept");
    assert!(processes.end_stage(1).is_empty());
    let pid = processes.pid(first.id).expect("it runs");

    processes.begin_stage();
    let again = processes
        .spawn(&sleeping("32.5004"), &SpawnOptions::default())
        .expect("kept");
    assert_eq!(
        (again.id, again.pid),
        (first.id, pid),
        "the stage is given the process that runs"
    );
    let twice = processes
        .spawn(&sleeping("32.5004"), &SpawnOptions::default())
        .expect("kept");
    assert_ne!(
        twice.id, first.id,
        "a process is taken over once: a second spawn of the same program is a second process"
    );
    assert!(processes.end_stage(2).is_empty());
    assert_eq!(processes.running(), 2);
    assert_eq!(processes.owner(first.id), Some(2), "the process has a new owner");
    assert_eq!(processes.pid(first.id), Some(pid), "and is the same process");
    assert!(alive(pid));

    for id in [first.id, twice.id] {
        processes.kill(id, "kill").expect("it is killed");
        let _ = end_of(&mut processes, id);
    }
}

#[test]
fn the_same_program_with_other_options_is_another_process() {
    let mut processes = Processes::default();
    processes.begin_stage();
    let first = processes
        .spawn(&sleeping("32.5005"), &SpawnOptions::default())
        .expect("kept");
    assert!(processes.end_stage(1).is_empty());

    processes.begin_stage();
    let other = processes
        .spawn(&sleeping("32.5005"), &options(r#"{"env":{"A":"b"}}"#))
        .expect("kept");
    assert_ne!(other.id, first.id);
    assert!(processes.end_stage(2).is_empty());
    assert_eq!(
        processes.running(),
        2,
        "the old one is stopped by the signal and ends on its own time"
    );
    for id in [first.id, other.id] {
        let _ = processes.kill(id, "kill");
        let _ = end_of(&mut processes, id);
    }
}

#[test]
fn what_the_config_no_longer_starts_is_stopped_and_what_a_handler_started_is_left() {
    let mut processes = Processes::default();
    processes.begin_stage();
    let declared = processes
        .spawn(&sleeping("32.5006"), &SpawnOptions::default())
        .expect("kept");
    assert!(processes.end_stage(1).is_empty());
    let handled = processes
        .spawn(&sleeping("32.5007"), &SpawnOptions::default())
        .expect("a handler's spawn starts at once");
    let (declared_pid, handled_pid) = (processes.pid(declared.id).expect("runs"), handled.pid);

    processes.begin_stage();
    assert!(processes.end_stage(2).is_empty(), "the new config spawns nothing");
    let exit = end_of(&mut processes, declared.id);
    assert_eq!(
        exit.signal.as_deref(),
        Some("term"),
        "the config's process was told to stop"
    );
    assert!(!alive(declared_pid));
    assert!(alive(handled_pid), "the process a handler started keeps running");
    assert_eq!(
        processes.owner(handled.id),
        Some(1),
        "and belongs to the generation that was running when it started"
    );

    processes.kill(handled.id, "kill").expect("it is killed");
    let _ = end_of(&mut processes, handled.id);
}

#[test]
fn a_program_that_cannot_start_when_the_generation_takes_over_ends_with_status_127_and_is_reported() {
    let mut processes = Processes::default();
    processes.begin_stage();
    let started = processes
        .spawn(&argv(&["/nonexistent/program"]), &SpawnOptions::default())
        .expect("the stage cannot know");
    let failed = processes.end_stage(1);
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].0, started.id);
    assert_eq!(failed[0].1.code, ProcessCode::NotFound);
    let exit = end_of(&mut processes, started.id);
    assert_eq!(exit.exit_code, Some(127), "the handle the script holds ends");
    assert!(exit.stderr.is_some_and(|text| text.contains("/nonexistent/program")));
}

#[test]
fn a_program_killed_before_the_generation_takes_over_never_starts_and_ends_with_the_signal() {
    let mut processes = Processes::default();
    processes.begin_stage();
    let started = processes
        .spawn(&sleeping("32.5008"), &SpawnOptions::default())
        .expect("kept");
    processes.kill(started.id, "term").expect("a kept spawn can be killed");
    assert!(processes.end_stage(1).is_empty());
    assert_eq!(processes.running(), 0, "it never started");
    let exit = end_of(&mut processes, started.id);
    assert_eq!(exit.signal.as_deref(), Some("term"));
}

#[test]
fn a_stage_counts_the_spawns_it_keeps_against_the_running_limit() {
    let mut processes = Processes::default();
    processes.begin_stage();
    for _ in 0..super::MAX_RUNNING {
        processes
            .spawn(&sleeping("32.5009"), &SpawnOptions::default())
            .expect("under the limit");
    }
    let refused = processes
        .spawn(&sleeping("32.5009"), &SpawnOptions::default())
        .expect_err("past it");
    assert_eq!(refused.code, ProcessCode::Busy);
    processes.abandon_stage();
}

#[test]
fn a_restage_inside_a_stage_starts_the_program_once_whatever_the_interrupted_generation_spawned() {
    let mut processes = Processes::default();
    processes.begin_stage();
    processes
        .spawn(&sleeping("32.5010"), &SpawnOptions::default())
        .expect("kept");
    // The first generation was interrupted: the one that replaces it spawns the same program.
    processes.restage();
    let again = processes
        .spawn(&sleeping("32.5010"), &SpawnOptions::default())
        .expect("kept");
    assert!(processes.end_stage(1).is_empty());
    assert_eq!(
        processes.running(),
        1,
        "one process, not one for each generation staged"
    );
    assert!(processes.pid(again.id).is_some(), "and it is the second generation's");
    processes.kill(again.id, "kill").expect("it is killed");
    let _ = end_of(&mut processes, again.id);
}

#[test]
fn a_restage_with_no_stage_open_changes_nothing() {
    let mut processes = Processes::default();
    processes.restage();
    let started = processes
        .spawn(&sleeping("32.5011"), &SpawnOptions::default())
        .expect("a spawn outside a stage starts at once");
    assert!(started.pid > 0);
    processes.kill(started.id, "kill").expect("it is killed");
    let _ = end_of(&mut processes, started.id);
}
