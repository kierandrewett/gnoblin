#!/usr/bin/env python3
"""Verify that a private Gnoblin session does not create native GNOME chrome.

Run with:
  GNOBLIN_TEST_DBUS_CLIENT=$PWD/tests/test-native-chrome.py \\
    scripts/run-gnome-shell.sh

The probe uses the Gnoblin user-script host. It does not need the unsafe Shell
Eval interface, so it also proves the normal control path can inspect the
session after native chrome has been removed.
"""
import ast
import json
import os
from pathlib import Path
import subprocess
import time


assert os.environ.get('WAYLAND_DISPLAY', '').startswith('gnoblin-gs-')

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = Path(os.environ['XDG_CONFIG_HOME']) / 'gnoblin' / 'scripts'
SCRIPTS.mkdir(parents=True, exist_ok=True)


def gdbus(service, path, interface, method, *arguments, ok=True, timeout=5):
    result = subprocess.run([
        'gdbus', 'call', '--session', '--dest', service, '--object-path', path,
        '--method', f'{interface}.{method}', *map(str, arguments),
    ], capture_output=True, text=True, timeout=timeout)
    assert (result.returncode == 0) == ok, result.stdout + result.stderr
    return result


# The script exposes a small D-Bus test interface. This lets the Python half
# force a real workspace transition and check the final Shell state without
# relying on the development-only org.gnome.Shell.Eval method.
(SCRIPTS / 'native-chrome.js').write_text(r'''
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as RemoteAccess from 'resource:///org/gnome/shell/ui/status/remoteAccess.js';

function absent(value, name) {
    if (value !== null)
        throw new Error(`${name} is still constructed`);
}

function nativeChromeState() {
    const backgrounds = Main.layoutManager._bgManagers.map(manager => ({
        menu: Boolean(manager.backgroundActor._backgroundMenu),
        menuManager: Boolean(manager.backgroundActor._backgroundManager),
        reactive: manager.backgroundActor.reactive,
    }));
    return {
        runDialog: Main.runDialog === null,
        welcomeDialog: Main.welcomeDialog === null,
        screenshotUI: Main.screenshotUI === null,
        extensionManager: Main.extensionManager === null,
        osdWindows: Main.osdWindowManager._osdWindows.length,
        osdMonitorLabels: Main.osdMonitorLabeler._osdLabels.length,
        workspacePopup: Main.wm._workspaceSwitcherPopup === null,
        backgrounds,
        activeWorkspace: global.workspace_manager.get_active_workspace_index(),
        workspaces: global.workspace_manager.n_workspaces,
    };
}

export default function (api) {
    // These calls must be harmless in the Gnoblin session. They are a direct
    // behavioural check that legacy entry points cannot make visible chrome.
    // The Gnoblin Run binding is covered by test-developer-console.py. Keep
    // this probe focused on removed native chrome so it does not construct the
    // replacement console as a side effect of the baseline assertion.
    Main.openWelcomeDialog();
    absent(Main.runDialog, 'run dialog');
    absent(Main.welcomeDialog, 'welcome dialog');
    absent(Main.screenshotUI, 'screenshot UI');
    absent(Main.extensionManager, 'extension manager');
    if (Main.osdWindowManager._osdWindows.length !== 0)
        throw new Error('OSD windows exist before an OSD request');
    // The labeler remains as a D-Bus compatibility object. Its Gnoblin mode
    // handler must not make label actors.
    Main.osdMonitorLabeler.show('native-chrome-test', {});
    if (Main.osdMonitorLabeler._osdLabels.length !== 0)
        throw new Error('monitor label request created native label actors');
    Main.osdWindowManager.showAll(null, 'native chrome test', .5, 1);
    if (Main.osdWindowManager._osdWindows.length !== 0)
        throw new Error('OSD request created native OSD actors');
    if (Main.wm._workspaceSwitcherPopup !== null)
        throw new Error('workspace switcher popup exists before a workspace switch');
    for (const background of nativeChromeState().backgrounds) {
        if (!background.menu || !background.menuManager || !background.reactive)
            throw new Error('desktop recovery menu is missing');
    }

    const iface = `<node><interface name="org.gnoblin.NativeChromeTest">
      <method name="Inspect"><arg type="s" direction="out"/></method>
      <method name="SwitchWorkspace"><arg type="i" direction="out"/></method>
      <method name="RestoreWorkspace"><arg type="i" direction="in"/></method>
      <method name="StartUnlockLifecycle"/>
    </interface></node>`;
    const object = Gio.DBusExportedObject.wrapJSObject(iface, {
        Inspect() {
            return JSON.stringify(nativeChromeState());
        },
        SwitchWorkspace() {
            const manager = global.workspace_manager;
            const original = manager.get_active_workspace_index();
            const target = original === 0 ? 1 : 0;
            while (manager.n_workspaces <= target)
                manager.append_new_workspace(false, global.get_current_time());
            Main.wm._showWorkspaceSwitcher(global.display, null, null, {
                get_name: () => `switch-to-workspace-${target + 1}`,
            });
            if (Main.wm._workspaceSwitcherPopup !== null)
                throw new Error('workspace transition created a native popup');
            return original;
        },
        RestoreWorkspace(original) {
            const manager = global.workspace_manager;
            manager.get_workspace_by_index(original).activate(global.get_current_time());
            if (Main.wm._workspaceSwitcherPopup !== null)
                throw new Error('workspace restore created a native popup');
        },
        StartUnlockLifecycle() {
            const originalMode = Main.sessionMode.currentMode;
            Main.sessionMode.pushMode('unlock-dialog');
            const path = GLib.build_filenamev([
                GLib.get_user_config_dir(), 'gnoblin', 'native-chrome-lifecycle.json',
            ]);
            const writeResult = result => Gio.File.new_for_path(path).replace_contents(
                new TextEncoder().encode(JSON.stringify(result)),
                null, false, Gio.FileCreateFlags.REPLACE_DESTINATION, null);
            const deadline = GLib.get_monotonic_time() + 4 * GLib.USEC_PER_SEC;
            let phase = 'locking';
            let osdTransportDuringLock = null;

            // The control component unexports this test D-Bus object when the
            // lock session becomes active. This timer is deliberately owned by
            // the test script rather than the exported object, so it can pop
            // the mode and observe the fresh control component after that.
            GLib.timeout_add(GLib.PRIORITY_DEFAULT, 20, () => {
                try {
                    if (GLib.get_monotonic_time() >= deadline) {
                        writeResult({ok: false, error: `timed out while ${phase}`});
                        return GLib.SOURCE_REMOVE;
                    }

                    if (phase === 'locking') {
                        if (!Main.sessionMode.isLocked)
                            throw new Error('unlock-dialog did not enter locked mode');
                        if (Main.componentManager._enabledComponents.includes('gnoblinControl'))
                            return GLib.SOURCE_CONTINUE;

                        const quickSettings = Main.panel.statusArea.quickSettings;
                        const systemItem = quickSettings?._system?._systemItem;
                        if (!systemItem)
                            return GLib.SOURCE_CONTINUE;
                        if (systemItem.child.get_children().some(
                            child => child.icon_name === 'screenshooter-symbolic'))
                            throw new Error('unlock-dialog created a native screenshot item');

                        absent(Main.screenshotUI, 'screenshot UI during unlock-dialog');
                        absent(Main.extensionManager, 'extension manager during unlock-dialog');
                        osdTransportDuringLock = Object.hasOwn(
                            Main.osdWindowManager, '_showOsdWindow');
                        if (osdTransportDuringLock)
                            throw new Error('OSD forwarding stayed installed during unlock-dialog');
                        Main.sessionMode.popMode('unlock-dialog');
                        phase = 'restoring';
                        return GLib.SOURCE_CONTINUE;
                    }

                    if (!Main.componentManager._enabledComponents.includes('gnoblinControl'))
                        return GLib.SOURCE_CONTINUE;
                    if (Main.sessionMode.currentMode !== originalMode)
                        throw new Error('unlock-dialog did not restore the user mode');
                    absent(Main.screenshotUI, 'screenshot UI after unlock-dialog');
                    absent(Main.extensionManager, 'extension manager after unlock-dialog');
                    // Exercise both screenshot-dependent indicators without
                    // waiting for the asynchronous network/Bluetooth imports.
                    const remoteAccess = new RemoteAccess.RemoteAccessApplet();
                    if (remoteAccess._handles)
                        remoteAccess._isRecording();
                    remoteAccess.destroy();
                    const recording = new RemoteAccess.ScreenRecordingIndicator();
                    recording.destroy();
                    writeResult({ok: true, osdTransportDuringLock});
                } catch (error) {
                    writeResult({ok: false, error: String(error), osdTransportDuringLock});
                }
                return GLib.SOURCE_REMOVE;
            });
        },
    });
    object.export(Gio.DBus.session, '/org/gnoblin/NativeChromeTest');
    const owner = Gio.bus_own_name(
        Gio.BusType.SESSION, 'org.gnoblin.NativeChromeTest',
        Gio.BusNameOwnerFlags.NONE, null, null, null);
    api._disposers.push(() => {
        object.unexport();
        Gio.bus_unown_name(owner);
    });
}
''')

