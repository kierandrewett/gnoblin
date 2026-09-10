#!/usr/bin/env python3
"""Exercise the actual control service and CLI in a private Gnoblin session."""
import json
import os
import re
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
CTL = ROOT / 'src/tools/gnoblinctl'

def ctl(*args, ok=True):
    result = subprocess.run([str(CTL), '--json', *args], capture_output=True, text=True, timeout=15)
    assert (result.returncode == 0) == ok, result.stdout + result.stderr
    return json.loads(result.stdout) if ok else result.stderr

state = ctl('permissions', 'list')
config = Path(state['path'])
config.write_text('# preserve this comment\n[shell]\nosd = false\n')
ctl('reload-config')
ctl('permissions', 'default', 'ask')
ctl('permissions', 'set', 'rustdesk', 'allow', '--match', '^host-exe:/usr/bin/rustdesk$',
    '--capability', 'screen-cast', '--capability', 'remote-desktop', '--monitor', 'primary',
    '--device', 'keyboard', '--device', 'pointer')
assert config.read_text().startswith('# preserve this comment\n[shell]\nosd = false')
identity = 'host-exe:/usr/bin/rustdesk'
assert ctl('permissions', 'check', 'remote-desktop', identity)['devices'] == 3
assert ctl('permissions', 'check', 'screen-cast', identity)['level'] == 'allow'
assert ctl('permissions', 'check', 'screen-cast', 'host-exe:/tmp/rustdesk')['level'] == 'ask'
ctl('permissions', 'set', 'blocked', 'deny', '--match', 'rustdesk', '--capability', 'screen-cast')
assert ctl('permissions', 'check', 'screen-cast', identity)['rule'] == 'blocked'
ctl('permissions', 'remove', 'blocked')
assert ctl('permissions', 'check', 'screen-cast', identity)['level'] == 'allow'
saved = config.read_text()
ctl('permissions', 'set', 'invalid', 'allow', '--match', '[', '--capability', 'screen-cast', ok=False)
assert config.read_text() == saved
config.write_text(saved + '\n[permissions.invalid]\noops = true\n')
ctl('reload-config', ok=False)
assert ctl('permissions', 'check', 'screen-cast', identity)['level'] == 'allow'
config.write_text(saved)
ctl('reload-config')
ctl('permissions', 'remove', 'rustdesk')
assert ctl('permissions', 'check', 'screen-cast', identity)['level'] == 'ask'
print('PASS: live permission policy, CLI writes, preserved config, reload and invalid-edit retention')

# Act as the trusted portal frontend on this private bus. No host portal state is used.
import gi
import time
from gi.repository import Gio, GLib
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
# The private shell can activate a frontend before this fixture starts.
try:
    pid = bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
        'GetConnectionUnixProcessID', GLib.Variant('(s)', ('org.freedesktop.portal.Desktop',)),
        None, Gio.DBusCallFlags.NONE, 2000, None).unpack()[0]
except GLib.Error:
    pid = None
if pid is not None:
    import signal
    environment = Path(f'/proc/{pid}/environ').read_bytes().split(b'\0')
    assert ('DBUS_SESSION_BUS_ADDRESS=' + os.environ['DBUS_SESSION_BUS_ADDRESS']).encode() in environment
    os.kill(pid, signal.SIGTERM)
    for _ in range(50):
        if not Path(f'/proc/{pid}').exists(): break
        time.sleep(0.05)
assert bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
              'RequestName', GLib.Variant('(su)', ('org.freedesktop.portal.Desktop', 4)),
              None, Gio.DBusCallFlags.NONE, 2000, None).unpack()[0] == 1
backend_name = 'org.freedesktop.impl.portal.desktop.gnome'
backend_log = open(Path(os.environ['XDG_CACHE_HOME']) / 'permission-backend.log', 'w+')
backend = subprocess.Popen([str(ROOT / 'build/xdg-desktop-portal-gnome/src/xdg-desktop-portal-gnome'), '--replace'],
                           stdout=backend_log, stderr=backend_log, env={**os.environ, 'G_MESSAGES_DEBUG': 'all'})
app = os.environ.get('GNOBLIN_PERMISSION_TEST_APP', 'com.example.PermissionProbe')
app_identity = '^app-id:' + re.escape(app) + '$'
app_match = re.escape(app)
owner = bus.get_unique_name().removeprefix(':').replace('.', '_')
serial = 0

def handles():
    global serial
    serial += 1
    return (f'/org/freedesktop/portal/desktop/request/{owner}/p{serial}',
            f'/org/freedesktop/portal/desktop/session/{owner}/p{serial}')

