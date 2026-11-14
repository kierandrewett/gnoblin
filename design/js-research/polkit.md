# polkit authentication: who verifies the password

Question 6 of `popups-portals.md` asked who verifies a password in Gnoblin's polkit path. This note fixes the real
contract from polkit's source and then says what Gnoblin did, what it does now, and what is and is not proven.

Sources, all at polkit commit `a1a78d7a` (master, read 2026-11-14), under
`https://github.com/polkit-org/polkit/blob/a1a78d7a9bdabcf0d9e714c3261e1d1cb263e7b7/`:
`pbia.c` = `src/polkitbackend/polkitbackendinteractiveauthority.c`, `polkitagentsession.c` = `src/polkitagent/polkitagentsession.c`,
`polkitagenthelper-pam.c` and `polkitagenthelperprivate.c` = `src/polkitagent/`, `polkitauthority.c` = `src/polkit/polkitauthority.c`.

## 1. The contract

1. **polkitd calls the agent.** `BeginAuthentication(action_id, message, icon_name, details, cookie, identities)` and
   `CancelAuthentication(cookie)` are the agent's two methods ([`data/org.freedesktop.PolicyKit1.AuthenticationAgent.xml`][agent],
   lines 10 and 51). polkitd turns a group identity into its users before it asks (`pbia.c:2384-2407`), and puts the
   caller and subject pids in `details` (`polkit.caller-pid`, `polkit.subject-pid`, `pbia.c:2427-2428`).
2. **The agent does not verify.** `PolkitAgentSession` is the reference agent code. `polkit_agent_session_initiate`
   spawns `PACKAGE_PREFIX/lib/polkit-1/polkit-agent-helper-1 <user name>` (`polkitagentsession.c:599-607`) and writes the
   cookie as the first line of its standard input, "so it can't be seen by other processes" (`:629-633`).
3. **The helper runs PAM.** It is setuid root (`polkitagenthelper-pam.c:95-103`), reads the cookie (`polkitagenthelperprivate.c:50`),
   and runs the `polkit-1` PAM service for the named user (`polkitagenthelper-pam.c:142-182`). It writes one escaped
   line per event to standard output: `PAM_PROMPT_ECHO_OFF`, `PAM_PROMPT_ECHO_ON`, `PAM_TEXT_INFO`, `PAM_ERROR_MSG`
   (`:55,260-285`), and a last line, `SUCCESS` or `FAILURE` (`:225,234`). The agent answers a prompt with one line on
   its standard input (`polkit_agent_session_response`, `polkitagentsession.c:531-548`) and reads the lines back with
   `g_strcompress` (`:463-503`). Anything else the helper says completes the session as a failure (`:507`).
4. **The helper tells polkitd, not the agent.** After PAM succeeds, and before it prints `SUCCESS`, the helper calls
   `AuthenticationAgentResponse2(uid, cookie, identity)` on polkitd (`polkitagenthelper-pam.c:211`, via
   `send_dbus_message`, `polkitagenthelperprivate.c:82-130`; the libpolkit wrapper sends `Response2` with `getuid()`,
   `polkitauthority.c:1502-1510`). polkitd accepts that call from uid 0 only: "Only uid 0 may invoke this method"
   (`polkitbackendinteractiveauthority.c:2861-2867`). It looks the session up by uid and cookie, checks the identity was
   one it offered (`:2873-2897`) and sets `is_authenticated` (`:2902`).
5. **The agent's reply is not the decision.** When `BeginAuthentication` returns, polkitd takes
   `gained_authorization = session->is_authenticated` (`pbia.c:1942`). A reply of success from an agent whose helper never
   reached `Response2` leaves it false, and an error that is polkit's `Cancelled` marks the dialog dismissed (`:1925-1931`).
   [INFERENCE from the code: I did not run a real polkitd against Gnoblin.]
