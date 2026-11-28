//! Example 23: the session services of `libgnoblin-core`.
//!
//! The things a shell asks of the session are objects of the core, not functions of a table: a program it starts, the time the user has
//! been away, the application it waits for, the session's lock. This shell uses them the way a desktop would:
//!
//! - starts a command line and logs how it ended and what it printed;
//! - watches the session for 300 milliseconds of idleness and logs when it is idle and when the user is back;
//! - begins a launch of an application that never opens a window, and logs that it timed out;
//! - asks for the session lock with `<Super>e` and logs how the request settled;
//! - logs the build identity it reads.
//!
//! Every line it logs starts with `services:`. A signal is told from the library's turn, after the call or the change that caused it. Build it
//! with `cargo build -p gnoblin-example-native-services` and run it with `gnoblin ./target/debug/libnative_services.so`.

use gnoblin_core as gnoblin;
use gnoblin_core::{LaunchOptions, Gnoblin, SpawnOptions, StreamMode};

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin::Res<()> {
    let objects = g.objects()?;
    let model = Gnoblin::get(&objects)?;

    let version = model.version()?;
    g.log(&format!(
        "services: gnoblin {} speaks api {}",
        version.gnoblin, version.api
    ));

    // A program: the end is told from the library's turn, with how it ended and what it printed.
    let process = model.shell(
        "printf hello; exit 3",
        Some(&SpawnOptions {
            stdout: Some(StreamMode::Capture),
            ..SpawnOptions::default()
        }),
    )?;
    process
        .on_exited(move |process| {
            let exit = process.exit().ok().flatten();
            g.log(&format!(
                "services: the process ended with code {} and printed {:?}",
                process.exit_code().ok().unwrap_or(-1),
                exit.and_then(|exit| exit.stdout).unwrap_or_default()
            ));
        })?
        .keep();

    // A launch that no window answers.
    let launches = model.launches()?;
    let launch = launches.begin(
        "never",
        "org.example.Never",
        Some(&LaunchOptions {
            timeout_ms: Some(100),
            ..LaunchOptions::default()
        }),
    )?;
    launch
        .on_timed_out(move |launch| {
            g.log(&format!(
                "services: the launch {} timed out",
                launch.token().unwrap_or_default()
            ));
        })?
        .keep();

    // An idle watch: idle after 300 ms without input, and resumed by the next key.
    let idle = model.idle()?;
    let watch = idle.watch(300.0, None)?;
    watch.on_idle(move |_| g.log("services: the session is idle"))?.keep();
    watch
        .on_resume(move |_| g.log("services: the session is active again"))?
        .keep();

    // The session lock, asked for by a chord: the operation settles when the compositor answers.
    let session = model.session()?;
    g.bind("<Super>e", move || match session.lock() {
        Ok(operation) => {
            let heard = operation.on_settled(move |settled| {
                g.log(&format!(
                    "services: the lock request settled, {}",
                    if settled.error().unwrap_or_default().is_empty() {
                        "accepted".to_owned()
                    } else {
                        format!("refused with {}", settled.error_code().unwrap_or_default())
                    }
                ));
            });
            match heard {
                Ok(listener) => listener.keep(),
                Err(error) => g.log(&format!("services: cannot hear the lock request: {error}")),
            }
        }
        Err(error) => g.log(&format!("services: cannot ask for the lock: {error}")),
    })?;

    g.log("services: running");
    Ok(())
}
