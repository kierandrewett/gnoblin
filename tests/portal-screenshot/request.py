#!/usr/bin/env python3
"""Main-only VM probe of the standard public Screenshot/Request lifecycle.

Run inside the intended guest session. This makes a real screenshot request;
answer its dialogs in that VM. It never edits permissions or restarts services.
"""

import argparse
import json
from pathlib import Path
import shutil
import uuid
import gi

gi.require_version("GdkPixbuf", "2.0")
from gi.repository import Gio, GLib, GdkPixbuf  # noqa: E402 - require_version must run first

DESKTOP = "org.freedesktop.portal.Desktop"
PATH = "/org/freedesktop/portal/desktop"
REQUEST = "org.freedesktop.portal.Request"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new directory for result.json and a PNG on success")
    parser.add_argument("--interactive", action="store_true")
    parser.add_argument("--expect", choices=("success", "failure", "cancel", "closed"), default="success")
    parser.add_argument("--close-after-ms", type=int)
    parser.add_argument("--timeout", type=int, default=45)
    args = parser.parse_args()
    if (args.expect == "closed") != (args.close_after_ms is not None):
        parser.error("--expect closed requires --close-after-ms, and vice versa")
    args.output.mkdir(parents=True, exist_ok=False)
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    loop = GLib.MainLoop()
    token = "gnoblinshot" + uuid.uuid4().hex
    handle = PATH + "/request/" + bus.get_unique_name()[1:].replace(".", "_") + "/" + token
    responses = []
    outcome = {"expected": args.expect, "interactive": args.interactive, "handle": handle}
    errors = []

    def response(conn, sender, path, iface, signal, params, data):
        responses.append(params.unpack())
        loop.quit()

    subscription = bus.signal_subscribe(
        DESKTOP, REQUEST, "Response", handle, None, Gio.DBusSignalFlags.NONE, response, None
    )
    options = {"interactive": GLib.Variant("b", args.interactive), "handle_token": GLib.Variant("s", token)}
    reply = bus.call_sync(
        DESKTOP,
        PATH,
        "org.freedesktop.portal.Screenshot",
        "Screenshot",
        GLib.Variant("(sa{sv})", ("", options)),
        GLib.VariantType.new("(o)"),
        Gio.DBusCallFlags.NONE,
        10000,
        None,
    )
    assert reply.unpack()[0] == handle

    def close():
        try:
            bus.call_sync(DESKTOP, handle, REQUEST, "Close", None, None, Gio.DBusCallFlags.NONE, 2000, None)
            outcome["close_returned"] = True
        except GLib.Error as error:
            errors.append(str(error))
        GLib.timeout_add(500, lambda: loop.quit() or False)
        return False

    if args.close_after_ms is not None:
        GLib.timeout_add(args.close_after_ms, close)
    timeout = GLib.timeout_add_seconds(args.timeout, lambda: errors.append("request timeout") or loop.quit() or False)
    loop.run()
    GLib.source_remove(timeout)
    # Allow a duplicate completion to arrive before asserting exactly once.
    GLib.timeout_add(300, lambda: loop.quit() or False)
    loop.run()
    bus.signal_unsubscribe(subscription)
    outcome["responses"] = responses
    outcome["errors"] = errors
    try:
        assert not errors, errors
        if args.expect == "closed":
            assert not responses and outcome.get("close_returned"), outcome
        else:
            assert len(responses) == 1, outcome
            code, results = responses[0]
            if args.expect == "success":
                assert code == 0 and "uri" in results, outcome
                file = Gio.File.new_for_uri(results["uri"])
                filename = file.get_path()
                assert filename is not None, results
                image = GdkPixbuf.Pixbuf.new_from_file(filename)
                outcome["size"] = [image.get_width(), image.get_height()]
                assert all(dimension > 0 for dimension in outcome["size"])
                shutil.copyfile(filename, args.output / "screenshot.png")
            else:
                assert code != 0 and "uri" not in results, outcome
                if args.expect == "cancel":
                    assert code == 1, outcome
        try:
            bus.call_sync(DESKTOP, handle, REQUEST, "Close", None, None, Gio.DBusCallFlags.NONE, 2000, None)
            raise AssertionError("completed request is still exported")
        except GLib.Error as error:
            assert "UnknownMethod" in str(error) or "UnknownObject" in str(error), error
        outcome["passed"] = True
    finally:
        (args.output / "result.json").write_text(json.dumps(outcome, indent=2) + "\n")
    print(json.dumps(outcome, indent=2))


if __name__ == "__main__":
    main()
