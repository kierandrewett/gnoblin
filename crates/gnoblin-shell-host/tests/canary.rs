//! Canary tests of the generation host: the lifetime guarantees a shell relies on, proven with closures standing in for a
//! shell's callbacks (the dlopen-level proofs are `gnoblin-core-impl`'s `tests/scenarios.rs`).
//!
//! Each test is named for the failure it would catch:
//!
//! - a callback of a retired generation is never run again (the stale-callback canary);
//! - a release (`drop_user`) runs exactly once, by whichever path removes the registration;
//! - a failed `init` leaves the running generation serving and no trace of the failed one;
//! - an abandoned stage leaves no registration behind and runs no hook;
//! - a panic in a callback is contained and the next event is still delivered.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use gnoblin_shell_host::generation::{
    CallbackOutcome, CtxError, GenerationHost, GenerationId, GenerationInfo, GenerationState, Listener, Release,
    RemoveError, Work,
};
use gnoblin_shell_host::guard::PANICS;

/// An event with a number, enough to tell deliveries apart.
type Event = u32;

fn host() -> GenerationHost<Event> {
    GenerationHost::new()
}

fn generation(host: &GenerationHost<Event>, name: &str) -> GenerationId {
    host.new_generation(GenerationInfo {
        module_path: name.to_owned(),
        ..GenerationInfo::default()
    })
}

fn active(host: &GenerationHost<Event>, name: &str) -> GenerationId {
    let id = generation(host, name);
    host.activate(id).expect("a staged generation activates");
    id
}

/// What a registration's callback and release did.
#[derive(Clone, Default)]
struct Probe {
    calls: Arc<AtomicUsize>,
    releases: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<Event>>>,
}

impl Probe {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn releases(&self) -> usize {
        self.releases.load(Ordering::SeqCst)
    }

    fn listener(&self) -> Listener<Event> {
        let calls = Arc::clone(&self.calls);
        let seen = Arc::clone(&self.seen);
        Listener {
            matches: Box::new(|_| true),
            call: Arc::new(move |event| {
                calls.fetch_add(1, Ordering::SeqCst);
                seen.lock().unwrap().push(*event);
                CallbackOutcome::Handled
            }),
        }
    }

    fn release(&self) -> Option<Release> {
        let releases = Arc::clone(&self.releases);
        Some(Box::new(move || {
            releases.fetch_add(1, Ordering::SeqCst);
        }))
    }

    fn hook(&self) -> Work {
        let calls = Arc::clone(&self.calls);
        Box::new(move || {
            calls.fetch_add(1, Ordering::SeqCst);
        })
    }
}

fn listen(host: &GenerationHost<Event>, owner: GenerationId, probe: &Probe) -> u64 {
    host.register_listener(owner, probe.listener(), probe.release())
        .unwrap_or_else(|(error, _)| panic!("registers: {error:?}"))
}

#[test]
fn a_callback_of_a_retired_generation_is_never_run_again() {
    let host = host();
    let old = active(&host, "old");
    let probe = Probe::default();
    listen(&host, old, &probe);

    host.publish(1);
    assert_eq!(host.dispatch(old, 0), Ok(1));
    assert_eq!(probe.calls(), 1);

    assert_eq!(host.finish_retire(old), 1);
    // Five events more: the retired generation's callback must not see any, and its dispatch is refused.
    for event in 2..7 {
        host.publish(event);
    }
    assert_eq!(host.dispatch(old, 0), Err(CtxError::Retired(old)));
    let new = active(&host, "new");
    let fresh = Probe::default();
    listen(&host, new, &fresh);
    host.publish(7);
    // The queue is the shell's, not a generation's: what was published while nobody was active goes to the next active
    // generation, and only its callback is left to receive it.
    assert_eq!(
        host.dispatch(new, 0),
        Ok(6),
        "events 2 to 7, each to the new generation's one callback"
    );
    assert_eq!(*fresh.seen.lock().unwrap(), vec![2, 3, 4, 5, 6, 7]);
    assert_eq!(probe.calls(), 1, "no stale call into the retired generation");
    assert_eq!(*probe.seen.lock().unwrap(), vec![1]);
    assert_eq!(probe.releases(), 1, "its release ran when it retired");
    assert!(host.stats().stale_calls >= 1, "the refused dispatch is counted");
}

