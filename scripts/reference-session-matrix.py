#!/usr/bin/env python3
"""Compare the recorded reference session with the adapters' checked-in support matrices.

usage: scripts/reference-session-matrix.py [--check]

Reads tests/parity/reference-session/ (recorded by scripts/reference-session-record.sh from stock Mutter 51 headless)
and crates/compat/*/support-matrix.json, and writes tests/parity/adapter-support-matrices.md to standard output. With
`--check` it compares that text with the stored file and fails when they differ, so the document cannot drift from the
recording or from the matrices.

Everything in the document is either read from the recording or from a matrix, or it is a rule stated here and named
as such. Where the recording cannot say (a consumer that could not run in the session, a Wayland global the candidate
may serve), the document says so with [INFERENCE] and gives no verdict.
"""

import json
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RECORD = ROOT / "tests" / "parity" / "reference-session"
STORED = ROOT / "tests" / "parity" / "adapter-support-matrices.md"

# Interfaces every D-Bus object has; they say nothing about the service.
STANDARD_PREFIX = "org.freedesktop.DBus."

# Wayland interface name prefixes of the standard families: the core protocol and wayland-protocols' stable, staging and
# unstable names. The test is by name only. A global outside these is private to a project, and is listed by name below.
PUBLIC_PREFIXES = ("wl_", "wp_", "xdg_", "zxdg_", "zwp_", "ext_", "zwlr_")


# What to do with an interface no adapter declares yet, one row per interface. These are the plan's words, not findings;
# the recording supplies the members and signatures they apply to.
PROPOSAL = {
    "org.gnome.Mutter.InputMapping": "adapter, later (redirect: tablet output mapping config)",
    "org.gnome.Mutter.ServiceChannel": "adapter, later (redirect: privileged client connection from the services role)",
    "org.gnome.Mutter.ColorManager": "dropped: Night Light moves to Gnoblin",
    "org.freedesktop.a11y.PointerLocator": "none in the plan's redirect map; it shares the a11y.Manager object with KeyboardMonitor",
}


# What the plan and the decisions say about each name Mutter uses. The first part of each entry is the plan's; the
# observation that does not fit it is stated beside it.
USED = {
    "org.gnome.SessionManager": (
        "redirect-map row: session inhibit and logout, inhibit on, logout off",
        "Mutter calls `Setenv` on it (observed) and watches its properties; `Setenv` is not in the plan's row",
    ),
    "org.gnome.SettingsDaemon.Color": (
        "Night Light moves to Gnoblin",
        "Mutter only watches the name for its properties and signals (observed)",
    ),
    "org.gnome.SettingsDaemon.Power.Screen": (
        "no row in the plan [INFERENCE: backlight and power-saver state]",
        "watched, never called (observed)",
    ),
    "org.gnome.Orca.KeyboardMonitor": (
        "the plan's a11y row names Orca as a consumer of `org.freedesktop.a11y.KeyboardMonitor` [INFERENCE]",
        "Mutter watches Orca's own name, so Orca serves something Mutter reads; the plan does not say what",
    ),
}


def parse_documents(path):
    """The introspection documents of a recording: `[(connection, object path, [(interface, members)])]`.

    `members` is a list of `(kind, name, signature)`. The documents are separated by `<!-- :conn /path -->` headers."""
    text = path.read_text(encoding="utf-8")
    parts = re.split(r"<!-- (:\S+) (/\S*) -->\n", text)
    out = []
    for index in range(1, len(parts), 3):
        body = re.sub(r"<!DOCTYPE[^>]*>", "", parts[index + 2])
        body = re.sub(r"<!--.*?-->", "", body, flags=re.S)
        root = ET.fromstring(body)
        interfaces = []
        for interface in root.findall("interface"):
            name = interface.get("name")
            if name.startswith(STANDARD_PREFIX):
                continue
            members = []
            for member in interface:
                if member.tag in ("method", "signal"):
                    signature = ",".join(
                        f"{arg.get('direction', 'in' if member.tag == 'method' else 'out')}:{arg.get('type')}"
                        for arg in member.findall("arg")
                    )
                    members.append((member.tag, member.get("name"), signature))
                elif member.tag == "property":
                    members.append(("property", member.get("name"), f"{member.get('type')} {member.get('access')}"))
            interfaces.append((name, members))
        out.append((parts[index], parts[index + 1], interfaces))
    return out


