# ScreenSaver reply ownership regression

Run the leak-enabled test from the repository root:

```sh
python3 tests/idle-reply-leak.py --output /var/tmp/idle-reply-candidate
```

Use a new output directory for each run. The test needs Clang with its
AddressSanitizer runtime, GLib/GIO and JSON-GLib development files,
`dbus-daemon`, and Python with PyGObject. `--cc` selects another compatible
compiler.

The fixture includes the real `src/idle.c` and exports its method handler on a
private D-Bus daemon. A separate GNOME ScreenSaver fixture supplies replies.
The C harness adds a test-only `Quit` method so it can close its connection and
exit normally for LeakSanitizer. It does not start the production idle main
loop, contact logind, or change a host service.

Each cycle checks empty replies from `SetActive(false)` and `Lock`, plus
boolean replies from `GetActive` before and after locking. Three additional
calls check the existing delegated error responses. The default 100 cycles
therefore make 403 delegated calls.

## Compare an earlier source file

Use `--source` to build the same harness against a saved pre-change `idle.c`:

```sh
python3 tests/idle-reply-leak.py \
  --source /path/to/previous/idle.c \
  --output /var/tmp/idle-reply-baseline
```

The runner always enables ASan/UBSan and leak detection. It supplies no
LeakSanitizer suppressions and overrides inherited sanitizer options.
Leak reports make the fixture exit with status 23 and the runner fail.

The output directory retains the binary, source hash, compiler command,
functional results, delegated error details, process exit status, and full
sanitizer log. Use `--cycles 1` for a short comparison or `--cycles 100` for
the repeated-call check. A passing run exits zero with an empty sanitizer log.

## Ownership contract

[`g_dbus_connection_call_sync`](https://docs.gtk.org/gio/method.DBusConnection.call_sync.html)
returns an owned, non-floating `GVariant`.
[`g_dbus_method_invocation_return_value`](https://docs.gtk.org/gio/method.DBusMethodInvocation.return_value.html)
consumes a variant only when it is floating. The method handler must retain
its automatic cleanup after forwarding the reply.

This regression checks reply ownership and D-Bus behavior. It does not measure
historical desktop memory growth or establish a cause for an OOM event.