#[test]
fn a_retired_generation_cannot_register_and_its_release_is_handed_back_unrun() {
    let host = host();
    let old = active(&host, "old");
    host.finish_retire(old);
    let probe = Probe::default();
    let refused = host.register_listener(old, probe.listener(), probe.release());
    let (error, release) = refused.expect_err("a retired generation registers nothing");
    assert_eq!(error, CtxError::Retired(old));
    assert!(release.is_some(), "the caller still owns what it offered");
    assert_eq!(
        probe.releases(),
        0,
        "a registration that was not adopted is not released"
    );
    assert_eq!(host.live_registrations(), 0);
    assert_eq!(error.code(), gnoblin_core_abi::E_RETIRED);
    assert!(
        error.message("on_signal").contains("has been retired"),
        "{}",
        error.message("on_signal")
    );
}

#[test]
fn a_release_runs_exactly_once_on_every_path_that_removes_a_registration() {
    let host = host();
    let owner = active(&host, "shell");

    // Removed explicitly, then removed again.
    let explicit = Probe::default();
    let id = listen(&host, owner, &explicit);
    assert_eq!(host.remove(id), Ok(()));
    assert_eq!(host.remove(id), Err(RemoveError::Gone));
    assert_eq!(explicit.releases(), 1);

    // Removed by the retirement of its generation, and retirement repeated.
    let by_retire = Probe::default();
    listen(&host, owner, &by_retire);
    let hook = Probe::default();
    host.register_shutdown(owner, hook.hook(), hook.release())
        .unwrap_or_else(|(error, _)| panic!("registers: {error:?}"));
    host.run_shutdown_hooks(owner);
    host.run_shutdown_hooks(owner);
    assert_eq!(hook.calls(), 1, "a shutdown hook runs once");
    assert_eq!(hook.releases(), 1, "and its release runs once, after it");
    assert_eq!(host.finish_retire(owner), 1);
    assert_eq!(host.finish_retire(owner), 0, "retiring twice does nothing");
    assert_eq!(by_retire.releases(), 1);
    assert_eq!(explicit.releases(), 1);
}

#[test]
fn a_callback_that_removes_itself_has_its_release_deferred_until_it_returns() {
    let host = Arc::new(host());
    let owner = active(&host, "shell");
    let releases = Arc::new(AtomicUsize::new(0));
    let inside = Arc::new(AtomicUsize::new(usize::MAX));
    let id = Arc::new(Mutex::new(0_u64));
    let call = {
        let host = Arc::clone(&host);
        let releases = Arc::clone(&releases);
        let inside = Arc::clone(&inside);
        let id = Arc::clone(&id);
        Arc::new(move |_: &Event| {
            host.remove(*id.lock().unwrap())
                .expect("a callback removes its own registration");
            inside.store(releases.load(Ordering::SeqCst), Ordering::SeqCst);
            CallbackOutcome::Handled
        })
    };
    let release: Release = {
        let releases = Arc::clone(&releases);
        Box::new(move || {
            releases.fetch_add(1, Ordering::SeqCst);
        })
    };
    let registered = host
        .register_listener(
            owner,
            Listener {
                matches: Box::new(|_| true),
                call,
            },
            Some(release),
        )
        .unwrap_or_else(|(error, _)| panic!("registers: {error:?}"));
    *id.lock().unwrap() = registered;
    host.publish(1);
    host.publish(2);
    assert_eq!(host.dispatch(owner, 0), Ok(1), "removed before the second event");
    assert_eq!(
        inside.load(Ordering::SeqCst),
        0,
        "not released while its callback was running"
    );
    assert_eq!(
        releases.load(Ordering::SeqCst),
        1,
        "released once the callback returned"
    );
    assert_eq!(host.live_registrations(), 0);
}

#[test]
fn a_failed_init_keeps_the_old_generation_and_leaves_no_trace_of_the_new_one() {
    let host = host();
    let old = active(&host, "old");
    let serving = Probe::default();
    listen(&host, old, &serving);

    // The new generation initialises: it registers, then init fails, and the loader retires the half-made generation.
    let new = generation(&host, "new");
    let half = Probe::default();
    listen(&host, new, &half);
    let hook = Probe::default();
    host.register_shutdown(new, hook.hook(), hook.release())
        .unwrap_or_else(|(error, _)| panic!("registers: {error:?}"));
    assert_eq!(
        host.finish_retire(new),
        2,
        "both registrations of the failed init are removed"
    );

    assert_eq!(half.releases(), 1);
    assert_eq!(hook.releases(), 1);
    assert_eq!(hook.calls(), 0, "no hook of a generation that never ran");
    assert_eq!(host.state_of(new), Some(GenerationState::Retired));
    assert_eq!(host.state_of(old), Some(GenerationState::Active));
    assert_eq!(host.registrations_of(new), 0);

    host.publish(1);
    assert_eq!(host.dispatch(old, 0), Ok(1), "the old generation serves on");
    assert_eq!(serving.calls(), 1);
    assert_eq!(half.calls(), 0);
    assert_eq!(serving.releases(), 0, "nothing of the old generation was touched");
}

