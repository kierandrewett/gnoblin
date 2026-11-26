# Session locking

Gnoblin uses the standard `ext-session-lock-v1` Wayland protocol in the
Gnoblin session. The compositor owns the security boundary: it covers every
output, stops normal input, refuses screen capture, and stays locked if a locker
exits. The locker owns its appearance and
authentication. See the [protocol XML](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/ext-session-lock/ext-session-lock-v1.xml).

The lock screen is a separate client. Bingux is one shell project with its own
lock client.

Third-party lockers such as hyprlock, swaylock, gtklock and waylock can connect
directly to the same standard protocol. Any client with access to this user's
Wayland socket can request the lock; Gnoblin does not filter requests by
process. The first request owns the lock, and concurrent requests receive
`finished`.

In Bingux, use its Lock action or run `bingux-lock`. With another locker
installed, run its command (for example, `hyprlock`) in your Gnoblin session.
Your shell or idle daemon decides when to run that command; Gnoblin does not
set a timeout or launch a locker for you.

The protocol is enabled by default in Gnoblin, and your shell file (`shell.ts`, `shell.py` or a Rust shell) cannot turn it off: see
[Settings with no object](/js-settings).

## Request a lock

A lock request asks your shell to show its lock screen. Gnoblin sends the
request as the `gnoblin.session.lock-requested` event to every shell client
that subscribed to it. The shell starts its locker, and the locker takes the lock with
`ext-session-lock-v1`. Gnoblin binds no key and starts no locker.

These sources send the request:

- `gnoblinctl session lock`
- a session lock call in your shell: `gnoblin.session.lock()` in TypeScript and Python, `session.lock()` on the `Session` object in Rust
- `org.freedesktop.ScreenSaver.Lock`, or `SetActive(true)`, on the session bus
- `loginctl lock-session` for your graphical session. Gnoblin locks when logind
  asks for the session lock, and locks before a suspend or hibernate: logind's
  `PrepareForSleep(true)` makes the lock happen before the sleep inhibitor is
  released.

A request does not prove the session is locked. The call reports whether a
shell received the request. Read `lock_state` in `gnoblinctl status` or
`gnoblin.session.status()` to see the real state.

If no shell is subscribed, the call fails and says so. `gnoblinctl` exits with
an error, and `ScreenSaver.Lock` returns the error
`no session-lock client is subscribed to lock requests`. A logind request writes
a warning with the same reason to the journal. Gnoblin leaves the session unlocked in this
case.

A request while the session is already locked succeeds and sends no second
event. `SetActive(false)` returns `NotSupported`, because only the locker can
unlock.

