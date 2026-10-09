#!/usr/bin/env python3
"""Write the gnoblin(1) and gnoblinctl(1) manual pages.

gnoblinctl(1) is generated from help_entries in src/tools/gnoblinctl.c, the table that the command line help reads, so
the manual page and `gnoblinctl help` cannot disagree about which commands exist or what they do.
"""

import argparse
import datetime
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "src/tools/gnoblinctl.c"
SITE = "https://gnoblin.org/"


def release_version():
    return subprocess.check_output(
        [sys.executable, str(ROOT / "scripts/gnoblin-version.py"), "get", "version"], text=True
    ).strip()


def man_date():
    epoch = os.environ.get("SOURCE_DATE_EPOCH")
    moment = (
        datetime.datetime.fromtimestamp(int(epoch), datetime.timezone.utc)
        if epoch
        else datetime.datetime.now(datetime.timezone.utc)
    )
    return moment.strftime("%Y-%m-%d")


def escape(text):
    """Escape text for roff: backslashes, and a leading dot or quote that would start a request."""
    text = text.replace("\\", "\\e").replace("-", "\\-")
    return re.sub(r"^([.'])", r"\\&\1", text, flags=re.MULTILINE)


def help_entries():
    source = SOURCE.read_text()
    start = source.index("static const HelpEntry help_entries[]")
    end = source.index("static const char* help_summary", start)
    entries = re.findall(r'\{"([a-z-]+)", (NULL|"[a-z-]+"), "([^"]+)"\}', source[start:end])
    if not entries:
        raise SystemExit("build-man-pages: no help_entries found in src/tools/gnoblinctl.c")
    commands = {}
    for command, action, summary in entries:
        record = commands.setdefault(command, {"summary": None, "actions": []})
        if action == "NULL":
            record["summary"] = summary
        else:
            record["actions"].append((action.strip('"'), summary))
    for command, record in commands.items():
        if record["summary"] is None:
            raise SystemExit(f"build-man-pages: {command} has no command summary in help_entries")
    return commands


def header(name, title, version, date):
    return f'.TH {title} 1 "{date}" "Gnoblin {version}" "Gnoblin Manual"\n'


def gnoblin_page(version, date):
    return (
        header("gnoblin", "GNOBLIN", version, date)
        + r""".SH NAME
gnoblin \- the Gnoblin Wayland compositor and desktop session
.SH SYNOPSIS
.B gnoblin
.RB [ \-\-version ]
.br
.B gnoblin
.RB [ \-\-config
.IR FILE ]
.RB [ \-\-wayland\-display
.IR NAME ]
.RB [ \-\-devkit ]
.SH DESCRIPTION
.B gnoblin
is the login command of the Gnoblin session.
It starts the compositor, the Lua configuration runtime and the programs listed in the configuration for autostart.
The login manager runs it when you choose
.B Gnoblin
at the login screen.
Gnoblin provides the compositor and session services only.
A panel, launcher or other shell comes from a separate program that you start with the session.
.PP
Run
.B gnoblin \-\-version
to print the installed build.
It prints the same text as
.BR "gnoblinctl \-\-version" .
.SH OPTIONS
.TP
.B \-\-version
Print the version, the build time, the Git commit and remote, and the Lua and native API versions, then exit.
Add
.B \-\-json
for the complete record as JSON.
.TP
.BI \-\-config " FILE"
Load FILE as the Lua configuration in place of
.IR ~/.config/gnoblin/init.lua .
.TP
.BI \-\-wayland\-display " NAME"
Use NAME as the Wayland display name of the compositor.
.TP
.B \-\-devkit
Run the compositor inside the nested development viewer, as a window on the current desktop.
This needs a build with the viewer enabled.
.SH FILES
.TP
.I ~/.config/gnoblin/init.lua
The configuration entry point.
Without this file, Gnoblin uses its embedded defaults.
Run
.B gnoblinctl init
to create an editable copy.
.TP
.I $XDG_STATE_HOME/gnoblin/session\-last.log
.TQ
.I $XDG_STATE_HOME/gnoblin/compositor\-last.log
The logs of the latest session.
The files with
.B previous
in place of
.B last
hold the session before it.
.B XDG_STATE_HOME
defaults to
.IR ~/.local/state .
.TP
.I /usr/share/wayland\-sessions/gnoblin.desktop
The login entry that distribution packages install.
.SH SEE ALSO
.BR gnoblinctl (1)
.PP
Documentation: """
        + SITE
        + "\n"
    )


