//! A shell with an event loop of its own, in the smallest form: a `poll(2)` loop on the descriptor `Gnoblin::wake_fd` gives it.
//!
//! Two threads post work to the shell thread through a [`Poster`](gnoblin_core::Poster). The loop sleeps in `poll` on the wake
//! descriptor and calls `dispatch(0)` only when it is readable: it never waits inside the core, and it must still run every post,
//! and then sleep again (a descriptor that stayed readable after a drain would be a loop that spins). A watcher thread outlives `init` and keeps asking the poster for a descriptor: once the generation retires the poster is
//! refused, and the thread says so. The lines it prints say what happened; `crates/gnoblin-core-impl/tests/scenarios.rs` reads them.

use std::io::Write as _;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use gnoblin_core as gnoblin;
use gnoblin_core::Gnoblin;
use rustix::event::{PollFd, PollFlags, poll};
use rustix::time::Timespec;

/// How many posts run, over all the threads.
const POSTS: u32 = 40;

/// How many posts have run.
static RAN: AtomicU32 = AtomicU32::new(0);

fn say(line: &str) {
    let _ = writeln!(std::io::stdout().lock(), "{line}");
}

/// Whether `fd` became readable within `wait`.
fn readable(fd: &std::os::fd::OwnedFd, wait: Duration) -> Result<bool, rustix::io::Errno> {
    let mut fds = [PollFd::new(fd, PollFlags::IN)];
    let timeout = Timespec {
        tv_sec: i64::try_from(wait.as_secs()).unwrap_or(i64::MAX),
        tv_nsec: i64::from(wait.subsec_nanos()),
    };
    poll(&mut fds, Some(&timeout)).map(|ready| ready > 0)
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> Result<(), Box<dyn std::error::Error>> {
    let fd = g.wake_fd()?;
    let poster = g.poster();
    let threads: Vec<_> = (0..2)
        .map(|_| {
            std::thread::spawn(move || {
                for _ in 0..POSTS / 2 {
                    std::thread::sleep(Duration::from_millis(3));
                    // The closure runs on the shell thread, in `dispatch`; a refusal is a post after the generation retired.
                    let _ = poster.post(|_| {
                        RAN.fetch_add(1, Ordering::SeqCst);
                    });
                }
            })
        })
        .collect();

    let (mut wakes, mut callbacks, mut spurious) = (0_u32, 0_u32, 0_u32);
    let deadline = Instant::now() + Duration::from_secs(4);
    while RAN.load(Ordering::SeqCst) < POSTS && Instant::now() < deadline {
        if !readable(&fd, Duration::from_millis(500))? {
            continue;
        }
        wakes += 1;
        let ran = g.dispatch(0)?;
        if ran == 0 {
            spurious += 1;
        }
        callbacks += ran;
    }
    for thread in threads {
        let _ = thread.join();
    }
    // Everything has run and been drained: the descriptor is quiet, so a loop would sleep now.
    let quiet = !readable(&fd, Duration::from_millis(150))?;
    say(&format!(
        "[shell-wake] ran={} callbacks={callbacks} wakes={wakes} spurious={spurious} quiet={quiet}",
        RAN.load(Ordering::SeqCst)
    ));
    // Any thread may ask for the descriptor while the generation lives; after it retires the poster is refused for good.
    std::thread::spawn(move || {
        loop {
            match poster.wake_fd() {
                Ok(_) => std::thread::sleep(Duration::from_millis(5)),
                Err(error) => {
                    say(&format!("[shell-wake] stale poster refused kind={:?}", error.kind()));
                    return;
                }
            }
        }
    });
    Ok(())
}
