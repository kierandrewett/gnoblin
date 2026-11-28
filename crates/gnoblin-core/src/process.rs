//! Programs and timers.
//!
//! A shell starts programs (`g.spawn("foot")`) and times things (`g.after(500, ...)`). Both belong to the generation that made
//! them. A program the shell's entry function starts waits until the generation takes over, so a reload that fails starts nothing,
//! and one a reload's generation starts again with the same arguments is adopted instead of doubled; a program a callback starts
//! belongs to the user and a reload leaves it running. A timer is removed with its generation.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering;

use gnoblin_core_abi::{self as abi, GnoblinProc, GnoblinProcessExit, GnoblinSpawn, GnoblinStr};

use crate::callback::CallbackResult;
use crate::error::{CoreError, Result, check};
use crate::events::Slot;
use crate::gnoblin::{CALLS, CallbackSlot, Host, Subscription, copy_str, drop_slot};

/// What a standard stream does with the program's output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stream {
    /// Writes to the compositor's own output, which the session log records.
    #[default]
    Inherit,
    /// Drops the output.
    Discard,
    /// Keeps the output, up to the output limit, for [`ProcessExit`].
    Capture,
}

impl Stream {
    fn raw(self) -> u32 {
        match self {
            Stream::Inherit => abi::STREAM_INHERIT,
            Stream::Discard => abi::STREAM_DISCARD,
            Stream::Capture => abi::STREAM_CAPTURE,
        }
    }
}

/// A signal to send a program's process group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// Ask it to end (`SIGTERM`).
    Term,
    /// End it at once (`SIGKILL`).
    Kill,
    /// Interrupt it (`SIGINT`).
    Int,
    /// Hang up (`SIGHUP`).
    Hup,
    /// `SIGUSR1`.
    Usr1,
    /// `SIGUSR2`.
    Usr2,
    /// Stop it (`SIGSTOP`).
    Stop,
    /// Continue it (`SIGCONT`).
    Cont,
}

impl Signal {
    fn name(self) -> &'static str {
        match self {
            Signal::Term => "term",
            Signal::Kill => "kill",
            Signal::Int => "int",
            Signal::Hup => "hup",
            Signal::Usr1 => "usr1",
            Signal::Usr2 => "usr2",
            Signal::Stop => "stop",
            Signal::Cont => "cont",
        }
    }
}

/// How a program ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessExit {
    /// The exit status after a normal exit.
    pub code: Option<i32>,
    /// The signal that ended it, after it was killed.
    pub signal: Option<String>,
    /// Whether the timeout ended it.
    pub timed_out: bool,
    /// What captured standard output held.
    pub stdout: String,
    /// What captured standard error held.
    pub stderr: String,
    /// Whether standard output passed the output limit and the rest was dropped.
    pub stdout_truncated: bool,
    /// Whether standard error did.
    pub stderr_truncated: bool,
}

impl ProcessExit {
    /// Whether it exited normally with status 0.
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// A program the shell started.
#[derive(Clone)]
pub struct Process {
    g: Host,
    handle: GnoblinProc,
}

impl std::fmt::Debug for Process {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Process").field(&self.handle).finish()
    }
}

impl Process {
    /// Sends a signal to the program's process group. A program that has ended is `Stale`.
    pub fn signal(&self, signal: Signal) -> Result<()> {
        let name = signal.name();
        // SAFETY: `proc_kill` reads `len` bytes at the name for the call.
        check(unsafe { (self.g.api().proc_kill)(self.g.ctx(), self.handle, name.as_ptr(), name.len()) })
    }

    /// Asks the program to end.
    pub fn terminate(&self) -> Result<()> {
        self.signal(Signal::Term)
    }

    /// Ends the program at once.
    pub fn kill(&self) -> Result<()> {
        self.signal(Signal::Kill)
    }
}

/// What runs when a program ends.
type ExitHook = Box<dyn FnMut(&ProcessExit)>;

/// A program to start: its arguments, environment and what to do with its streams.
///
/// ```ignore
/// g.command("sh").arg("-c").arg("notify-send hi").on_exit(|exit| { /* ... */ }).spawn()?;
/// ```
pub struct Command {
    g: Host,
    argv: Vec<String>,
    cwd: String,
    env: Vec<String>,
    stdin: Option<String>,
    stdout: Stream,
    stderr: Stream,
    output_limit: u64,
    timeout_ms: f64,
    on_exit: Option<ExitHook>,
}

impl Command {
    /// Adds one argument.
    pub fn arg(mut self, arg: impl Into<String>) -> Command {
        self.argv.push(arg.into());
        self
    }

