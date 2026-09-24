#!/usr/bin/env python3
"""Check live configuration inside the isolated run-gnome-shell.sh session."""

import json
import os
from pathlib import Path
import subprocess
import time

root = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
root.mkdir(parents=True, exist_ok=True)
config = root / "init.lua"
scripts = root / "scripts"
scripts.mkdir(exist_ok=True)
probe = scripts / "config-test.js"


def call(method, success=True):
    result = subprocess.run(
        [
            "gdbus",
            "call",
            "--session",
            "--dest",
            "org.gnoblin.Shell",
            "--object-path",
            "/org/gnoblin/Shell",
            "--method",
            f"org.gnoblin.Shell.{method}",
        ],
        capture_output=True,
        text=True,
    )
    if (result.returncode == 0) != success:
        raise RuntimeError(result.stdout + result.stderr)


def check(expected):
    probe.write_text(
        """
import * as Config from 'resource:///org/gnome/shell/ui/components/gnoblinConfig.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import Clutter from 'gi://Clutter';
export default function () {
    const expected = EXPECTED;
    for (const [key, value] of Object.entries(expected)) {
        if (Config.settings[key] !== value)
            throw new Error(`${key}: ${Config.settings[key]} != ${value}`);
    }
    // A disabled switcher must return before accessing the popup arguments.
    if (!expected['window-switcher'])
        Main.wm._startSwitcher(null, null, null, {get_name: () => 'switch-applications'});
    for (const action of ['minimize', 'unminimize']) {
        let completed = false;
        const actor = new Clutter.Actor();
        actor.set_size(800, 600);
        actor.meta_window = {
            is_monitor_sized: () => false,
            get_monitor: () => 0,
            get_icon_geometry: () => [true, {x: 400, y: 700, width: 40, height: 40}],
            get_buffer_rect: () => ({x: 100, y: 100}),
        };
        const wm = {
            _shouldAnimateActor: () => true,
            _minimizing: new Set(), _unminimizing: new Set(),
            _gnoblinAnimationControllers: new Map(),
        };
        // The configuration API's window-rule matcher requires a real Meta.Window.
        // Keep this test's target lightweight while exercising the actual engine resolver.
        wm._resolveGnoblinAnimation = function (event, name, context, options) {
            return Main.wm._resolveGnoblinAnimation.call(this, event, name,
                {...context, window: null}, options);
        };
        Main.wm[`_${action}Window`].call(wm, {
            [`completed_${action}`]() { completed = true; },
        }, actor);
        const entry = wm._gnoblinAnimationControllers.get(actor);
        if (expected['minimize-animation'] === 'none') {
            if (!completed || entry)
                throw new Error(`${action}: none must complete without animation`);
        } else {
            if (!entry || entry.spec.duration !== expected['minimize-duration'])
                throw new Error(`${action}: configured duration`);
            if (entry.spec.event !== (action === 'minimize' ? 'minimize' : 'restore'))
                throw new Error(`${action}: lifecycle event ${entry?.spec.event}`);
            if (action === 'minimize' && (entry.spec.to.x !== 400 || entry.spec.to.y !== 700))
                throw new Error('zoom must target the icon rectangle');
            entry.controller.cancel({restore: true});
        }
    }
}
""".replace("EXPECTED", json.dumps(expected))
    )
    call("Reload")


check({"window-switcher": False, "minimize-animation": "zoom", "minimize-duration": 200})
config.write_text("return {shell = {['window-switcher'] = true, ['minimize-animation'] = 'none'}}\n")
time.sleep(0.5)
check({"window-switcher": True, "minimize-animation": "none", "minimize-duration": 200})
temporary = config.with_suffix(".tmp")
temporary.write_text("return {shell = {['minimize-duration'] = 80}}\n")
temporary.replace(config)
time.sleep(0.5)
check({"window-switcher": False, "minimize-animation": "zoom", "minimize-duration": 80})
config.write_text("return {shell = {['minimize-duration'] = }}\n")
call("ReloadConfig", success=False)
check({"window-switcher": False, "minimize-animation": "zoom", "minimize-duration": 80})
config.write_text("return {shell = {['minimize-duration'] = 50}}\n")
call("ReloadConfig")
check({"window-switcher": False, "minimize-animation": "zoom", "minimize-duration": 50})
config.unlink()
time.sleep(0.5)
check({"window-switcher": False, "minimize-animation": "zoom", "minimize-duration": 200})
config.write_text("return {shell = {osd = false, screenshot = false}}\n")
call("ReloadConfig")
for feature in ["osd", "screenshot"]:
    result = subprocess.check_output(
        [
            "gdbus",
            "call",
            "--session",
            "--dest",
            "org.gnoblin.Shell",
            "--object-path",
            "/org/gnoblin/Shell",
            "--method",
            "org.gnoblin.Shell.GetFeature",
            feature,
        ],
        text=True,
    )
    assert result.strip() == "(false,)", result