def reference_interfaces():
    """Every non-standard interface the recording saw, by name: `{name: {"paths": set, "members": {(kind, name)}}}`.

    Session and stream objects come from sessions.xml, which was taken while they were alive, so their interfaces are in
    the same table as the ones at fixed paths."""
    found = {}
    for file in ("mutter.xml", "sessions.xml"):
        for _, path, interfaces in parse_documents(RECORD / "dbus" / file):
            for name, members in interfaces:
                entry = found.setdefault(name, {"paths": set(), "members": {}})
                # A session path carries a counter (`Session/u1`); the table names the kind of object.
                entry["paths"].add(re.sub(r"/u\d+$", "/<id>", path))
                for kind, member, signature in members:
                    entry["members"][(kind, member)] = signature
    return found


def adapter_matrices():
    """`{interface: [(crate, {(kind, name): support})]}` from every adapter's support-matrix.json."""
    out = {}
    for file in sorted((ROOT / "crates" / "compat").glob("*/support-matrix.json")):
        crate = file.parent.name
        data = json.loads(file.read_text(encoding="utf-8"))
        for interface in data["interfaces"]:
            members = {(m["kind"], m["name"]): m["support"] for m in interface["members"]}
            out.setdefault(interface["name"], []).append((crate, members))
    return out


def owned_names():
    """The well-known names the Mutter connection owned, from dbus-names.txt, with the connection."""
    run = (RECORD / "run.txt").read_text(encoding="utf-8")
    connection = re.search(r"mutter's connection: (\S+)", run).group(1)
    names = []
    for line in (RECORD / "dbus-names.txt").read_text(encoding="utf-8").splitlines():
        columns = line.split()
        if len(columns) >= 5 and not columns[0].startswith(":") and columns[4] == connection:
            names.append(columns[0])
    return connection, sorted(names)


def mutter_calls():
    """What Mutter's own connection did on the bus, from dbus-monitor-mutter.txt.

    Returns `(requested names, called names with members, watched names, dconf paths, bus calls)`."""
    text = (RECORD / "dbus-monitor-mutter.txt").read_text(encoding="utf-8")
    requested, called, watched, dconf = [], {}, [], []
    for block in text.split("\n\n"):
        header = re.search(
            r"Sender=\S+\s+Destination=(\S+)(?:\s+Path=(\S+))?(?:\s+Interface=(\S+))?(?:\s+Member=(\S+))?", block
        )
        if not header or "Type=method_call" not in block:
            continue
        destination, _, interface, member = header.groups()
        strings = re.findall(r'STRING "(.*)";', block)
        if destination == "org.freedesktop.DBus":
            if member == "RequestName" and strings:
                requested.append(strings[0])
            elif member == "GetNameOwner" and strings and not strings[0].startswith(":"):
                if strings[0] not in watched:
                    watched.append(strings[0])
            elif member == "AddMatch" and strings:
                match = strings[0]
                if "ca.desrt.dconf.Writer" in match:
                    path = re.search(r"arg0path='([^']*)'", match)
                    if path:
                        dconf.append(path.group(1))
        else:
            called.setdefault(destination, set()).add(f"{interface}.{member}")
    return requested, called, watched, sorted(set(dconf))