    /// Adds arguments.
    pub fn args<I, S>(mut self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.argv.extend(args.into_iter().map(Into::into));
        self
    }

    /// Sets the working directory.
    pub fn cwd(mut self, dir: impl Into<String>) -> Command {
        self.cwd = dir.into();
        self
    }

    /// Adds a variable to the session's environment.
    pub fn env(mut self, name: &str, value: &str) -> Command {
        self.env.push(format!("{name}={value}"));
        self
    }

    /// Writes `text` to the program's standard input, then closes it.
    pub fn stdin(mut self, text: impl Into<String>) -> Command {
        self.stdin = Some(text.into());
        self
    }

    /// Sets what standard output does.
    pub fn stdout(mut self, stream: Stream) -> Command {
        self.stdout = stream;
        self
    }

    /// Sets what standard error does.
    pub fn stderr(mut self, stream: Stream) -> Command {
        self.stderr = stream;
        self
    }

    /// Sets the most bytes of captured output kept.
    pub fn output_limit(mut self, bytes: u64) -> Command {
        self.output_limit = bytes;
        self
    }

    /// Ends the program's process group after `ms` milliseconds.
    pub fn timeout(mut self, ms: u32) -> Command {
        self.timeout_ms = f64::from(ms);
        self
    }

    /// Calls `f` once, on the shell thread, when the program ends.
    pub fn on_exit<F>(mut self, f: F) -> Command
    where
        F: FnMut(&ProcessExit) + 'static,
    {
        self.on_exit = Some(Box::new(f));
        self
    }

    /// Starts the program. A program the entry function starts waits for the generation to take over. `NotFound`-style failures
    /// (the program does not exist) come back as `Failed` with the system's reason in the message.
    pub fn spawn(self) -> Result<Process> {
        let Command {
            g,
            argv,
            cwd,
            env,
            stdin,
            stdout,
            stderr,
            output_limit,
            timeout_ms,
            on_exit,
        } = self;
        let words: Vec<GnoblinStr> = argv.iter().map(|word| GnoblinStr::borrowed(word)).collect();
        let env_words: Vec<GnoblinStr> = env.iter().map(|word| GnoblinStr::borrowed(word)).collect();
        let options = GnoblinSpawn {
            size: size_of::<GnoblinSpawn>() as u32,
            stdout: stdout.raw(),
            stderr: stderr.raw(),
            reserved: 0,
            cwd: GnoblinStr::borrowed(&cwd),
            env: if env_words.is_empty() {
                core::ptr::null()
            } else {
                env_words.as_ptr()
            },
            n_env: env_words.len(),
            stdin: stdin.as_deref().map_or(
                GnoblinStr {
                    ptr: core::ptr::null(),
                    len: 0,
                },
                |text| GnoblinStr {
                    // An empty string is not "no input": point at something non-null.
                    ptr: if text.is_empty() {
                        core::ptr::NonNull::<u8>::dangling().as_ptr()
                    } else {
                        text.as_ptr()
                    },
                    len: text.len(),
                },
            ),
            output_limit,
            timeout_ms,
        };
        let mut handle: GnoblinProc = 0;
        let slot = on_exit.map(|f| Box::into_raw(Box::new(Slot { f, g: g.clone() })));
        let (cb, user, drop_user): (Option<abi::GnoblinExitFn>, *mut c_void, Option<abi::GnoblinDropFn>) = match slot {
            Some(slot) => (
                Some(exit_tramp::<ExitHook>),
                slot.cast(),
                Some(drop_slot::<Slot<ExitHook>>),
            ),
            None => (None, core::ptr::null_mut(), None),
        };
        // SAFETY: `spawn` reads the argument strings, the options and what they point at for the call (all live locals of this
        // function) and adopts `user` on success.
        let rc = unsafe {
            (g.api().spawn)(
                g.ctx(),
                words.as_ptr(),
                words.len(),
                &options,
                cb,
                user,
                drop_user,
                &mut handle,
            )
        };
        if rc < abi::E_OK {
            if let Some(slot) = slot {
                // SAFETY: the core refused the spawn, so it has not adopted the slot and nothing else owns the box.
                drop(unsafe { Box::from_raw(slot) });
            }
            return Err(CoreError::from_code(rc));
        }
        if slot.is_some() {
            crate::gnoblin::REGISTERED.fetch_add(1, Ordering::Relaxed);
        }
        Ok(Process { g, handle })
    }
}