gdbus('org.gnoblin.Shell', '/org/gnoblin/Shell', 'org.gnoblin.Shell', 'ReloadScripts')


def state():
    result = gdbus(
        'org.gnoblin.NativeChromeTest', '/org/gnoblin/NativeChromeTest',
        'org.gnoblin.NativeChromeTest', 'Inspect')
    return json.loads(ast.literal_eval(result.stdout)[0])


before = state()
assert before['runDialog'] and before['welcomeDialog'], before
assert before['screenshotUI'], before
assert before['extensionManager'], before
assert before['osdWindows'] == 0 and before['osdMonitorLabels'] == 0, before
assert before['workspacePopup'], before
assert before['backgrounds'], before
assert all(row['menu'] and row['menuManager'] and row['reactive']
           for row in before['backgrounds']), before

# Workspaces remain compositor state. Switch through the real workspace API,
# wait for the signal-driven state to settle, then restore the starting one.
switched = gdbus(
    'org.gnoblin.NativeChromeTest', '/org/gnoblin/NativeChromeTest',
    'org.gnoblin.NativeChromeTest', 'SwitchWorkspace')
original = ast.literal_eval(switched.stdout)[0]
target = 1 if original == 0 else 0
deadline = time.monotonic() + 3
while time.monotonic() < deadline:
    after_switch = state()
    if after_switch['activeWorkspace'] == target:
        break
    time.sleep(.05)