6. The task text described the agent calling `AuthenticationAgentResponse2`. The source says the setuid helper does:
   the agent is an unprivileged process and polkitd refuses it ([`authority.xml:316-336`][authority]: "intended only for
   use by a privileged helper process internal to polkit").

[agent]: https://github.com/polkit-org/polkit/blob/a1a78d7a9bdabcf0d9e714c3261e1d1cb263e7b7/data/org.freedesktop.PolicyKit1.AuthenticationAgent.xml
[authority]: https://github.com/polkit-org/polkit/blob/a1a78d7a9bdabcf0d9e714c3261e1d1cb263e7b7/data/org.freedesktop.PolicyKit1.Authority.xml

## 2. What Gnoblin did (before this change)

- `crates/gnoblin-service-polkit/src/lib.rs` answered `BeginAuthentication` from a provider's answer: an answer that
  was a non-empty identity became `Decision::Allowed` (`gnoblin-service-core/src/agent.rs`, `Decision::from_outcome`) and
  the call returned success. The module said "polkitd runs the authentication through its own helper" and held no
  secret. **Nobody ran the helper.** The compositor's `auth.polkit` slot let a shell claim the request, and the JS
  `gnoblin.authentication.setProvider` offered `request.allow(identity)` with no password check. With a real polkitd the
  `allow` could not grant anything (point 5), but nothing in Gnoblin could ever authenticate either, and the compat
  tests' fake polkitd treated the reply as the grant, which polkitd does not.
- With no claimant the offer went back to the services connection that made it and was declined: every request failed.
- The Lua page (`docs/shell-api/authentication.md`) said "Not usable" and the JS page copied its shape. A script
  deciding "the user authenticated as X" is the thing the owner ruled out (J8).

## 3. What Gnoblin does now

- **The compositor verifies.** `gnoblin-service-polkit` offers each `BeginAuthentication` on `auth.polkit` with a payload
  (`gnoblin-polkit-auth::AuthRequest`: action, message, identities, requester, cookie). The compositor holds it itself
  (`ipc/requests.rs`), draws `PolkitUi` as a secure `Modal` (`ipc/secret.rs`, `gnoblin-portal-consent/src/polkit.rs`),
  collects the password into a `SecretText`, and runs the helper (`ipc/polkit.rs`, `gnoblin-polkit-auth::HelperSession`)
  with the chosen identity and the cookie. It shows PAM's prompts and messages, asks again after a failure (at most 5
  attempts), and answers the request only on the helper's `SUCCESS`. The agent then ends `BeginAuthentication`; polkitd
  already has the helper's own call.
- **No script is in the path.** A shell cannot claim `auth.polkit` (the claim is refused) or offer to it, and nothing but
  the compositor's dialog can answer. `gnoblin.authentication.addListener` observes through `dialogs.observe`: a
  listener sees action, message, requesting program and identities (never the cookie) and may `decline`, which makes
  polkitd see "not authorized". There is no `allow` and no password access.
- **Cancel ends everything.** `CancelAuthentication` makes the agent withdraw the request, which closes the dialog and
  kills the helper; so do Escape, the request deadline, a departed agent and a session lock.
- The helper is `GNOBLIN_POLKIT_HELPER`, else the first of `/run/wrappers/bin`, `/usr/lib/polkit-1`,
  `/usr/libexec/polkit-1`, `/usr/lib/policykit-1` (polkit compiles its libexec in and does not report it). A missing helper
  is shown in the dialog and the request fails closed.

## 4. Proven, and not

Proven by tests that run with a fake helper (a script that speaks the protocol) and, for the D-Bus half, a fake polkitd on
a private bus plus a fake helper that makes the `Response2` call the way the real one does: the payload reaches the helper
with the cookie on standard input; success, wrong password then success, Escape, `CancelAuthentication`, the deadline, a
missing helper and too many failures each end as described; the password is in no log, `Debug` output or drawn frame; a
shell cannot claim, offer, answer or read the cookie. **Not proven:** that a real `polkit-agent-helper-1` accepts this
driving (I wrote it from the source, not against the binary), that a real polkitd accepts the outcome, PAM
modules such as fprintd (the dialog shows their `PAM_TEXT_INFO` and prompts; no fingerprint reader was used), and the
Nix setuid wrapper path. Treat those as open until a session on a machine with polkit runs `pkexec true`.