def gnoblinctl_page(version, date, commands):
    out = [header("gnoblinctl", "GNOBLINCTL", version, date)]
    out.append(
        r""".SH NAME
gnoblinctl \- control a running Gnoblin session
.SH SYNOPSIS
.B gnoblinctl
.RI [ OPTIONS ]
.I COMMAND
.RI [ ACTION ]
.RI [ ARGUMENTS ]
.br
.B gnoblinctl \-\-version
.SH DESCRIPTION
.B gnoblinctl
talks to the compositor of the Gnoblin session you are in.
It lists and manages windows, workspaces and monitors, shows and reloads the configuration, and runs Lua against the session API.
Most commands need a running session.
.B \-\-version
works without one.
.PP
With the default format, lists and records print as tables in a terminal and as JSON in a pipe.
A single value, such as the path from
.BR "config path" ,
always prints as plain text.
.SH OPTIONS
.TP
.BR \-j ", " \-\-json
Print JSON, including in a terminal.
.TP
.BI \-\-format " FORMAT"
.B auto
prints lists and records as tables in a terminal and as JSON in a pipe.
A single value stays plain text.
.B json
and
.B table
force one form.
The default is
.BR auto .
.TP
.BI \-\-timeout " SECONDS"
Wait this long for the compositor, from 1 to 60.
The default is 5, and 30 for
.BR "shortcut capture" .
.TP
.BI \-\-socket " PATH"
Use the compositor socket at PATH.
.TP
.BR \-h ", " \-\-help
Print the help.
.TP
.B \-\-version
Print the installed build.
.SH COMMANDS
"""
    )
    for command, record in commands.items():
        out.append(".TP\n.B " + command + "\n" + escape(record["summary"]) + ".\n")
        for action, summary in record["actions"]:
            out.append(
                ".RS\n.TP\n.B " + command + " " + action.replace("-", "\\-") + "\n" + escape(summary) + ".\n.RE\n"
            )
    out.append(
        r""".PP
Run
.B gnoblinctl help
.I COMMAND
.I ACTION
for the arguments of one action.
.SH EXAMPLES
.TP
.B gnoblinctl ping
Check that the compositor answers.
.TP
.B gnoblinctl window list
List the open windows.
.TP
.B gnoblinctl \-\-format json workspace list
Print the workspaces as JSON.
.TP
.B gnoblinctl config reload
Reload the configuration after you edit it.
.SH EXIT STATUS
0 on success.
A non\-zero status means the command failed, and the message on standard error says why.
.SH FILES
.TP
.I $XDG_RUNTIME_DIR/gnoblin/compositor\-v1.sock
The compositor control socket.
.TP
.I $XDG_STATE_HOME/gnoblin/console\-history.ini
The history of
.BR "gnoblinctl lua" .
.SH SEE ALSO
.BR gnoblin (1)
.PP
Documentation: """
        + SITE
        + "\n"
    )
    return "".join(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", help="directory to write gnoblin.1 and gnoblinctl.1 into")
    arguments = parser.parse_args()
    output = Path(arguments.output)
    output.mkdir(parents=True, exist_ok=True)
    version, date = release_version(), man_date()
    (output / "gnoblin.1").write_text(gnoblin_page(version, date))
    (output / "gnoblinctl.1").write_text(gnoblinctl_page(version, date, help_entries()))
    print(f"[man] wrote {output}/gnoblin.1 and {output}/gnoblinctl.1")


if __name__ == "__main__":
    main()