else:
    raise AssertionError(after_switch)
assert after_switch['workspacePopup'], after_switch
gdbus(
    'org.gnoblin.NativeChromeTest', '/org/gnoblin/NativeChromeTest',
    'org.gnoblin.NativeChromeTest', 'RestoreWorkspace', original)
deadline = time.monotonic() + 3
while time.monotonic() < deadline:
    restored = state()
    if restored['activeWorkspace'] == original:
        break
    time.sleep(.05)
else:
    raise AssertionError(restored)
assert restored['workspacePopup'], restored

# Locking temporarily changes the session component list. The completion file
# survives the script-host unload/reload which this transition can trigger.
lifecycle = Path(os.environ['XDG_CONFIG_HOME']) / 'gnoblin' / 'native-chrome-lifecycle.json'
if lifecycle.exists():
    lifecycle.unlink()
gdbus(
    'org.gnoblin.NativeChromeTest', '/org/gnoblin/NativeChromeTest',
    'org.gnoblin.NativeChromeTest', 'StartUnlockLifecycle')
deadline = time.monotonic() + 5
while time.monotonic() < deadline:
    if lifecycle.exists():
        break
    time.sleep(.05)
else:
    raise AssertionError('unlock-dialog lifecycle did not finish')
lifecycle_result = json.loads(lifecycle.read_text())
assert lifecycle_result == {'ok': True, 'osdTransportDuringLock': False}, lifecycle_result

# The compatibility endpoint must fail promptly after ScreenshotUI removal. A
# direct caller is deliberately untrusted, but this also catches a null object
# path which waits forever instead of returning an error.
result = gdbus(
    'org.gnome.Shell.Screenshot', '/org/gnome/Shell/Screenshot',
    'org.gnome.Shell.Screenshot', 'InteractiveScreenshot', ok=False, timeout=3)
assert result.returncode != 0
assert 'Gnoblin delegates interactive screenshots to the external shell' in result.stderr, result.stderr

print('PASS: no native run, welcome, screenshot, OSD, monitor-label or workspace-popup chrome; desktop recovery menu retained')
print('PASS: workspace state switches and restores without a popup; unlock lifecycle keeps extension/capture UI absent')
print('NOTE: OSD forwarding is intentionally absent while the stock unlock-dialog disables gnoblinControl')
print('PASS: interactive screenshot rejects promptly')