unsafe extern "C" fn exit_tramp<F: FnMut(&ProcessExit)>(user: *mut c_void, exit: *const GnoblinProcessExit) -> i32 {
    CALLS.fetch_add(1, Ordering::Relaxed);
    // SAFETY: `user` is the `Box<Slot<F>>` leaked by `spawn`, handed back unchanged on the shell thread and not freed before
    // `drop_slot` runs.
    let slot = unsafe { &mut *user.cast::<Slot<F>>() };
    // SAFETY: the core passes a pointer valid until this callback returns, or null.
    let Some(exit) = (unsafe { exit.as_ref() }) else {
        return abi::E_ERR;
    };
    let signal = copy_str(&exit.signal);
    let report = ProcessExit {
        code: (exit.has_code != 0).then_some(exit.code),
        signal: (!signal.is_empty()).then_some(signal),
        timed_out: exit.timed_out != 0,
        stdout: copy_str(&exit.stdout),
        stderr: copy_str(&exit.stderr),
        stdout_truncated: exit.truncated & 1 != 0,
        stderr_truncated: exit.truncated & 2 != 0,
    };
    match catch_unwind(AssertUnwindSafe(|| (slot.f)(&report))) {
        Ok(()) => abi::E_OK,
        Err(_) => abi::E_PANIC,
    }
}

unsafe extern "C" fn timer_tramp<F: FnMut()>(user: *mut c_void) -> i32 {
    CALLS.fetch_add(1, Ordering::Relaxed);
    // SAFETY: `user` is the `Box<CallbackSlot<F>>` leaked by `every` or `after`, handed back unchanged on the shell thread and not
    // freed before `drop_slot` runs.
    let slot = unsafe { &mut *user.cast::<CallbackSlot<F>>() };
    match catch_unwind(AssertUnwindSafe(|| (slot.f)())) {
        Ok(()) => abi::E_OK,
        Err(_) => abi::E_PANIC,
    }
}

impl Host {
    /// A program to start, with `program` as its name; add arguments and options and call [`Command::spawn`].
    pub fn command(&self, program: impl Into<String>) -> Command {
        Command {
            g: self.clone(),
            argv: vec![program.into()],
            cwd: String::new(),
            env: Vec::new(),
            stdin: None,
            stdout: Stream::Inherit,
            stderr: Stream::Inherit,
            output_limit: 0,
            timeout_ms: 0.0,
            on_exit: None,
        }
    }

    /// Starts `program` with no arguments, in a process group of its own. The name is looked up on the `PATH`; it is not run
    /// through a shell.
    ///
    /// ```ignore
    /// g.bind("<Super>Return", || { g.spawn("foot").ok(); })?;
    /// ```
    pub fn spawn(&self, program: &str) -> Result<Process> {
        self.command(program).spawn()
    }

    /// Starts `program` with `args`.
    pub fn spawn_with<I, S>(&self, program: &str, args: I) -> Result<Process>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.command(program).args(args).spawn()
    }

    /// Calls `f` once, on the shell thread, `ms` milliseconds from now. Remove the timer before it fires with
    /// [`Subscription::unsubscribe`]. `f` returns `()` or a `Result` (an `Err` is logged).
    pub fn after<F, R>(&self, ms: u32, f: F) -> Result<Subscription>
    where
        F: FnOnce() -> R + 'static,
        R: CallbackResult,
    {
        let g = self.clone();
        let mut f = Some(f);
        self.timer(ms, false, move || {
            if let Some(f) = f.take() {
                f().settle(&g, "after");
            }
        })
    }

    /// Calls `f` every `ms` milliseconds, until the subscription is removed or the generation retires. A shell that fell behind
    /// is called once for the periods it missed, not once for each. `f` returns `()` or a `Result` (an `Err` is logged).
    pub fn every<F, R>(&self, ms: u32, mut f: F) -> Result<Subscription>
    where
        F: FnMut() -> R + 'static,
        R: CallbackResult,
    {
        let g = self.clone();
        self.timer(ms, true, move || f().settle(&g, "every"))
    }

    fn timer<F>(&self, ms: u32, repeat: bool, f: F) -> Result<Subscription>
    where
        F: FnMut() + 'static,
    {
        let slot = Box::into_raw(Box::new(CallbackSlot { f }));
        // SAFETY: `slot` is a leaked box the core hands back to `timer_tramp::<F>` and `drop_slot::<CallbackSlot<F>>`; on failure
        // the core has not adopted it and `adopt` reclaims it.
        let reg = unsafe {
            (self.api().timer_add)(
                self.ctx(),
                ms,
                u32::from(repeat),
                timer_tramp::<F>,
                slot.cast(),
                Some(drop_slot::<CallbackSlot<F>>),
            )
        };
        self.adopt(reg, slot)
    }
}