def portal(interface, method, signature, values):
    values = (handles()[0], *values[1:])
    return bus.call_sync(backend_name, '/org/freedesktop/portal/desktop',
        'org.freedesktop.impl.portal.' + interface, method, GLib.Variant(signature, values),
        None, Gio.DBusCallFlags.NONE, 6000, None)

def close(session):
    bus.call_sync(backend_name, session, 'org.freedesktop.impl.portal.Session', 'Close',
                  None, None, Gio.DBusCallFlags.NONE, 2000, None)

def capture(options=None):
    request, session = handles()
    assert portal('ScreenCast', 'CreateSession', '(oosa{sv})', (request, session, app, {})).unpack()[0] == 0
    options = options or {}
    options.update(types=GLib.Variant('u', 1), persist_mode=GLib.Variant('u', 2))
    assert portal('ScreenCast', 'SelectSources', '(oosa{sv})', (request, session, app, options)).unpack()[0] == 0
    result = portal('ScreenCast', 'Start', '(oossa{sv})', (request, session, app, '', {}))
    close(session)
    return result

try:
    for attempt in range(50):
        try:
            pid = bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
                'GetConnectionUnixProcessID', GLib.Variant('(s)', (backend_name,)), None,
                Gio.DBusCallFlags.NONE, 1000, None).unpack()[0]
            if pid != backend.pid:
                time.sleep(0.1)
                continue
            bus.call_sync(backend_name, '/org/freedesktop/portal/desktop',
                'org.freedesktop.DBus.Properties', 'Get',
                GLib.Variant('(ss)', ('org.freedesktop.impl.portal.ScreenCast', 'AvailableSourceTypes')),
                None, Gio.DBusCallFlags.NO_AUTO_START, 1000, None)
            break
        except GLib.Error:
            if backend.poll() is not None: raise RuntimeError('Portal backend exited')
            time.sleep(0.1)
    else: raise RuntimeError('Portal backend did not initialise ScreenCast')
    ctl('permissions', 'default', 'deny')
    for capability in ('screen-cast', 'remote-desktop', 'input-capture', 'screenshot', 'access'):
        ctl('permissions', 'set', capability, 'allow', '--match', app_identity,
            '--capability', capability, *(['--monitor', 'primary'] if capability == 'screen-cast' else []),
            *(['--device', 'keyboard', '--device', 'pointer'] if capability == 'remote-desktop' else []))
    result = capture()
    assert result.unpack()[0] == 0 and result.unpack()[1]['streams'], result
    restore = result.get_child_value(1).lookup_value('restore_data', None)
    assert restore is not None, result
    ctl('permissions', 'set', 'block-capture', 'deny', '--match', app_match, '--capability', 'screen-cast')
    assert capture({'restore_data': restore}).unpack()[0] == 2
    ctl('permissions', 'remove', 'block-capture')
    request, session = handles()
    assert portal('RemoteDesktop', 'CreateSession', '(oosa{sv})', (request, session, app, {})).unpack()[0] == 0
    assert portal('RemoteDesktop', 'SelectDevices', '(oosa{sv})', (request, session, app,
                  {'types': GLib.Variant('u', 3)})).unpack()[0] == 0
    result = portal('RemoteDesktop', 'Start', '(oossa{sv})', (request, session, app, '', {})).unpack()
    assert result[0] == 0 and result[1]['devices'] == 3, result
    close(session)
    request, session = handles()
    result = portal('InputCapture', 'CreateSession', '(oossa{sv})', (request, session, app, '',
                    {'capabilities': GLib.Variant('u', 3)})).unpack()
    assert result[0] == 0, result
    close(session)
    request, _ = handles()
    result = portal('Screenshot', 'Screenshot', '(ossa{sv})', (request, app, '', {})).unpack()
    assert result[0] == 0 and result[1]['uri'].startswith('file:'), result
    request, _ = handles()
    result = portal('Access', 'AccessDialog', '(osssssa{sv})', (request, app, '', 'Test', 'Test', 'Test', {})).unpack()
    assert result[0] == 0, result
    # An allow rule is limited to its declared input devices.
    request, session = handles()
    assert portal('RemoteDesktop', 'CreateSession', '(oosa{sv})', (request, session, app, {})).unpack()[0] == 0
    assert portal('RemoteDesktop', 'SelectDevices', '(oosa{sv})', (request, session, app,
                  {'types': GLib.Variant('u', 7)})).unpack()[0] == 0
    assert portal('RemoteDesktop', 'Start', '(oossa{sv})', (request, session, app, '', {})).unpack()[0] == 2
    close(session)
    # Every adapter must honour the blocklist, not just capture.
    for capability, interface, method, signature in [
        ('screenshot', 'Screenshot', 'Screenshot', '(ossa{sv})'),
        ('access', 'Access', 'AccessDialog', '(osssssa{sv})'),
        ('input-capture', 'InputCapture', 'CreateSession', '(oossa{sv})')]:
        ctl('permissions', 'set', 'block', 'deny', '--match', app_match, '--capability', capability)
        request, session = handles()
        values = (request, app, '', {})
        if capability == 'access': values = (request, app, '', 'Test', 'Test', 'Test', {})
        if capability == 'input-capture': values = (request, session, app, '', {'capabilities': GLib.Variant('u', 3)})
        assert portal(interface, method, signature, values).unpack()[0] != 0
        ctl('permissions', 'remove', 'block')
    # A forced prompt must ignore otherwise valid restore data and remain cancellable.
    ctl('permissions', 'set', 'prompt', 'ask', '--match', app_match, '--capability', 'screen-cast')
    request, session = handles()
    assert portal('ScreenCast', 'CreateSession', '(oosa{sv})', (request, session, app, {})).unpack()[0] == 0
    assert portal('ScreenCast', 'SelectSources', '(oosa{sv})', (request, session, app,
        {'types': GLib.Variant('u', 1), 'restore_data': restore})).unpack()[0] == 0
    responses = []
    loop = GLib.MainLoop()
    def started(connection, result):
        responses.append(connection.call_finish(result).unpack()[0])
        loop.quit()
    bus.call(backend_name, '/org/freedesktop/portal/desktop', 'org.freedesktop.impl.portal.ScreenCast',
        'Start', GLib.Variant('(oossa{sv})', (request, session, app, '', {})), None,
        Gio.DBusCallFlags.NONE, 6000, None, started)
    def cancel():
        assert not responses, 'ask restored without a prompt'
        bus.call_sync(backend_name, request, 'org.freedesktop.impl.portal.Request', 'Close',
                      None, None, Gio.DBusCallFlags.NONE, 2000, None)
        return GLib.SOURCE_REMOVE
    GLib.timeout_add(400, cancel)
    loop.run()
    assert responses == [2], responses
    close(session)
    # A direct backend caller cannot borrow an allowed application ID.
    direct = Gio.DBusConnection.new_for_address_sync(os.environ['DBUS_SESSION_BUS_ADDRESS'],
        Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
        None, None)
    request, _ = handles()
    result = direct.call_sync(backend_name, '/org/freedesktop/portal/desktop',
        'org.freedesktop.impl.portal.Access', 'AccessDialog',
        GLib.Variant('(osssssa{sv})', (request, app, '', 'Test', 'Test', 'Test', {})),
        None, Gio.DBusCallFlags.NONE, 2000, None).unpack()
    assert result[0] != 0
    direct.close_sync(None)
    # Native applications with an empty app ID use the actual caller executable.
    executable = os.readlink('/proc/self/exe')
    ctl('permissions', 'set', 'native', 'allow', '--match', '^host-exe:' + re.escape(executable) + '$',
        '--capability', 'access')
    request, _ = handles()
    assert portal('Access', 'AccessDialog', '(osssssa{sv})', (request, '', '', 'Test', 'Test', 'Test', {})).unpack()[0] == 0
    # A stale control client must not replace newer policy.
    old = ctl('permissions', 'list')['policy']
    ctl('permissions', 'default', 'ask')
    try:
        bus.call_sync('org.gnoblin.Shell', '/org/gnoblin/Shell', 'org.gnoblin.Shell', 'SetPermissions',
            GLib.Variant('(ss)', (json.dumps(old), json.dumps(old))), None, Gio.DBusCallFlags.NONE, 2000, None)
        raise AssertionError('stale write accepted')
    except GLib.Error as error:
        assert 'changed' in str(error), error
    assert ctl('permissions', 'list')['policy']['default'] == 'ask'
    print('PASS: real portal unattended ScreenCast, RemoteDesktop, InputCapture, Screenshot, Access; deny overrides restore data')
finally:
    backend.terminate()
    backend.wait(timeout=5)
    backend_log.seek(0)
    log = backend_log.read()
    print('\n'.join(line for line in log.splitlines() if 'gnoblin:' in line or 'CRITICAL' in line))
    assert 'CRITICAL' not in log, log[-3000:]
    backend_log.close()