def wayland_globals():
    """`[(interface, version)]` as wayland-info listed them, and the set of interface names a GTK 4 client bound."""
    globals_ = []
    for line in (RECORD / "wayland-globals.txt").read_text(encoding="utf-8").splitlines():
        match = re.match(r"interface: '(\w+)',\s+version:\s+(\d+)", line)
        if match:
            globals_.append((match.group(1), int(match.group(2))))
    bound = {}
    for line in (RECORD / "wayland-gtk4.txt").read_text(encoding="utf-8").splitlines():
        match = re.search(r'bind\(\d+, "(\w+)", (\d+),', line)
        if match:
            bound[match.group(1)] = int(match.group(2))
    return globals_, bound


def camel(name):
    return "".join(part.capitalize() for part in name.split("_"))


def named_in_candidate(interface, sources):
    """Whether the interface or its generated type name appears in the candidate's Wayland sources. A name in a source
    file is not proof the global is advertised; it is the cheapest evidence that the protocol was looked at."""
    base = re.sub(r"_v\d+$", "", interface)
    needles = {interface, camel(interface), base}
    return any(needle in sources for needle in needles)


def candidate_sources():
    texts = []
    for pattern in ("crates/gnoblin-wayland/src/**/*.rs", "crates/compat/gnoblin-compat-wayland-*/src/**/*.rs"):
        for file in ROOT.glob(pattern):
            texts.append(file.read_text(encoding="utf-8", errors="replace"))
    return "\n".join(texts)


def gtk_private_requests():
    """The `(interface, request)` pairs the GTK client sent to gtk_shell1 or gtk_surface1, once each, in trace order."""
    seen = []
    for line in (RECORD / "wayland-gtk4-trace.txt").read_text(encoding="utf-8").splitlines():
        match = re.search(r" -> (gtk_shell1|gtk_surface1)#\d+\.(\w+)\(", line)
        if match and (match.group(1), match.group(2)) not in seen:
            seen.append((match.group(1), match.group(2)))
    return seen


def count(members, support):
    return sum(1 for value in members.values() if value == support)