#[test]
fn an_abandoned_stage_leaves_no_registration_and_runs_no_callback() {
    let host = host();
    let old = active(&host, "old");
    let serving = Probe::default();
    listen(&host, old, &serving);
    let baseline = host.live_registrations();

    let staged = generation(&host, "staged");
    let listener = Probe::default();
    let hook = Probe::default();
    let post = Probe::default();
    listen(&host, staged, &listener);
    host.register_shutdown(staged, hook.hook(), hook.release())
        .unwrap_or_else(|(error, _)| panic!("registers: {error:?}"));
    let ran = Arc::clone(&post.calls);
    let discarded = Arc::clone(&post.releases);
    host.post(
        staged,
        Box::new(move || {
            ran.fetch_add(1, Ordering::SeqCst);
        }),
        Box::new(move || {
            discarded.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .unwrap_or_else(|(error, ..)| panic!("posts: {error:?}"));
    assert_eq!(host.registrations_of(staged), 2);
    assert_eq!(host.posts_of(staged), 1);

    // A staged generation receives nothing while the old one serves.
    host.publish(1);
    assert_eq!(host.dispatch(old, 0), Ok(1));
    assert_eq!(listener.calls(), 0, "staged registrations are inactive");

    // The stage is abandoned.
    host.finish_retire(staged);
    assert_eq!(host.live_registrations(), baseline, "no registration is left");
    assert_eq!(host.posts_of(staged), 0, "no post is left");
    assert_eq!(listener.releases(), 1);
    assert_eq!(hook.calls(), 0, "an abandoned generation runs no shutdown hook");
    assert_eq!(hook.releases(), 1);
    assert_eq!(post.calls(), 0, "a queued post never runs");
    assert_eq!(post.releases(), 1, "and is discarded exactly once");

    host.publish(2);
    assert_eq!(host.dispatch(old, 0), Ok(1));
    assert_eq!(listener.calls(), 0);
    assert_eq!(serving.calls(), 2);
}

#[test]
fn a_panic_in_a_callback_is_contained_and_the_next_event_is_still_delivered() {
    let host = host();
    let owner = active(&host, "shell");
    let after = Probe::default();
    let panicking = Probe::default();
    let flag = Arc::clone(&panicking.calls);
    host.register_listener(
        owner,
        Listener {
            matches: Box::new(|event: &Event| *event == 1),
            call: Arc::new(move |_| {
                flag.fetch_add(1, Ordering::SeqCst);
                panic!("a shell callback panics");
            }),
        },
        panicking.release(),
    )
    .unwrap_or_else(|(error, _)| panic!("registers: {error:?}"));
    listen(&host, owner, &after);

    let before = PANICS.load(Ordering::SeqCst);
    host.publish(1);
    host.publish(2);
    let called = host.dispatch(owner, 0).expect("the dispatch survives the panic");
    assert_eq!(called, 3, "the panicking callback and both of the other's");
    assert_eq!(panicking.calls(), 1);
    assert_eq!(
        *after.seen.lock().unwrap(),
        vec![1, 2],
        "later registrations and later events still arrive"
    );
    assert!(PANICS.load(Ordering::SeqCst) > before, "the panic is counted");

    // A callback that reports its own panic (a trampoline that caught it) is counted too.
    let reporting = Probe::default();
    host.register_listener(
        owner,
        Listener {
            matches: Box::new(|event: &Event| *event == 3),
            call: Arc::new(|_| CallbackOutcome::Panicked),
        },
        reporting.release(),
    )
    .unwrap_or_else(|(error, _)| panic!("registers: {error:?}"));
    let before = PANICS.load(Ordering::SeqCst);
    host.publish(3);
    host.dispatch(owner, 0).expect("dispatches");
    assert!(PANICS.load(Ordering::SeqCst) > before);
}

#[test]
fn a_panic_in_a_release_or_a_hook_does_not_stop_the_retirement() {
    let host = host();
    let owner = active(&host, "shell");
    let after = Probe::default();
    host.register_listener(
        owner,
        Listener {
            matches: Box::new(|_| true),
            call: Arc::new(|_| CallbackOutcome::Handled),
        },
        Some(Box::new(|| panic!("a drop_user panics"))),
    )
    .unwrap_or_else(|(error, _)| panic!("registers: {error:?}"));
    host.register_shutdown(owner, Box::new(|| panic!("a hook panics")), None)
        .unwrap_or_else(|(error, _)| panic!("registers: {error:?}"));
    listen(&host, owner, &after);
    host.run_shutdown_hooks(owner);
    assert_eq!(host.finish_retire(owner), 2);
    assert_eq!(
        after.releases(),
        1,
        "the registrations behind the panicking one are still released"
    );
}

#[test]
fn a_registration_of_a_staged_generation_receives_nothing_until_it_activates() {
    let host = host();
    let staged = generation(&host, "staged");
    let probe = Probe::default();
    listen(&host, staged, &probe);
    host.publish(1);
    assert_eq!(host.dispatch(staged, 0), Ok(0));
    assert_eq!(probe.calls(), 0);
    host.activate(staged).expect("activates");
    assert!(host.activate(staged).is_err(), "only a staged generation activates");
    assert_eq!(
        host.dispatch(staged, 0),
        Ok(1),
        "the event that waited is delivered once active"
    );
}

#[test]
fn shell_thread_calls_are_refused_on_another_thread_and_posted_work_runs_on_the_shell_thread() {
    let host = Arc::new(host());
    let owner = active(&host, "shell");
    let shell_thread = thread::current().id();
    let ran_on = Arc::new(Mutex::new(None));

    let other = {
        let host = Arc::clone(&host);
        let ran_on = Arc::clone(&ran_on);
        thread::spawn(move || {
            let dispatch = host.dispatch(owner, 0);
            let register = host
                .register_listener(
                    owner,
                    Listener {
                        matches: Box::new(|_| true),
                        call: Arc::new(|_| CallbackOutcome::Handled),
                    },
                    None,
                )
                .err()
                .map(|(error, _)| error);
            let post = host.post(
                owner,
                Box::new(move || {
                    *ran_on.lock().unwrap() = Some(thread::current().id());
                }),
                Box::new(|| {}),
            );
            (dispatch, register, post.is_ok())
        })
        .join()
        .expect("the other thread finishes")
    };
    assert_eq!(
        other,
        (
            Err(CtxError::WrongThread(owner)),
            Some(CtxError::WrongThread(owner)),
            true
        )
    );
    assert_eq!(host.dispatch(owner, 100), Ok(1));
    assert_eq!(*ran_on.lock().unwrap(), Some(shell_thread));
}

#[test]
fn the_mailbox_drops_the_oldest_event_when_nobody_dispatches() {
    let host = host();
    let owner = active(&host, "shell");
    let probe = Probe::default();
    listen(&host, owner, &probe);
    let cap = gnoblin_shell_host::generation::MAILBOX_CAP;
    for event in 0..(cap + 3) {
        host.publish(u32::try_from(event).expect("fits"));
    }
    assert_eq!(host.queued_events(), cap);
    assert_eq!(host.dispatch(owner, 0), Ok(cap));
    assert_eq!(
        probe.seen.lock().unwrap().first(),
        Some(&3),
        "the three oldest were dropped"
    );
}

#[test]
fn a_context_that_was_never_issued_is_unknown_and_not_retired() {
    let host = host();
    assert_eq!(host.check(99, false), Err(CtxError::Unknown(99)));
    assert_eq!(host.state_of(99), None);
    assert_eq!(host.dispatch(99, 0), Err(CtxError::Unknown(99)));
    let id = generation(&host, "kept");
    assert_eq!(host.info(id).expect("a record").module_path, "kept");
    host.finish_retire(id);
    assert_eq!(
        host.info(id)
            .expect("the record of a retired generation is kept")
            .module_path,
        "kept"
    );
}

#[test]
fn what_a_generation_still_holds_counts_its_registrations_and_its_posts_without_deadlock() {
    let host = host();
    let owner = active(&host, "shell");
    assert_eq!(host.held_by(owner), 0);
    listen(&host, owner, &Probe::default());
    host.post(owner, Box::new(|| {}), Box::new(|| {}))
        .unwrap_or_else(|(error, ..)| panic!("posts: {error:?}"));
    assert_eq!(host.held_by(owner), 2);
    host.finish_retire(owner);
    assert_eq!(
        host.held_by(owner),
        0,
        "a retired generation holds nothing, which is what lets it be unloaded"
    );
}
