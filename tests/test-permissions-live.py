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

config = Path(os.environ['XDG_CONFIG_HOME']) / 'gnoblin' / 'init.lua'
config.parent.mkdir(parents=True, exist_ok=True)

def lua(value):
    if value is None:
        return 'nil'
    if isinstance(value, bool):
        return str(value).lower()
    if isinstance(value, (int, float)):
        return str(value)
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, list):
        return '{' + ', '.join(lua(entry) for entry in value) + '}'
    if isinstance(value, dict):
        entries = []
        for key, entry in value.items():
            name = key if key.isidentifier() else '[' + lua(key) + ']'
            entries.append(f'{name} = {lua(entry)}')
        return '{' + ', '.join(entries) + '}'
    raise TypeError(value)

def apply_policy(default, rules, ok=True):
    settings = {'shell': {'osd': False}, 'permissions': {'default': default, 'rules': rules}}
    config.write_text('local g = require("gnoblin")\ng.set(' + lua(settings) + ')\n')
    return ctl('config', 'reload', ok=ok)

rustdesk = {'name': 'rustdesk', 'level': 'allow', 'match': '^host-exe:/usr/bin/rustdesk$',
            'capabilities': ['screen-cast', 'remote-desktop'], 'monitors': ['primary'],
            'devices': ['keyboard', 'pointer']}
apply_policy('ask', [rustdesk])
identity = 'host-exe:/usr/bin/rustdesk'
assert ctl('permissions', 'check', 'remote-desktop', identity)['devices'] == 3
assert ctl('permissions', 'check', 'screen-cast', identity)['level'] == 'allow'
assert ctl('permissions', 'check', 'screen-cast', 'host-exe:/tmp/rustdesk')['level'] == 'ask'
blocked = {'name': 'blocked', 'level': 'deny', 'match': 'rustdesk', 'capabilities': ['screen-cast']}
apply_policy('ask', [rustdesk, blocked])
assert ctl('permissions', 'check', 'screen-cast', identity)['rule'] == 'blocked'
apply_policy('ask', [rustdesk])
assert ctl('permissions', 'check', 'screen-cast', identity)['level'] == 'allow'
saved = config.read_text()
config.write_text('local g = require("gnoblin")\ng.set({permissions = {rules = {{match = "["}}}})\n')
ctl('config', 'reload', ok=False)
assert ctl('permissions', 'check', 'screen-cast', identity)['level'] == 'allow'
config.write_text(saved)
ctl('config', 'reload')
apply_policy('ask', [])
assert ctl('permissions', 'check', 'screen-cast', identity)['level'] == 'ask'
print('PASS: live Lua permission policy, read-only CLI checks, reload and invalid-edit retention')

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
    policy_rules = []
    for capability in ('screen-cast', 'remote-desktop', 'input-capture', 'screenshot', 'access'):
        rule = {'name': capability, 'level': 'allow', 'match': app_identity, 'capabilities': [capability]}
        if capability == 'screen-cast': rule['monitors'] = ['primary']
        if capability == 'remote-desktop': rule['devices'] = ['keyboard', 'pointer']
        policy_rules.append(rule)
    apply_policy('deny', policy_rules)
    result = capture()
    assert result.unpack()[0] == 0 and result.unpack()[1]['streams'], result
    restore = result.get_child_value(1).lookup_value('restore_data', None)
    assert restore is not None, result
    apply_policy('deny', policy_rules + [{'name': 'block-capture', 'level': 'deny', 'match': app_match,
                                          'capabilities': ['screen-cast']}])
    assert capture({'restore_data': restore}).unpack()[0] == 2
    apply_policy('deny', policy_rules)
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
        apply_policy('deny', policy_rules + [{'name': 'block', 'level': 'deny', 'match': app_match,
                                              'capabilities': [capability]}])
        request, session = handles()
        values = (request, app, '', {})
        if capability == 'access': values = (request, app, '', 'Test', 'Test', 'Test', {})
        if capability == 'input-capture': values = (request, session, app, '', {'capabilities': GLib.Variant('u', 3)})
        assert portal(interface, method, signature, values).unpack()[0] != 0
        apply_policy('deny', policy_rules)
    # A forced prompt must ignore otherwise valid restore data and remain cancellable.
    apply_policy('deny', policy_rules + [{'name': 'prompt', 'level': 'ask', 'match': app_match,
                                          'capabilities': ['screen-cast']}])
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
    policy_rules.append({'name': 'native', 'level': 'allow', 'match': '^host-exe:' + re.escape(executable) + '$',
                         'capabilities': ['access']})
    apply_policy('deny', policy_rules)
    request, _ = handles()
    assert portal('Access', 'AccessDialog', '(osssssa{sv})', (request, '', '', 'Test', 'Test', 'Test', {})).unpack()[0] == 0
    apply_policy('ask', policy_rules)
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
