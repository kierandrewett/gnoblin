# Access dialog lifecycle

This fixture reproduces gnoblin-8gx at the generic Access backend boundary.
It compiles the actual `access.c` and Request implementation. The dialog and
its invisible parent are real GTK windows on a private Broadway display.
Policy and external parenting are substitutes. Native session proof belongs
to the main integration lane.

## Reproduce and verify

The prepared portal source must contain the existing patches through 47,
including Access patches 20 and 46, before patch 49. Its `libgxdp` subproject
must also be present. The default is `subprojects/xdg-desktop-portal-gnome`.
Use `--upstream /path/to/prepared-portal` for a separate source tree, such as
when working in a clean worktree with uninitialised submodules.

The runner copies `access.c` and leaves that source tree unchanged.
`--source /path/to/access.c` overrides only the Access file. Each output
directory must be new.

```sh
python3 tests/portal-access-lifecycle/run.py /var/tmp/access-before --baseline
python3 tests/portal-access-lifecycle/run.py /var/tmp/access-after
python3 tests/portal-access-lifecycle/run.py /var/tmp/access-public-before --baseline --frontend
python3 tests/portal-access-lifecycle/run.py /var/tmp/access-public-after --frontend
python3 tests/portal-access-lifecycle/run.py /var/tmp/access-decisions --frontend --decisions-only
```

Baseline exit 1 is expected. The patched checks must exit 0. The public
frontend checks take about 52 seconds. They use the installed frontend;
the recorded reproduction used version 1.20.4. Run its `--version` option
before interpreting results on another system.

The runner needs Clang, GTK4/libadwaita and GIO development files,
`gdbus-codegen`, PyGObject, `patch`, `dbus-run-session` and
`gtk4-broadwayd`. The public check also needs `xdg-desktop-portal` and
`xdg-permission-store`. It does not install dependencies.

Each run has private D-Bus, Broadway and XDG directories. D-Bus service
activation is disabled. The public check starts its own frontend and
permission store. It does not operate host services or a VM. The screenshot
pixel provider is a failing stub; reaching it unexpectedly fails the check.

`build-command.txt`, `run.log` and process logs remain in the output directory.
ASan and UBSan are enabled. Leak detection is disabled for GTK process-global
allocations. The function-type sanitizer is disabled because generated
GObject signal marshallers invoke upstream callbacks through generic types.
Window counts separately verify that both dialog and parent are destroyed.

## Failure cases and results

The failure cases were specified before the patch:

- Owner Close must complete Access once and remove its Request and windows.
- Foreign Close must return AccessDenied and leave the owner's dialog usable.
- Caller loss, frontend connection loss and frontend name loss must clean up.
- Public Request removal after frontend timeout must clean up without Close.
- Concurrent completion paths must not reply twice or use freed state.
- Grant and Deny must destroy the invisible parent as well as the dialog.
- A live request must remain usable beyond the frontend's 25-second default.
- Cancellation must not store a denial without a user decision.

The fixture also closes a request while its public-object check is pending.
The cancelled callback must release its retained handle without touching the
completed dialog. Caller loss and frontend name loss are tested together.

The baseline leaves one mapped dialog after owner Close, accepts foreign
Close, and retains windows after caller or frontend loss. Normal responses
leave the invisible parent allocated.

The public frontend fixture keeps the requesting connection alive through
each 25-second timeout. Baseline attempts leave one then two mapped dialogs,
with two then four total GTK windows. The patched backend reaches zero
windows after each public Response(2).

Caller exit and dialog dismissal must
leave the permission store empty; explicit Deny must store one denial.
`--decisions-only` runs those three permission cases without the two timeouts.
These checks do not prove native
window-manager behaviour or screenshot capture.

## Frontend cancellation boundary

The authoritative sources are the tagged
[xdg-desktop-portal 1.20.4 Screenshot implementation](https://github.com/flatpak/xdg-desktop-portal/blob/1.20.4/src/screenshot.c)
and [Request implementation](https://github.com/flatpak/xdg-desktop-portal/blob/1.20.4/src/xdp-request.c).

`handle_screenshot_in_thread_func` holds the request mutex while it calls
Access synchronously, without a cancellable. The Access proxy retains the
default D-Bus timeout. The backend Request proxy is assigned only after
consent succeeds. On timeout, `send_response` emits Response(2) and unexports
the public Request. It does not call the Access backend's Close method.

Public Close and caller-loss handling normally forward Close through
`request->impl_request`. That proxy is not yet assigned during Screenshot
consent, and those handlers also need the request mutex. Thus backend Close
handling alone cannot repair this frontend path.

Patch 49 checks the public Request with asynchronous D-Bus introspection
for verified frontend requests. It permits one pending check per dialog,
then schedules another check after one second. A missing Request interface
or UnknownObject/UnknownMethod response ends the dialog. A transport error
alone does not establish removal. Completion cancels a pending check and
removes all watches and timers.

The frontend stores a denial for any nonzero numeric Access result. The patch
therefore returns a cancellation method error for owner Close, dialog dismissal
and request abandonment. The frontend's error path skips permission storage.
Explicit Allow and Deny retain their normal numeric results.

This interval controls detection latency, not consent expiry. The patch adds
no arbitrary dialog deadline. It also watches the backend caller's unique
name and the original caller encoded in the public handle. Only the verified
frontend receives public-object and frontend-name monitoring. An encoded
caller name can cancel only its own dialog; it confers no permission.

A direct backend caller's local method timeout has no D-Bus cancellation
signal. Such a caller must send owner Close or disconnect. The patch does
not infer an expiry for it. GNOME Shell delegation is unchanged.

Public Screenshot Close can still wait for the frontend's synchronous Access
call to finish or time out. Removing that delay requires a frontend change.
Main owns native proof and tracking of that separate frontend limitation.