`ScreenSaver.GetActive` returns `false` for `unlocked` and `true` for
`covering`, `locked`, or `failsafe`. These are the compositor's
[lock states](/bridge/session-and-windows#track-lock-state); only `locked` confirms the
lock transition.

Subscribe to `org.freedesktop.ScreenSaver.ActiveChanged` on the session bus at
`/org/freedesktop/ScreenSaver` to receive that boolean when it changes. Read
`GetActive` for the initial value. Changes between active lock states do not
send duplicate signals. Use `gnoblin.session.lock_state_changed` when you need
the individual state transitions.

If the compositor connection is interrupted, Gnoblin sends no unlock signal.
It reconnects and reports a changed boolean from the current state. After the
idle service restarts, read `GetActive` again for its initial value.

To lock with a key of your choice, bind a key in your shell:

::: tabs

```typescript
gnoblin.input.primary.bind("Super+l", () => {
    gnoblin.session.lock();
});
```

```python
from gnoblin import gnoblin
from gnoblin.objects.model import Gnoblin


@gnoblin.bind("<Super>l")
def lock():
    Gnoblin.get().session.lock()
```

```rust
use gnoblin_core::objects::ModelRoot;
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let Some(session) = model.session()? else {
        return Ok(());
    };
    g.bind("<Super>l", move || {
        if let Err(error) = session.lock() {
            g.log(&format!("cannot lock the session: {error}"));
        }
    })?;
    Ok(())
}
```

:::

Shells do not receive `gnoblin.session.lock-requested`. They see the outcome instead: listen
for `gnoblin.session.lock_state_changed`, which reports `locked` once the lock took (see
[Lifecycle events](/events/lifecycle#session)).

A shell process that connects to the [compositor bridge](/bridge/) subscribes
to `gnoblin.session.lock-requested` and starts its locker.

Check the result from a terminal:

```sh
gnoblinctl session lock
gnoblinctl status
```

`lock_state` changes from `unlocked` to `locked` after the locker takes the
lock.

## The lock node

The compositor's tree has one `lock` node, made at startup. Its `state` is `not-locked`, `pending` (a lock was taken and
the cover is not proven on every output yet) or `locked`.

Everything that reads the lock reads this node: the stock lock
screen, the ScreenSaver service (`GetActive`, `ActiveChanged`), the capture embargo of the D-Bus compatibility adapters,
and `gnoblinctl status`. `pending` already counts as locked for all of them, so nothing captures the desktop while the
cover goes up.

`Lock:lock()` takes the lock:

- It has no answer of its own. The confirmation is `state` reaching `locked`; a lock that falls back to `not-locked` did not take.
- The ScreenSaver service and `gnoblinctl lock` use it. A second call while the session is `pending` or `locked` changes nothing.
- Only the session owner's connection can read the node or call it. Any other connection is refused `not-owner`.
- A reader that is not the shell (the service, an adapter, `gnoblinctl`) can take the lock and can never unlock it.
- Unlocking goes through the authenticated check below, or through the locker's own `ext-session-lock-v1` request.

There is no `gnoblin.lock` object (`locked`, `lock_session`); a client that sends one is refused `unknown-member`.

## Password check (PAM)

The stock lock screen unlocks through the compositor's `lock` node: `Lock:unlock(password)` calls `authenticate`, and the
compositor checks the password for the session user on a worker thread through the PAM service `gnoblin-lock`. The
session unlocks only when PAM accepts. Every other answer keeps it locked:

| Answer         | Meaning                                                                                        | Session |
| -------------- | ---------------------------------------------------------------------------------------------- | ------- |
| `ok`           | PAM accepted the password and the account.                                                     | unlocks |
| `denied`       | Wrong password or unknown user. Three denials in a row delay the next attempt by 2 s.          | locked  |
| `rate-limited` | The delay is running; the answer says how many milliseconds remain.                            | locked  |
| `failed: ...`  | PAM could not decide: it is missing, or `pam_acct_mgmt` refused the account (expired, denied). | locked  |

An empty password is answered `denied` without a check, so an account with no password cannot be opened with a bare Enter. A password
over 1024 bytes is answered `failed: the password is longer than 1024 bytes` and no check starts.

A `failed` check logs one warning that names the failure. Neither the log nor the answer carries the password.

The service file `packaging/pam.d/gnoblin-lock` is installed to `lib/pam.d/gnoblin-lock` and holds two lines,
`auth include login` and `account include login`, so the lock accepts what a console login accepts. `login` is a valid
include target on each distribution below, checked as follows:

| Distribution | What `login` pulls in for `auth` and `account`                                                                                                | Evidence                                                                                                                                          |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| Debian       | `pam_faildelay`, `pam_nologin` (requisite), then `@include common-auth` and `@include common-account` (`pam_unix`)                            | Read from `/etc/pam.d/login`, `common-auth` and `common-account` on a Debian instance.                                                            |
| Ubuntu       | The same file: Ubuntu ships Debian's `util-linux` `login.pam`.                                                                                | Inferred from the Debian package. The file was not read on an Ubuntu system.                                                                      |
| Arch         | `login` includes `system-local-login`, which includes `system-login`: `pam_shells`, `pam_nologin`, `pam_access` (account), then `system-auth` | Read from the `pambase` sources (`system-local-login`, `system-login`) in the Arch packaging repository. The `login` file itself was not fetched. |
| Fedora       | `login` includes `system-auth` (and `postlogin`).                                                                                             | From distribution documentation only. The file could not be fetched, so this row is not verified.                                                 |

Reading the login stack has two consequences:

- `pam_nologin` is in it on Debian and Arch, so while `/etc/nologin` exists a non-root user cannot unlock, as at a
  console.
- A system with no `/etc/pam.d/login` makes every check `failed` and the session stays locked.

The real-PAM test is `#[ignore]`d and needs `GNOBLIN_PAM_TEST=1` and a test account.

The answers are tested through the compositor's lock path with a scripted authenticator, in
`tests/view_capture_e2e.rs` of `gnoblin-wayland`:

- `the_right_password_unlocks_the_session`
- `a_wrong_password_keeps_the_session_locked_and_repeated_ones_are_rate_limited`
- `a_check_that_cannot_run_keeps_the_session_locked_and_answers_without_the_password`

## The lock screen, and the lock card when there is none

When the compositor holds the lock itself, something has to take the password. The lock is the compositor's own when it
was taken through the `lock` node: `gnoblinctl`, the screensaver service or logind.

The stock JavaScript shell takes the password with a lock screen of its own. `data/js/gnoblin-std/lock.ts`, which a script
loads by importing `gnoblin-std`, draws a clock and a masked password entry over the cover, under the `lock` node:

- It comes up when `state` leaves `not-locked`. A shell that starts while the session is locked finds the state already
  there.
- It sets the node's `engaged` while it is up, sends the password with `authenticate`, and renews its lease on the lock
  screen with `heartbeat` (see "The lock card").
- It never decides anything: only an accepted check unlocks.
- A wrong password clears the entry and says `Wrong password.`, and a rate-limited one says how long to wait.
- An Enter pressed while a check runs is not a second attempt, and a key typed after the unlock is dropped.
- A check that is lost is not waited for. When the compositor refuses the transaction that carries it (`stale-base`: the
  password is wiped once sent, so it cannot go again) the screen is answered `The password could not be checked.` at
  once, and a check the compositor does not answer within 30 seconds is answered the same, so the next Enter is an
  attempt.
- The check goes in a transaction of its own, ahead of whatever else the shell wrote in that frame, so a refusal that a
  window write caused cannot cost it.

### The lock card

If no shell holds the lock screen, the compositor draws a lock card of its own over the cover. It shows a padlock, the time
of day, a masked entry and one line for the last attempt. Enter sends the password to the same PAM check, and a wrong
one is answered on the card.

The card is wanted when both of these hold:

- The lock is the compositor's own and the cover is proven (`locked`). An `ext-session-lock-v1` client's lock is that
  client's to draw and check, so the card is never drawn over it.
- No shell holds the lock screen: none holds the tree, or one does and has not set `engaged`, or one has and its lease
  has lapsed, or one has a fresh lease and the keyboard is not on its screen.

`engaged` alone is a claim. A shell that engaged and then hung, or whose lock frame throws every frame, would hold the
lock UI with nothing to type into. So holding the lock screen takes a lease and a keyboard.

The lease:

- The shell renews it with the lock node's `heartbeat` call, at most twice a second, and only after a healthy turn.
- A lease that is not renewed for two seconds lapses.
- A lock first seen engaged starts the lease, so a shell has two seconds to send its first beat.
- The call has no answer and changes no node, and only the shell that holds the tree may send it.

A turn is healthy when:

- the screen is up and its frame ran to its end;
- every canvas of the screen is still in the tree;
- the toolkit drew all of them: no refresh, arrange or draw of one failed, such as a `draw()` that throws or a write the
  compositor refused.

A turn that failed to draw renews nothing, and the first healthy turn after it renews again. A key handler that throws is
reported and does not make a turn unhealthy.

The keyboard:

- The keyboard is on the screen when the seat's focus is a node under the `lock` node that takes input: the entry.
- A shell with a fresh lease and the keyboard elsewhere holds nothing. No key reaches the screen, and a healthy shell is
  no guarantee that anybody can type a password.
- A window that arrives under the lock is given the focus by the focus policy, and a script can move it too. The screen
  takes the keyboard back on its next frame, and the card takes the keys after the grace below if nothing does.

A shell that is attached gets one second to engage or renew before the card is drawn, and the same second after it lets
go or loses the keyboard, so a reload does not flash the card.

- A shell that disconnects releases `engaged`, so the card is back at once, and the next shell starts from a lock nobody
  holds.
- A shell that hangs while holding the lock screen loses it to the card after three seconds.
- A shell that renews its lease and has no keyboard on its screen loses it to the card after one second.
- The shell takes the screen back, and the card is dropped, when it renews its lease and the keyboard is on its screen.

When a shell engages the lock, the card is dropped with anything typed into it, and the shell's screen is drawn from the
same frame. The session stays locked throughout and the desktop is never drawn.

The card cannot be reached from a script:

- It is not a tree node.
- It is the last layer of the frame, above the shell's own lock screen.
- It has no Escape key or button.
- While it is up, every key goes to it and to no shell node, client or binding.

The cover and the input gate are unchanged. The card's colours, radius and font follow `dialogs.style`, clamped like the
other built-in dialogs. The time of day is read from `$TZ` or `/etc/localtime`, including the rule at the end of a TZif
file. A zone the card cannot read shows UTC.

### A shell on gnoblin-core only

The stock lock screen is `gnoblin-std/lock`. A shell that imports only `gnoblin-core` loads no std module, so it draws no
lock screen. The card is in core on purpose: a locked session whose shell does not draw
a lock screen is never a black screen with no way back.

What a core-only shell must know about the lock is nothing:

- It does not draw a lock screen, set `engaged`, send `heartbeat` or call `authenticate`. It holds the tree and places its
  windows, and the lock is not its business.
- The lock is taken, the cover is proven, and the card takes the password. A shell that holds the tree gets the one second
  of grace every shell gets, and a session with no shell attached gets the card at once.
- A wrong password and an empty one never unlock the session, and the right one does. No key reaches a window while the
  session is locked, and the window that had the keyboard hears keys again after the unlock.
- The shell is told nothing of the password. The card keeps it in a wiped buffer on the compositor's side, the check
  runs there, and nothing the compositor sends the shell, a node, an event or an answer, carries it.

A core-only shell that wants a lock screen of its own can draw one: the same contract as the stock screen (the nodes, the
`engaged` flag, the lease and the keyboard above) is open to it, and the card steps aside while it holds.

The card takes the password after the grace for a shell that holds the tree, and at once when no shell is attached, in both
cases plus the half second below.

### What this protects, and what it cannot

The lock stops other people at the keyboard. It does not protect the session from the shell. The shell is a process of
your own: your session starts it, it runs your shell file, and whatever can read its memory can already read your keyring,
your agent sockets and every other password you type.

Who can see what you type at a locked session:

- Someone at the keyboard sees a mask. A wrong password and an empty one never unlock the session, through the shell's
  screen or the card: the check and its rate limit are the compositor's.
- A window of your session hears nothing. No key reaches a window while the session is locked.
- No other node of the shell hears a key. A node that holds a keyboard or pointer grab loses it when the session locks, and
  a grab is refused while the session is locked unless the node is part of the lock screen the shell has up.
- A program that reads the tree, such as `gnoblinctl`, sees nothing. The password is the `authenticate` call's secret
  text and no property holds it.
- The logs hold nothing. A failed check logs one warning without the password.
- The compositor sees every key and runs the check, through the shell's screen or the card.
- The shell, and the modules your shell file imports, see every key and the password as one string for the check, when the
  shell's screen is the entry. They see nothing when the card is.

A module you import that wants the password can have it. Replace the stock screen only with code you trust, and keep
modules you did not read out of a shell file you use on a machine you lock.

The compositor's card keeps the password in a wiped buffer from the first key. It is the entry whenever the shell's
screen is absent, not alive or not answering.

The entry keeps what is typed as an array of code points, not as a string, and overwrites the array when the call is
made. The one string the password is built into lives for the call to `authenticate`.

A JavaScript string cannot be wiped. That copy, and the one in each key event, stay in the script engine's heap until the
collector frees them, and the engine may have copied them again.

### When the shell fails

The cover and the unlock check are the compositor's, so the session stays locked whatever the shell does, and a wrong
or an empty password never unlocks it. What the shell's failure changes is who takes the password:

- The shell hangs, is frozen, spins in a handler or a reload, or throws in its lock frame: the card takes the password
  3 seconds after the last beat (a 2 second lease and 1 second of grace).
- The shell crashes or is killed: the card takes it at once, and the restarted shell takes the screen back.
- The shell is slow but renews inside 2 seconds: the shell's screen takes it, and the card never appears.
- The shell is slower than that for a stretch and then recovers: the card takes it for the stretch, and the screen takes
  it back.

The three seconds do not depend on the shell's budgets. A frame may use 5 seconds and a module load 30. The tests spin a
script for 10 seconds, longer than the bound, and the card unlocks the session inside the spin.

A healthy shell that has lost the keyboard is the one failure no beat shows. A window that arrives under the lock takes
the focus from the focus policy, and its keys go nowhere the screen can read. The shell keeps renewing its lease, so the
card would stay away and nobody could unlock the session.

So the lock screen counts as held only while the keyboard is on its entry, and the card takes the keys one second after
it is not. The bounds, from the instant of the event to the instant the password is accepted:

- The card takes the password at most 3 seconds, plus half a second, after the last healthy beat.
- The card takes it at most 1 second, plus half a second, after the keyboard left the screen.

The half second is what lies between the grace running out and the password accepted:

- The session's clock moves on a loop turn of at most 100 ms, and the lapse and the grace are each noticed on a turn.
- The card's pump runs every 50 ms. It draws the card, takes the keys and sends the check.
- The answer to the check is applied on the next turn.

The clock counts the whole milliseconds between two turns and keeps the rest for the next turn, so 3 seconds of the
session's clock are 3 seconds of the wall clock.

A clock that dropped the rest of every turn ran slow by half a millisecond a turn on average. While the test typed,
3 seconds of that clock took 3.2 of the wall clock's.

`crates/gnoblin/tests/js_lock_bound.rs` measures both from outside the compositor and asserts them. In a release build
on a 2-CPU box, 20 runs each:

| From                                   | Min     | Median  | Max     |
| -------------------------------------- | ------- | ------- | ------- |
| the last healthy beat, to the password | 3.095 s | 3.143 s | 3.165 s |
| the keyboard leaving, to the password  | 1.121 s | 1.137 s | 1.149 s |

A debug build, which is what `cargo test` runs, is slower:

- The pump that draws the card takes about 110 ms there, and it draws twice: when the card appears, and when the first
  keys reach it.
- Sixty runs on the same box took 3.294 s to 3.456 s from the last beat and 1.275 s to 1.425 s from the keyboard
  leaving, so the test allows a second there.
- With both CPUs kept busy by two loops of their own, 48 runs took up to 3.731 s and 1.909 s: inside that second, but
  not far inside it.
- The other lock tests allow 5 seconds more, for a machine that other tests keep busy.

The compositor does not wait for the shell. Its tree answers a second connection within milliseconds while the shell is
inside a call, and while a frozen shell's socket is not read.

### Tests

- `crates/gnoblin-js/tests/lock.rs` runs against a fake compositor:
    - the screen engages on a lock, waits for a screen, shows to a shell attached to a locked session, and comes down on an
      unlock and on `ok`;
    - the password is sent as the call's secret text and never an argument, and a wrong, busy, failed and rate-limited check
      is answered with the entry cleared;
    - the clock, the focus given back, an Enter during a check and keys after the unlock;
    - the password is in no node of the tree.
- `crates/gnoblin-js/tests/write_refusals.rs` runs the same screen over the shared fake compositor, which can refuse a
  transaction as the real one does:
    - a check lost to a `stale-base` refusal is answered `failed` and the next Enter is an attempt;
    - a check goes first and alone, and a refusal of the writes beside it leaves it waiting for its answer;
    - a check nobody answers fails after 30 s;
    - the screen renews its lease twice a second while it is up and not after it comes down;
    - a lock frame that throws renews nothing, and neither does a canvas of the screen that fails to draw every frame or
      a screen whose canvases the compositor destroyed, and the lease comes back when the screen draws again;
    - a key handler that throws does not stop the lease;
    - a window that takes the focus while the screen is up loses it to the screen and has it back when the screen comes
      down.
- `crates/gnoblin-compositor/tests/lock_card.rs` runs against the real compositor with a scripted check in place of PAM:
    - with no shell the right password unlocks the session;
    - a wrong one does not, and the card still takes the right one after it;
    - a shell that holds the lock screen keeps the card away, and one that dies brings it back;
    - a shell that attaches while the card is up takes over;
    - a shell that stays attached and `engaged` but stops renewing its lease loses the lock UI to the card;
    - a shell that renews its lease on time but whose keyboard is outside its lock screen loses it to the card, and the
      card gives the keys back when the shell takes the keyboard again.
- `crates/gnoblin-compositor/tests/inproc_shell.rs` runs a shell that lives in the compositor process, on a thread of its
  own, against the same card: a shell thread that builds its lock screen, renews its lease once and then hangs holds up
  no frame (the session locks, the lease lapses and the card takes the password), and the hung shell is then detached
  because it never reads its mailbox.
- `crates/gnoblin/tests/js_lock.rs` runs the JavaScript shell against the real compositor: no shell and the card, the
  shell's screen painted under the `lock` node, a wrong password answered on it, the right one unlocking, and a check the
  compositor refused `stale-base` answered `failed` on the screen.
- `crates/gnoblin/tests/js_lock_failures.rs` fails the shell against the real compositor. For each failure it asserts that
  the session stays locked, that a wrong and an empty password never unlock it, that the right one does, and how long
  the card took. The failures:
    - a shell that hangs;
    - one whose lock frame throws every frame;
    - one that crashes;
    - a slow shell that renews in time, and one that does not;
    - a script that spins in a frame handler, one that spins in a reload, one that spins when a window arrives, and one
      that spins while windows arrive and go (10 seconds each, every read of the tree timed).

    A spin test also reads, from outside, how long the shell's thread has been inside one tick. It fails unless the shell
    was still in the spin when the card took the password, because a script that never spun leaves the shell's own screen
    to take it, which passes every other assertion.

    Its second half is the windows. A healthy shell, which renews its lease on time, must still leave the keys with a
    taker in each of these cases:

    - a window arrives under the lock, or five do;
    - one arrives and is closed;
    - a client opens a window and never draws, and one never commits at all;
    - a window maps as the lock is taken, or was open before it;
    - windows map while wrong passwords are typed, with the stock focus policy writing the focus in the same turns.

    A healthy shell that keeps the keyboard from its own screen on every frame gets the card within the grace. A shell
    that crashes with a window open under the lock leaves the card to take the password at once, and the window hears
    none of it.

- `crates/gnoblin/tests/js_lock_boundary.rs` pins what the lock keeps back:
    - a window that had the keyboard hears nothing typed at the shell's screen or at the card;
    - the password is in no node of the tree, typed or checked;
    - after an unlock, when a window arrived under the lock, the window that had the keyboard before the lock has it
      again, through the shell's screen and through the card;
    - a window that arrived under the lock when none had the keyboard before gets it.
- `crates/gnoblin/tests/js_lock_bound.rs` measures the takeover bounds above and asserts them, and the card's bounds with a
  shell that imports only `gnoblin-core` and with none.
- The tests of `TurnClock` in `crates/gnoblin-wayland/src/clock.rs` pin how the loop counts the session clock: turns
  shorter than a millisecond lose no time, and the count is never ahead of the wall clock.
- `crates/gnoblin/tests/js_lock_core_only.rs` runs the real compositor with a shell that imports only `gnoblin-core` and
  holds the tree, and with no shell:
    - no lock screen is drawn and the card takes the password;
    - an empty and a wrong password never unlock the session, and the right one does;
    - a window that had the keyboard hears none of it, and hears keys again after the unlock;
    - nothing the compositor told a core-only shell, and no node of the tree, holds the password or its bytes.
- `crates/gnoblin/tests/js_guardian.rs` kills the real runtime process while the session is locked (the session stays
  locked and the restarted shell holds the screen again) and freezes it with `SIGSTOP` while a window floods the tree with
  1.4 MB of title changes (the compositor's tree answers within 150 ms and the session stays locked).

The capture embargo, the input gate and the log are pinned where they live: `a_covered_client_receives_no_input` and
`a_scroll_while_locked_reaches_only_the_lock_surface` (`crates/gnoblin-wayland/tests/session_lock.rs`),
`the_lock_cuts_every_reader_of_a_view_and_none_is_served_a_picture_from_before_it`
(`crates/gnoblin-wayland/tests/view_capture_e2e.rs`), `a_capture_while_the_lock_is_being_established_is_refused_by_name`
(`crates/gnoblin-compositor/tests/capture_screen.rs`) and `a_failed_check_logs_one_warning_without_the_password`
(`crates/gnoblin-wayland/src/lock_auth.rs`).

Not proven:

- No test reads the output's pixels. That the card and the shell's screen are drawn over the cover is shown by the
  tree's paint lists and the compositor's frame path, not by a screenshot.
- The password check is a scripted double, not PAM. `GNOBLIN_PAM_TEST=1` runs the real one by hand.
- The real runtime process is killed and frozen, but no test types at it, because the real session has no key
  injection. The card taking a password after a kill is proved in-process.
- The bounds are measured on one kind of machine (2 CPUs) with the shell driven by the test, which stops ticking to
  hang. A release build was measured with nothing else running, and a debug build also with both CPUs kept busy by two
  loops. A slower machine, a real runtime process hung by a signal and a real keyboard were not measured.
- The card's first frame is not timed, because no test reads pixels: the interval ends at the password accepted.

## Portal and remote access while locked

Remote and portal streams are routed by the selected portal backend. While the
session is locked, a new capture request is refused, and a monitor stream that was
already running when the lock began is stopped: its client receives `stopped` and
no further frame. No desktop pixels are sent while the lock holds.

A View is covered by the same rule:

- A capture session on a View is stopped when the lock begins, and a PipeWire stream of one ends.
- The shell's `View:capture` is answered `locked` and writes no file.
- A session or a cast that starts while the session is locked is refused.
- When a View is drawn on the GPU (`compositor.renderer = gles`) the cover names no View, so the renderer gives back
  every View texture at the lock.
- After the unlock a reader sees the window as it is then.

The tests are `the_lock_cuts_every_reader_of_a_view_and_none_is_served_a_picture_from_before_it` (`gnoblin-wayland`,
`tests/view_capture_e2e.rs`, on both renderers) and
`a_locked_output_names_no_view_alive_so_the_renderer_gives_each_texture_back`.

Remote keyboard and pointer input is available after `locked` and lock-scene
presentation, and is routed only to the active lock surface. It is refused
during the transition and failsafe states. No lock-specific portal setting is
required.

## Locker requirements

A locker binds `ext_session_lock_manager_v1` and sends `lock`. The compositor
then sends `locked` or `finished` on the new lock object. `finished` means the
request was not granted; do not show an authentication UI for that lock.

After requesting the lock, create one `wl_surface` per output and pass each to
`get_lock_surface` before attaching or committing a buffer. An output can have
only one lock surface for this lock. For every `configure(serial, width,
height)`, acknowledge its serial before committing a non-null buffer with those
exact surface-local dimensions. If multiple configures arrive before a commit,
acknowledging the last one is sufficient.

Gnoblin draws the owner's lock surface over the cover on its output, cropped
from the surface's committed buffer. A lock surface with no committed buffer
shows the bare cover, so a locker that unmaps its surface sees the cover again.

The owner's lock surfaces receive frame callbacks while the session is locked.
Windows behind the cover receive none, so a client cannot render frames the
user cannot see. The lock surface is masked in screen captures, so the lock
screen's content does not leak into them.

| Interface                     | Request or event                    | Contract                                                                                                         |
| ----------------------------- | ----------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `ext_session_lock_manager_v1` | `destroy`                           | Release the manager object; existing lock objects remain valid.                                                  |
| `ext_session_lock_manager_v1` | `lock`                              | Create a lock request; it later receives `locked` or `finished`.                                                 |
| `ext_session_lock_v1`         | `destroy`                           | Cancel or retire a lock only before `locked`; destroy its lock surfaces too.                                     |
| `ext_session_lock_v1`         | `get_lock_surface(surface, output)` | Create the output's lock surface before the `wl_surface` has a role or buffer.                                   |
| `ext_session_lock_v1`         | `locked`                            | The lock scene has been presented and it is safe to treat the session as locked.                                 |
| `ext_session_lock_v1`         | `finished`                          | The compositor did not grant this lock request.                                                                  |
| `ext_session_lock_v1`         | `unlock_and_destroy`                | After authentication succeeds and `locked` was received, release the session lock.                               |
| `ext_session_lock_surface_v1` | `ack_configure(serial)`             | Acknowledge a configure before committing its buffer.                                                            |
| `ext_session_lock_surface_v1` | `configure(serial, width, height)`  | Resize to the exact output-local dimensions, acknowledge `serial`, then commit a matching buffer.                |
| `ext_session_lock_surface_v1` | `destroy`                           | Retire the lock surface. Removing one for an active output before unlock makes Gnoblin show a solid color there. |

After `locked`, only the owner calls `unlock_and_destroy`; destroying the lock
object instead is a protocol error. If the client exits immediately after
unlocking, send `wl_display.sync` and wait for its callback before disconnecting
so the compositor processes the request. A locker crash leaves the compositor
covered and locked; another client may acquire the lock afterwards.

The protocol rejects a null lock-surface buffer, a commit before the first
configure is acknowledged, a buffer with the wrong dimensions, duplicate
output surfaces, and destroying the active lock without unlocking. See the
[protocol XML](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/ext-session-lock/ext-session-lock-v1.xml)
for the complete error enums.
