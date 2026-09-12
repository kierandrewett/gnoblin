#!/usr/bin/env python3
"""Compare compositor CPU/PSS under the same private GTK window workload.

Run through run-gnome-shell.sh with GNOBLIN_TEST_MODE=gnoblin or user.
This compares session modes of the same patched build, not stock GNOME.
"""
import json
import os
from pathlib import Path
import time

import gi
gi.require_version('Gtk', '4.0')
from gi.repository import Gio, GLib, Gtk

if not os.environ.get('WAYLAND_DISPLAY', '').startswith('gnoblin-gs-'):
    raise SystemExit('Use GNOBLIN_TEST_DBUS_CLIENT in scripts/run-gnome-shell.sh')

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
pid = bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
                    'org.freedesktop.DBus', 'GetConnectionUnixProcessID',
                    GLib.Variant('(s)', ('org.gnome.Shell',)), None,
                    Gio.DBusCallFlags.NONE, 5000, None).unpack()[0]
context = GLib.MainContext.default()

def spin(seconds):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        while context.pending():
            context.iteration(False)
        time.sleep(.002)

def sample():
    fields = Path(f'/proc/{pid}/stat').read_text().split(') ')[1].split()
    memory = {}
    for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines():
        key, _, value = line.partition(':')
        if key in ('Rss', 'Pss', 'Private_Dirty'):
            memory[key + 'KiB'] = int(value.split()[0])
    return {'ticks': int(fields[11]) + int(fields[12]), **memory}

rows = []
draws = 0
def measure(name):
    spin(1)
    before = sample()
    initial_draws = draws
    start = time.monotonic()
    spin(float(os.environ.get('GNOBLIN_BENCH_SECONDS', '4')))
    elapsed = time.monotonic() - start
    after = sample()
    rows.append({'phase': name, 'cpuPercent': round((after['ticks'] - before['ticks']) /
                 os.sysconf('SC_CLK_TCK') / elapsed * 100, 2),
                 'clientDraws': draws - initial_draws, **{k: v for k, v in after.items() if k != 'ticks'}})
    print(json.dumps(rows[-1]), flush=True)

Gtk.init()
if os.environ['GNOBLIN_ACTIVE_MODE'] == 'user':
    # GNOME starts in Overview. Dismiss it before comparing normal desktops;
    # otherwise its mapped window clones intentionally keep clients drawing.
    def evaluate(code):
        ok, value = bus.call_sync('org.gnome.Shell', '/org/gnome/Shell', 'org.gnome.Shell',
                                  'Eval', GLib.Variant('(s)', (code,)), None,
                                  Gio.DBusCallFlags.NONE, 5000, None).unpack()
        assert ok, 'GNOME comparison requires GNOBLIN_TEST_UNSAFE_MODE=1 on the private test bus: ' + value
        return value
    evaluate('Main.overview.hide(); true')
    spin(.5)
    assert evaluate('Main.overview.visible') == 'false', 'Overview is still visible'
measure('empty')
windows = []
for i in range(8):
    window = Gtk.Window(title=f'Performance fixture {i}')
    window.set_default_size(800, 500)
    window.set_child(Gtk.Label(label=f'Window {i}'))
    window.present()
    windows.append(window)
    spin(.15)
spin(2)
measure('eight-static-windows')
area = Gtk.DrawingArea()
area.set_content_width(800)
area.set_content_height(500)
def draw(widget, cr, width, height):
    global draws
    draws += 1
    cr.set_source_rgb(.1, .2, .3)
    cr.paint()
    cr.set_source_rgb(.8, .8, .8)
    cr.rectangle((draws * 4) % max(1, width - 100), 20, 100, height - 40)
    cr.fill()
area.set_draw_func(draw)
windows[-1].set_child(area)
tick = area.add_tick_callback(lambda widget, clock: (widget.queue_draw(), True)[1])
measure('visible-animation')
cover = Gtk.Window(title='Opaque benchmark cover')
cover.set_child(Gtk.Label(label='Covered animation'))
cover.set_default_size(320, 180)
cover.present()
spin(.5)
measure('partially-covered-animation')
cover.fullscreen()
measure('covered-animation')
if os.environ.get('GNOBLIN_BENCH_SCREENSHOT'):
    # This is an isolated bus. Acquire a permitted screenshot client name
    # without replacing or queueing behind an existing owner.
    owned = bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
                         'org.freedesktop.DBus', 'RequestName',
                         GLib.Variant('(su)', ('org.gnome.SettingsDaemon.MediaKeys', 4)),
                         None, Gio.DBusCallFlags.NONE, 5000, None).unpack()[0]
    assert owned == 1, 'Private screenshot client name already owned'
    bus.call_sync('org.gnome.Shell.Screenshot', '/org/gnome/Shell/Screenshot',
                  'org.gnome.Shell.Screenshot', 'Screenshot',
                  GLib.Variant('(bbs)', (False, False, os.environ['GNOBLIN_BENCH_SCREENSHOT'])),
                  None, Gio.DBusCallFlags.NONE, 10000, None)
cover.destroy()
measure('revealed-animation')
area.remove_tick_callback(tick)
for window in windows:
    window.destroy()
    spin(.15)
measure('after-close')
report = {'mode': os.environ['GNOBLIN_ACTIVE_MODE'], 'rows': rows}
output = os.environ.get('GNOBLIN_BENCH_REPORT')
if output:
    Path(output).write_text(json.dumps(report, indent=2) + '\n')
assert next(row for row in rows if row['phase'] == 'visible-animation')['clientDraws'] > 30, 'Animation did not render'
assert next(row for row in rows if row['phase'] == 'revealed-animation')['clientDraws'] > 30, 'Animation did not resume'
assert next(row for row in rows if row['phase'] == 'partially-covered-animation')['clientDraws'] > 30, 'Partially visible animation stopped'
if os.environ.get('GNOBLIN_BENCH_REQUIRE_OCCLUSION') == '1':
    assert next(row for row in rows if row['phase'] == 'covered-animation')['clientDraws'] < 10, 'Covered animation kept rendering'