def render():
    run = (RECORD / "run.txt").read_text(encoding="utf-8").splitlines()
    facts = {line.split(":", 1)[0]: line.split(":", 1)[1].strip() for line in run if re.match(r"^[a-z-]+: ", line)}
    connection, names = owned_names()
    reference = reference_interfaces()
    matrices = adapter_matrices()
    requested, called, watched, dconf = mutter_calls()
    globals_, bound = wayland_globals()
    sources = candidate_sources()

    out = []
    w = out.append
    w(
        "<!-- Generated by `scripts/reference-session-matrix.py` from tests/parity/reference-session/ and the adapters' -->"
    )
    w(
        "<!-- support-matrix.json files. Do not edit by hand; `scripts/reference-session-matrix.py --check` fails on drift. -->"
    )
    w("")
    w("# Adapter support matrices against a reference session")
    w("")
    w(
        "What stock Mutter 51 serves and uses in a headless session, taken from a recording, set beside what each "
        "compatibility adapter's `support-matrix.json` declares. The recording is in "
        "`tests/parity/reference-session/` and `scripts/reference-session-record.sh` makes it. The plan's rule is that an "
        "adapter declares every upstream member as supported, partial or unsupported (the rules in the documentation of "
        "`gnoblin-compat-core`); this page checks the declarations against a live Mutter."
    )
    w("")
    w("## The session")
    w("")
    w(f"- Mutter: {facts.get('mutter', 'unknown')}")
    w(f"- Image: {facts.get('os', 'unknown')}, {facts.get('host', 'unknown')} (an Agent37 instance under gVisor)")
    w(f"- Bus: {facts.get('dbus', 'unknown')}; `busctl`: {facts.get('busctl', 'unknown')}")
    w(f"- Wayland: {facts.get('libwayland', 'unknown')}; client {facts.get('wayland-info', 'unknown')}")
    w(f"- Recorded: {facts.get('recorded', 'unknown')}")
    w(
        "- Backend: `mutter --headless --wayland --virtual-monitor 1280x720`, llvmpipe, a private session bus, built with"
    )
    w("  `-Dxwayland=false` (`scripts/parity-reference.sh mutter`)")
    w("")
    w("What this session could not show, so the tables below make no claim about it:")
    w("")
    w(
        "- **The stock GNOME portal's calls into Mutter.** `xdg-desktop-portal` refused every call with `Access denied` "
        "because its sandbox check opens `/proc/PID/root`, and an Agent37 instance has no `/proc` inside a chroot "
        "(`dbus-calls.txt`). The known consumers in the redirect map stay [INFERENCE]."
    )
    w(
        "- **GNOME Shell, gnome-settings-daemon, gnome-session, Orca.** Not installed. Mutter looks for four of them; see below."
    )
    w("- **Real hardware.** There is no GPU, no libinput device and no logind.")
    w("")

    w("## D-Bus names Mutter 51 owns")
    w("")
    w(
        f"Every name below is owned by one connection (`{connection}`). Object paths are the ones that answered `Introspect`;"
    )
    w("`<id>` is a session or stream counter, introspected while the object was alive.")
    w("")
    w("| Name | Interfaces served | Object path | Adapter crate |")
    w("| --- | --- | --- | --- |")
    for name in names:
        prefix = "/" + name.replace(".", "/")
        rows = []
        for interface, entry in sorted(reference.items()):
            for path in sorted(entry["paths"]):
                if path == prefix or path.startswith(prefix + "/"):
                    rows.append((interface, path))
        interfaces = "<br>".join(f"`{interface}`" for interface, _ in rows) or "none found"
        paths = "<br>".join(f"`{path}`" for _, path in rows) or "-"
        adapters = sorted({crate for interface, _ in rows for crate, _ in matrices.get(interface, [])})
        w(f"| `{name}` | {interfaces} | {paths} | {', '.join(f'`{c}`' for c in adapters) or 'none'} |")
    w("")

    w("## Interfaces against the adapters' matrices")
    w("")
    w(
        "`in matrix` counts the members a matrix declares for an interface; `missing` are members Mutter 51 has that the matrix"
    )
    w(
        "does not declare (an undeclared member is refused, so a caller of it breaks); `extra` are declared members Mutter 51"
    )
    w("does not have.")
    w("")
    w(
        "| Interface | Members on Mutter 51 | Adapter | Supported | Partial | Unsupported | Missing from matrix | Extra in matrix |"
    )
    w("| --- | --- | --- | --- | --- | --- | --- | --- |")
    gaps = []
    for interface, entry in sorted(reference.items()):
        members = entry["members"]
        declared = matrices.get(interface)
        if not declared:
            w(f"| `{interface}` | {len(members)} | none | - | - | - | all | - |")
            gaps.append((interface, "no adapter declares this interface", None))
            continue
        for crate, matrix in declared:
            missing = sorted(set(members) - set(matrix))
            extra = sorted(set(matrix) - set(members))
            supported = count(matrix, "supported")
            partial = sum(1 for v in matrix.values() if v not in ("supported", "unsupported"))
            unsupported = count(matrix, "unsupported")

            def fmt(items):
                return ", ".join(f"`{n}`" for _, n in items) or "none"

            w(
                f"| `{interface}` | {len(members)} | `{crate}` | {supported} | {partial} | {unsupported} | "
                f"{fmt(missing)} | {fmt(extra)} |"
            )
            if missing or extra:
                gaps.append((interface, crate, (missing, extra)))
    w("")

    w("### Members of the interfaces no adapter declares")
    w("")
    w(
        "Taken from the recording, with the plan's proposal for each interface. An adapter that is written from this list declares"
    )
    w("every row; a member it does not serve is declared `unsupported` with a reason, never left out.")
    w("")
    for interface, entry in sorted(reference.items()):
        if interface in matrices:
            continue
        w(f"**`{interface}`**: {PROPOSAL.get(interface, 'no proposal in the plan')}.")
        w("")
        w("| Kind | Member | Signature |")
        w("| --- | --- | --- |")
        for (kind, member), signature in sorted(entry["members"].items()):
            w(f"| {kind} | `{member}` | `{signature}` |")
        w("")

    w("### Adapters whose interface this session did not show")
    w("")
    w(
        "D-Bus interfaces a matrix declares that Mutter 51 headless does not serve. Each is served by another program on a stock"
    )
    w("session (GNOME Shell, gnome-session, a portal), so a session with only Mutter cannot check it [INFERENCE].")
    w("")
    w("| Interface | Adapter | Members declared |")
    w("| --- | --- | --- |")
    wayland_names = {name for name, _ in wayland_globals()[0]} | {"gtk_surface1"}
    other_surfaces = []
    for interface, entries in sorted(matrices.items()):
        if interface in reference or interface in wayland_names or "mutter_x11_interop" in interface:
            continue
        for crate, matrix in entries:
            if re.match(r"^[A-Za-z]+(\.[A-Za-z0-9]+)+$", interface):
                w(f"| `{interface}` | `{crate}` | {len(matrix)} |")
            else:
                other_surfaces.append((interface, crate, len(matrix)))
    w("")
    w(
        "Matrices for surfaces that are neither a bus interface nor a Wayland global, which this recording has nothing to say about:"
    )
    w("")
    for interface, crate, size in other_surfaces:
        w(f"- `{interface}` in `{crate}` ({size} member(s)).")
    w("")

    w("## D-Bus names Mutter uses")
    w("")
    w("From the method calls its connection sent (`dbus-monitor-mutter.txt`):")
    w("")
    w(f"- **Names it requested:** {', '.join(f'`{n}`' for n in requested)}.")
    w("")
    w("| Name Mutter looks up with `GetNameOwner` and watches | The plan says | The recording shows |")
    w("| --- | --- | --- |")
    for name in watched:
        plan, observed = USED.get(name, ("not in the plan", "watched (observed)"))
        w(f"| `{name}` | {plan} | {observed} |")
    w("")
    for destination, members in sorted(called.items()):
        w(f"- **Methods it called on `{destination}`:** {', '.join(f'`{m}`' for m in sorted(members))}.")
    w(f"- **dconf paths whose changes it listens for** (`ca.desrt.dconf.Writer`, {len(dconf)} paths):")
    for path in dconf:
        w(f"  - `{path}`")
    w(
        "- **Logged and not on the bus:** `mutter.log` records a failed `org.freedesktop.locale1` proxy and a failed connection "
        "to colord. Both are system-bus services, which this session does not have."
    )
    w("")

    w("## Wayland globals")
    w("")
    w(
        "From `wayland-info` against the session (`wayland-globals.txt`, with the client's `WAYLAND_DEBUG=1` trace in "
        "`wayland-trace.txt`). `bound by GTK 4` is the version `gtk4-widget-factory` bound in its own trace "
        "(`wayland-gtk4.txt`), so it is a real consumer and not an inference. `named in the candidate` says whether the "
        "interface or its generated type appears in `crates/gnoblin-wayland` or a `compat/wayland` crate: a name in a source "
        "file is not proof that a global is advertised, and a live comparison is the parity runner's job."
    )
    w("")
    w("| Global | Version offered | Kind | Bound by GTK 4 | Named in the candidate | Adapter matrix |")
    w("| --- | --- | --- | --- | --- | --- |")
    private = []
    missing_names = []
    for interface, version in globals_:
        kind = "standard family" if interface.startswith(PUBLIC_PREFIXES) else "private"
        if kind == "private":
            private.append(interface)
        bound_to = f"v{bound[interface]}" if interface in bound else "-"
        named = named_in_candidate(interface, sources)
        if not named:
            missing_names.append(interface)
        adapter = ", ".join(f"`{crate}`" for crate, _ in matrices.get(interface, [])) or "-"
        w(f"| `{interface}` | {version} | {kind} | {bound_to} | {'yes' if named else 'no'} | {adapter} |")
    w("")
    w(
        f"{len(globals_)} globals. Private to GNOME or GTK: {', '.join(f'`{n}`' for n in private) or 'none'}. "
        "`mutter_x11_interop` is absent because this Mutter has no Xwayland, so the `gnoblin-compat-wayland-mutter-x11-interop` "
        "matrix cannot be checked against it here [INFERENCE: a build with Xwayland would advertise it]."
    )
    w("")
    sent = gtk_private_requests()
    w(
        "What a GTK 4 client (`gtk4-node-editor`) sent to the private protocols in its own trace (`wayland-gtk4-trace.txt`), and what the"
    )
    w("adapter matrix declares for each request:")
    w("")
    w("| Request sent | Matrix status |")
    w("| --- | --- |")
    for interface, request in sent:
        status = "no matrix"
        for _, matrix in matrices.get(interface, []):
            support = matrix.get(("method", request)) or matrix.get(("request", request))
            status = support if support else "not declared"
        w(f"| `{interface}.{request}` | {status} |")
    if not sent:
        w("| none observed | - |")
    w("")
    w(
        f"Globals Mutter offers that the candidate's sources never name ({len(missing_names)}): "
        + (", ".join(f"`{n}`" for n in missing_names) or "none")
        + "."
    )
    w("")
    w("## Adapter crates with no matrix")
    w("")
    w(
        "A crate under `crates/compat/` that has no `support-matrix.json` has declared nothing yet. The ones below are one-line crate"
    )
    w("roots, so the declaration is owed with the adapter:")
    w("")
    for directory in sorted((ROOT / "crates" / "compat").glob("gnoblin-compat-*")):
        # The shared frame and the host are not adapters; they serve no upstream interface.
        if (directory / "support-matrix.json").exists() or directory.name in (
            "gnoblin-compat-core",
            "gnoblin-compat-host",
        ):
            continue
        lib = directory / "src" / "lib.rs"
        lines = lib.read_text(encoding="utf-8").splitlines() if lib.exists() else []
        first = lines[0].removeprefix("//!").strip() if lines else "no lib.rs"
        w(f"- `{directory.name}`: `{first}` ({len(lines)} line(s))")
    w("")
    for destination, members in sorted(called.items()):
        if destination not in matrices:
            gaps.append(
                (
                    destination,
                    f"Mutter calls {', '.join(f'`{m}`' for m in sorted(members))} on it (observed) and no adapter matrix declares it",
                    None,
                )
            )
    w("## Gaps the comparison found")
    w("")
    if not gaps:
        w("None.")
    for interface, crate, detail in gaps:
        if detail is None:
            w(f"- `{interface}`: {crate}.")
            continue
        missing, extra = detail
        parts = []
        if missing:
            parts.append(
                "Mutter 51 has " + ", ".join(f"`{n}` ({k})" for k, n in missing) + " which the matrix does not declare"
            )
        if extra:
            parts.append(
                "the matrix declares " + ", ".join(f"`{n}` ({k})" for k, n in extra) + " which Mutter 51 does not have"
            )
        w(f"- `{interface}` in `{crate}`: " + "; ".join(parts) + ".")
    w("")
    return "\n".join(out)


def main(argv):
    text = render()
    if "--check" in argv:
        if STORED.read_text(encoding="utf-8") == text:
            print(
                "reference-session-matrix: tests/parity/adapter-support-matrices.md matches the recording and the matrices"
            )
            return 0
        print(
            "reference-session-matrix: tests/parity/adapter-support-matrices.md is stale; run "
            "`python3 scripts/reference-session-matrix.py > tests/parity/adapter-support-matrices.md`",
            file=sys.stderr,
        )
        return 1
    sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