config.write_text("return {shell = {osd = true, screenshot = true}}\n")
time.sleep(0.5)
for feature in ["osd", "screenshot"]:
    result = subprocess.check_output(
        [
            "gdbus",
            "call",
            "--session",
            "--dest",
            "org.gnoblin.Shell",
            "--object-path",
            "/org/gnoblin/Shell",
            "--method",
            "org.gnoblin.Shell.GetFeature",
            feature,
        ],
        text=True,
    )
    assert result.strip() == "(false,)", result
# Named commands run once, including after they exit and the file reloads.
started = root / "autostart-count"
command = ["sh", "-c", 'echo started >> "$1"', "autostart-test", str(started)]
lua_command = "{" + ", ".join(json.dumps(argument) for argument in command) + "}"
config.write_text("return {autostart = {{name = 'probe', command = " + lua_command + "}}}\n")
call("ReloadConfig")
time.sleep(0.3)
call("ReloadConfig")
time.sleep(0.3)
assert started.read_text().splitlines() == ["started"]
config.write_text(
    "return {autostart = {{name = 'probe', command = "
    + lua_command
    + "}, {name = 'second', command = "
    + lua_command
    + "}}}\n"
)
time.sleep(0.5)
assert started.read_text().splitlines() == ["started", "started"]
print("PASS: autostart runs once per name and accepts newly added entries")
print("PASS: live feature configuration")
print("PASS: live shell watcher, manual reload, invalid-file recovery, minimise/restore, switcher gate")

init = config
module = root / "appearance.lua"
module.write_text("return {shell = {['minimize-animation'] = 'none'}}\n")
init.write_text("""local g = require('gnoblin')
g.load('appearance.lua')
g.set({shell = {['minimize-duration'] = 3 * 40}})
""")
time.sleep(0.6)
check({"window-switcher": False, "minimize-animation": "none", "minimize-duration": 120})
module.with_suffix(".tmp").write_text("return {shell = {['minimize-animation'] = 'zoom'}}\n")
module.with_suffix(".tmp").replace(module)
time.sleep(0.6)
check({"window-switcher": False, "minimize-animation": "zoom", "minimize-duration": 120})
module.write_text("this is not valid Lua")
call("ReloadConfig", success=False)
check({"window-switcher": False, "minimize-animation": "zoom", "minimize-duration": 120})
module.write_text("return {shell = {['minimize-animation'] = 'none'}}\n")
time.sleep(0.6)
check({"window-switcher": False, "minimize-animation": "none", "minimize-duration": 120})
# A normal Lua module can also load a packaged component and override its defaults.
bingux = Path(__file__).resolve().parents[2] / "bingux/packaging/gnoblin/bingux.lua"
if bingux.is_file():
    init.write_text(
        "local g = require('gnoblin')\ng.load(" + json.dumps(str(bingux)) + ")\n"
        "g.set({shell = {['minimize-animation'] = 'none', ['minimize-duration'] = 65}})\n"
    )
    call("ReloadConfig")
    check({"window-switcher": False, "minimize-animation": "none", "minimize-duration": 65})
    result = subprocess.check_output(
        [
            "gdbus",
            "call",
            "--session",
            "--dest",
            "org.gnoblin.Shell",
            "--object-path",
            "/org/gnoblin/Shell",
            "--method",
            "org.gnoblin.Shell.GetFeature",
            "osd",
        ],
        text=True,
    )
    assert result.strip() == "(false,)", result
    print("PASS: packaged Bingux Lua settings apply through live Shell")
# A wildcard initially has no matches. Adding a nested file must reload it.
init.write_text("require('gnoblin').load('conf.d/**/*.lua')\n")
call("ReloadConfig")
check({"window-switcher": False, "minimize-animation": "zoom", "minimize-duration": 200})
fragment = root / "conf.d/local/50-motion.lua"
fragment.parent.mkdir(parents=True)
fragment.write_text("return {shell = {['minimize-animation'] = 'none', ['minimize-duration'] = 95}}\n")
time.sleep(0.6)
check({"window-switcher": False, "minimize-animation": "none", "minimize-duration": 95})
fragment.unlink()
time.sleep(0.6)
check({"window-switcher": False, "minimize-animation": "zoom", "minimize-duration": 200})
print("PASS: live recursive glob detects new and removed user config files")
init.unlink()
time.sleep(0.6)
check({"window-switcher": False, "minimize-animation": "zoom", "minimize-duration": 200})
print("PASS: live Lua module reload, invalid-edit recovery, and user overrides")
