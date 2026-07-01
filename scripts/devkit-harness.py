#!/usr/bin/env python3
# gnoblin devkit harness — "playwright for the devkit".
#
# Drives a headless gnoblin-shell entirely from the LLM/CI side: boot it in an
# isolated dbus + XDG sandbox, screenshot it (grim), inject pointer/keyboard
# (mutter RemoteDesktop), add a virtual monitor *late* at runtime to reproduce
# the devkit's late-monitor flow (mutter ScreenCast RecordVirtual + a PipeWire
# consumer), and detect compositor aborts — with no human running `just devkit`
# and pasting logs.
#
# Why a long-lived Python process and not gdbus one-shots: a mutter ScreenCast /
# RemoteDesktop session is torn down the instant the D-Bus connection that
# created it closes. The harness therefore owns ONE persistent Gio connection to
# a private session bus that it also spawns gnoblin-shell on, and holds the
# session objects alive for the lifetime of the run. It also owns the lifecycle
# of its private dbus-daemon, so it never leaks one (the bug that exhausted the
# inotify-instance limit and broke the real devkit).
#
# Usage:
#   devkit-harness.py shot [OUT.png]      boot (monitor at start), settle, grim, crash-check
#   devkit-harness.py late [OUT.png]      boot with NO monitor, start clients, add a monitor
#                                         LATE via ScreenCast — reproduces the devkit late flow
#   devkit-harness.py storm               add a late monitor then renegotiate its size in a
#                                         tight loop — exercises the configure-storm path
#   devkit-harness.py run CLIENT [args]   boot a bare compositor, run an arbitrary layer-shell
#                                         client, report COMPOSITOR SURVIVED/CRASHED (rc 10)
#   devkit-harness.py keys SPEC [OUT]     inject a key chord via mutter RemoteDesktop, then
#                                         screenshot. SPEC = 'Super+Space' or 'Super+Space:calc'
#                                         (chord, then type the text after ':')
#   devkit-harness.py click 'X,Y' [OUT]   pointer click via RemoteDesktop + linked ScreenCast
#   devkit-harness.py wm 'ops' [OUT]      drive the window manager over dev.gnoblin.Shell. ops is
#                                         comma-separated: spawn:foot,maximize,snap:left,minimize,
#                                         close — prints window state after each, screenshots OUT
#   devkit-harness.py inspect ['ops'] [OUT]  run optional ops (same syntax as wm), then dump the
#                                         live scene: every surface's frame/buffer rect, csd_inset
#                                         and the resolved+attached gnoblin effects (round/ring/blur)
#                                         — accurate rendering truth, not eyeballed screenshots
#   devkit-harness.py smoke               boot + crash-check + teardown, prints PASS/CRASH
#   devkit-harness.py boot                boot and keep alive (prints WAYLAND_DISPLAY), Ctrl-C
#
# Env: GNOBLIN_PREFIX (default <repo>/install), MONITOR (default 1280x800),
#      HIDPI (default 1; set 2 for crisp "retina" screenshots — renders the
#      virtual output at 2x pixels with output scale 2, keeping the logical
#      1280x800 layout but capturing 2560x1600). E.g. HIDPI=2 …harness inspect
#      CLIENTS=0 to boot a bare compositor, SETTLE seconds (default 7).
import os, sys, time, signal, shutil, socket, subprocess, tempfile, pathlib

import gi
gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import devkit_dbus
from devkit_crash_detector import match_crash_log

PREFIX = pathlib.Path(os.environ.get("GNOBLIN_PREFIX", ROOT / "install"))
SHELL_BIN = PREFIX / "bin" / "gnoblin-shell"
MONITOR = os.environ.get("MONITOR", "1280x800")
# HiDPI / "retina" capture: render the virtual output at HIDPI× the logical size
# with output scale = HIDPI, so the UI keeps its 1280x800 logical layout but is
# drawn (and grim-captured) at HIDPI× the pixels — crisp screenshots. Default 1.
HIDPI = float(os.environ.get("HIDPI", "1"))


def _logical_wh():
    w, h = (int(x) for x in MONITOR.split("x"))
    return w, h


def _phys_wh():
    """Physical (rendered/captured) resolution = logical × HiDPI scale."""
    w, h = _logical_wh()
    return int(round(w * HIDPI)), int(round(h * HIDPI))


def _phys_spec():
    w, h = _phys_wh()
    return f"{w}x{h}"


CLIENTS = os.environ.get("CLIENTS", "1") == "1"
# Dummy quick-settings plugin scripts (FAKE data — never touch the host's real
# network/audio/media/bluetooth). Injected as [qs-plugin.*] so the devkit's
# control centre shows plugin tiles. Disable with QS_PLUGINS=0.
PROVIDERS_DIR = ROOT / "tests" / "devkit" / "providers"
QS_PLUGINS = os.environ.get("QS_PLUGINS", "1") == "1"
SETTLE = float(os.environ.get("SETTLE", "7"))

def log(msg):
    print(f"[harness] {msg}", flush=True)


class Devkit:
    def __init__(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp(prefix="gnoblin-harness."))
        suffix = self.tmp.name.split(".", 1)[-1]
        self.disp = f"gnoblin-harness-{os.getpid()}-{suffix}"
        self.dbus_proc = None
        self.dbus_addr = None
        self.shell_proc = None
        self.shell_log = self.tmp / "shell.log"
        self.conn = None          # persistent Gio session-bus connection
        self._consumers = []      # gst PipeWire consumers (keep monitors alive)
        self._rd_consumers = []   # gst consumers for RemoteDesktop pointer mapping
        self._sc_session = None   # ScreenCast session proxy (must stay alive)
        self._sc_node = None      # last ScreenCast pipewire node id
        self._rd_session = None   # RemoteDesktop session proxy (input injection)
        self._rd_started = False
        self._rd_stream_path = None   # ScreenCast stream linked to RD (pointer map)
        self._sc_for_rd = None        # the linked ScreenCast session (keep alive)
        self.extra_appearance = ""    # extra lines appended to [appearance] (e.g. wallpaper)
        self.extra_conf = os.environ.get("GNOBLIN_EXTRA_CONF", "").replace("\\n", "\n")  # raw config appended at the end

    # --- sandbox + environment ------------------------------------------------
    def _env(self):
        env = dict(os.environ)
        env["LD_LIBRARY_PATH"] = f"{PREFIX}/lib64:{PREFIX}/lib64/mutter-17" + \
            (":" + env["LD_LIBRARY_PATH"] if env.get("LD_LIBRARY_PATH") else "")
        env["GI_TYPELIB_PATH"] = f"{PREFIX}/lib64/mutter-17"
        env["PATH"] = f"{PREFIX}/bin:" + env.get("PATH", "")
        env["GSETTINGS_SCHEMA_DIR"] = f"{PREFIX}/share/glib-2.0/schemas"
        env["XDG_DATA_DIRS"] = f"{PREFIX}/share:/usr/share"
        env["GDK_BACKEND"] = "wayland"
        env["GSETTINGS_BACKEND"] = "memory"
        env["GTK_A11Y"] = "none"
        env["NO_AT_BRIDGE"] = "1"
        env["GIO_USE_VFS"] = "local"
        env["GVFS_DISABLE_FUSE"] = "1"
        env["XDG_CURRENT_DESKTOP"] = "GNOME:Gnoblin"
        env["XDG_DATA_HOME"] = str(self.tmp / "data")
        env["XDG_CONFIG_HOME"] = str(self.tmp / "config")
        env["XDG_CACHE_HOME"] = str(self.tmp / "cache")
        env["HOME"] = str(self.tmp / "home")
        # Make Slint clients emit their inspection sidecars (icon resolutions etc.)
        # under $XDG_RUNTIME_DIR/gnoblin-inspect/. Cheap; harmless when unread.
        env["GNOBLIN_INSPECT"] = "1"
        # HiDPI: mutter only does proper framebuffer scaling (logical = physical /
        # scale, crisp 2× buffers) with the scale-monitor-framebuffer experimental
        # feature. The sandbox gsettings is empty (memory backend), so enable it
        # via mutter's env override — otherwise scale 2 keeps logical == the full
        # mode and clients render 2× into a 1× framebuffer (top-left quarter only).
        if HIDPI != 1.0:
            env["MUTTER_DEBUG_EXPERIMENTAL_FEATURES"] = "scale-monitor-framebuffer"
        if self.dbus_addr:
            env["DBUS_SESSION_BUS_ADDRESS"] = self.dbus_addr
        return env

    def _write_conf(self, per_output=True):
        cfgdir = self.tmp / "config" / "gnoblin"
        cfgdir.mkdir(parents=True, exist_ok=True)
        portal_dir = self.tmp / "config" / "xdg-desktop-portal"
        portal_dir.mkdir(parents=True, exist_ok=True)
        (portal_dir / "gnoblin-portals.conf").write_text(
            "[preferred]\n"
            "default=gtk\n"
            "org.freedesktop.impl.portal.ScreenCast=gnome\n"
            "org.freedesktop.impl.portal.RemoteDesktop=gnome\n"
            "org.freedesktop.impl.portal.Screenshot=gnome\n"
            "org.freedesktop.impl.portal.GlobalShortcuts=gnome\n"
            "org.freedesktop.impl.portal.Background=none\n"
            "org.freedesktop.impl.portal.Clipboard=none\n"
            "org.freedesktop.impl.portal.InputCapture=none\n"
            "org.freedesktop.impl.portal.Lockdown=none\n"
            "org.freedesktop.impl.portal.Secret=none\n"
            "org.freedesktop.impl.portal.Usb=none\n"
            "org.freedesktop.impl.portal.Wallpaper=none\n"
        )
        kw = "exec_per_output" if per_output else "exec"
        startup = ""
        if CLIENTS:
            startup = (f"{kw} = gnoblin-topbar\n{kw} = gnoblin-dock\n"
                       f"exec = gnoblin-notifyd\n{kw} = gnoblin-wallpaper\n")
        wallpaper = pathlib.Path(os.environ.get(
            "GNOBLIN_WALLPAPER",
            str(pathlib.Path.home() / "Documents" / "wallpaper_light.jpg"),
        ))
        wallpaper_conf = ""
        if wallpaper.is_file():
            wallpaper_conf = f"wallpaper = {wallpaper}\nwallpaper-style = zoom\n"
        # Dummy QS plugins: each writes its event log under the sandbox's cache
        # dir so callbacks can be asserted; absolute paths so they always resolve.
        qs_conf = ""
        if QS_PLUGINS and CLIENTS and PROVIDERS_DIR.is_dir():
            qs_log = self.tmp / "cache" / "gnoblin-qs-events.log"
            qs_state = self.tmp / "cache"
            def plugin(name, command, mode, interval=""):
                env = f"GNOBLIN_QS_LOG={qs_log} GNOBLIN_QS_STATE={qs_state}/qs-{name}.state "
                block = (f"[qs-plugin.{name}]\n"
                         f"command = {env}{PROVIDERS_DIR / command}\n"
                         f"mode = {mode}\n")
                if interval:
                    block += f"interval = {interval}\n"
                return block
            qs_conf = (plugin("wifi", "wifi", "oneshot", "4s")
                       + plugin("audio", "audio", "oneshot", "4s")
                       + plugin("bluetooth", "bluetooth", "oneshot", "4s")
                       + plugin("mpris", "mpris", "persistent"))
        # Launcher providers (process/command search sources), prefix-gated so
        # they only run on their keyword. Registered whenever clients are up.
        launcher_conf = ""
        if CLIENTS and PROVIDERS_DIR.is_dir():
            launcher_conf = (
                f'[launcher-provider.web]\ncommand = {PROVIDERS_DIR / "launcher-web"}\nprefix = "g "\n'
                f'[launcher-provider.files]\ncommand = {PROVIDERS_DIR / "launcher-files"}\nprefix = "f "\n'
                f'[launcher-provider.emoji]\ncommand = {PROVIDERS_DIR / "launcher-emoji"}\nprefix = "e "\n'
                f'[launcher-provider.kill]\ncommand = {PROVIDERS_DIR / "launcher-kill"}\nprefix = "k "\n'
                f'[launcher-provider.convert]\ncommand = {PROVIDERS_DIR / "launcher-convert"}\nprefix = "c "\n'
                f'[launcher-provider.color]\ncommand = {PROVIDERS_DIR / "launcher-color"}\nprefix = "# "\n'
                f'[launcher-provider.base]\ncommand = {PROVIDERS_DIR / "launcher-base"}\nprefix = "b "\n'
                f'[launcher-provider.time]\ncommand = {PROVIDERS_DIR / "launcher-time"}\nprefix = "t "\n'
                '[launcher]\nweb-search = https://duckduckgo.com/?q=%s\n'
            )
        # HiDPI: apply the output scale via mutter's monitors.xml (read at OUTPUT
        # CREATION) rather than a runtime [output] override. mutter does NOT resize
        # already-created layer surfaces when the scale changes at runtime, so a
        # runtime override leaves the wallpaper/dock sized for the unscaled output
        # (rendering their top-left quarter). monitors.xml gets the scale right
        # before any surface — wallpaper/topbar/dock/windows all come up correct.
        output_conf = ""
        if HIDPI != 1.0:
            (cfgdir.parent / "monitors.xml").write_text(
                '<monitors version="2">\n'
                '  <configuration>\n'
                '    <logicalmonitor>\n'
                '      <x>0</x>\n      <y>0</y>\n'
                f'      <scale>{HIDPI:g}</scale>\n'
                '      <primary>yes</primary>\n'
                '      <monitor>\n'
                '        <monitorspec>\n'
                '          <connector>Meta-0</connector>\n'
                '          <vendor>MetaVendor</vendor>\n'
                '          <product>MetaVirtualMonitor</product>\n'
                '          <serial>0x00</serial>\n'
                '        </monitorspec>\n'
                '        <mode>\n'
                f'          <width>{_phys_wh()[0]}</width>\n'
                f'          <height>{_phys_wh()[1]}</height>\n'
                '          <rate>60.000</rate>\n'
                '        </mode>\n'
                '      </monitor>\n'
                '    </logicalmonitor>\n'
                '  </configuration>\n'
                '</monitors>\n'
            )
        # ANIM_SPEED: force animations on + a global playback-speed multiplier
        # (0.1 = 10x slower, for frame-by-frame filmstrip capture of transitions).
        anim_conf = ""
        anim_speed = os.environ.get("ANIM_SPEED", "")
        if anim_speed:
            anim_conf = f"[animations]\nenabled = on\nanimation-speed = {anim_speed}\n"
        (cfgdir / "gnoblin.conf").write_text(
            output_conf + anim_conf +
            "[appearance]\n"
            'background = "#202434"\n'
            + wallpaper_conf +
            "rounding = 14\n"
            'shadow = "0 20px 48px -20px rgba(0,0,0,.22), 0 4px 12px -6px rgba(0,0,0,.14)"\n'
            + (self.extra_appearance or "") +
            "[startup]\n" + startup +
            "[roles]\nwindow-menu = gnoblin-window-menu\n"
            "[topbar]\n"
            "left = workspaces, focused-app, appmenu, spring\n"
            "center = clock\n"
            "right = launcher, tray, status\n"
            "appmenu-backend = auto\n"
            "[bind]\n"
            "Super+Space = spawn gnoblin-launcher\n"
            "Super+F12 = spawn sh -c 'mkdir -p \"${XDG_RUNTIME_DIR:-/tmp}/gnoblin-inspect\"; gnoblinctl inspect --pretty \"${XDG_RUNTIME_DIR:-/tmp}/gnoblin-inspect/scene.json\"'\n"
            "Super+Q = close\n"
            "Super+Escape = window-menu\n"
            + qs_conf
            + launcher_conf
            + (self.extra_conf or "")
        )

    # --- private session bus (owned, never leaked) ----------------------------
    def _write_dbus_config(self):
        return devkit_dbus.write_config(self.tmp, ROOT)

    def start_bus(self):
        conf = self._write_dbus_config()
        p = subprocess.Popen(
            ["dbus-daemon", f"--config-file={conf}", "--nofork", "--print-address"],
            env=self._env(), stdout=subprocess.PIPE, text=True)
        self.dbus_proc = p
        self.dbus_addr = p.stdout.readline().strip()
        if not self.dbus_addr:
            raise RuntimeError("dbus-daemon gave no address")
        flags = (Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT |
                 Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION)
        self.conn = Gio.DBusConnection.new_for_address_sync(
            self.dbus_addr, flags, None, None)
        log(f"private bus: {self.dbus_addr}")

    # --- boot -----------------------------------------------------------------
    def boot(self, with_monitor=True, per_output=True, monitors=None, command=None):
        """Boot gnoblin-shell. monitors: list of WxH specs for multiple outputs
        (mutter accepts repeated --virtual-monitor); defaults to a single MONITOR
        when with_monitor. Pass with_monitor=False to start headless with none.
        command, when provided, is appended after `--` and spawned by the shell."""
        for d in ("data", "config", "cache", "home"):
            (self.tmp / d).mkdir(parents=True, exist_ok=True)
        self._write_conf(per_output=per_output)
        if self.dbus_proc is None:
            self.start_bus()
        args = [str(SHELL_BIN), "--headless", "--no-x11",
                "--wayland-display", self.disp]
        if monitors is None and with_monitor:
            # Physical resolution = logical × HiDPI; [output] scale brings the
            # logical layout back to MONITOR so the UI is unchanged but crisp.
            monitors = [_phys_spec()]
        for spec in (monitors or []):
            args += ["--virtual-monitor", spec]
        if command:
            args += ["--", *command]
        logf = open(self.shell_log, "wb")
        self.shell_proc = subprocess.Popen(args, env=self._env(),
                                            stdout=logf, stderr=subprocess.STDOUT)
        sock = pathlib.Path(os.environ["XDG_RUNTIME_DIR"]) / self.disp
        for _ in range(60):
            if sock.exists():
                break
            if self.shell_proc.poll() is not None:
                raise RuntimeError(f"shell died on boot:\n{self._tail()}")
            time.sleep(0.5)
        else:
            raise RuntimeError(f"shell never created {sock}:\n{self._tail()}")
        log(f"booted: WAYLAND_DISPLAY={self.disp} (monitor={'yes' if with_monitor else 'LATE'})")



    def crashed(self):
        if self.shell_proc and self.shell_proc.poll() is not None:
            rc = self.shell_proc.returncode
            if rc != 0:
                return f"shell exited rc={rc}"
        txt = self.shell_log.read_text(errors="replace") if self.shell_log.exists() else ""
        if line := match_crash_log(txt):
            return f"log match: {line}"
        return None

    def _tail(self, n=25):
        if not self.shell_log.exists():
            return "(no log)"
        return "\n".join(self.shell_log.read_text(errors="replace").splitlines()[-n:])

    # --- screenshot -----------------------------------------------------------
    def shot(self, out):
        if self._rd_stream_path is not None or self._rd_consumers:
            self.stop_remote_desktop()
        env = self._env()
        env["WAYLAND_DISPLAY"] = self.disp
        timeout = float(os.environ.get("SHOT_TIMEOUT", "10"))
        try:
            r = subprocess.run(
                ["grim", str(out)],
                env=env,
                capture_output=True,
                text=True,
                timeout=timeout,
            )
        except subprocess.TimeoutExpired as e:
            raise RuntimeError(
                f"grim timed out after {timeout:g}s capturing {out}\n"
                f"stdout: {(e.stdout or '').strip()}\n"
                f"stderr: {(e.stderr or '').strip()}\n"
                f"shell log tail:\n{self._tail()}"
            ) from e
        if r.returncode != 0 or not pathlib.Path(out).exists():
            raise RuntimeError(f"grim failed: {r.stderr.strip()}")
        log(f"screenshot -> {out} ({pathlib.Path(out).stat().st_size} bytes)")

    # --- late virtual monitor via ScreenCast RecordVirtual --------------------
    def add_monitor_late(self, w=1280, h=800):
        sc = Gio.DBusProxy.new_sync(
            self.conn, Gio.DBusProxyFlags.NONE, None,
            "org.gnome.Mutter.ScreenCast", "/org/gnome/Mutter/ScreenCast",
            "org.gnome.Mutter.ScreenCast", None)
        sess_path = sc.call_sync("CreateSession", GLib.Variant("(a{sv})", ({},)),
                                 Gio.DBusCallFlags.NONE, -1, None).unpack()[0]
        self._sc_session = Gio.DBusProxy.new_sync(
            self.conn, Gio.DBusProxyFlags.NONE, None,
            "org.gnome.Mutter.ScreenCast", sess_path,
            "org.gnome.Mutter.ScreenCast.Session", None)
        props = {
            "is-platform": GLib.Variant("b", True),
            "cursor-mode": GLib.Variant("u", 1),
        }
        stream_path = self._sc_session.call_sync(
            "RecordVirtual", GLib.Variant("(a{sv})", (props,)),
            Gio.DBusCallFlags.NONE, -1, None).unpack()[0]
        node_holder = {}
        stream = Gio.DBusProxy.new_sync(
            self.conn, Gio.DBusProxyFlags.NONE, None,
            "org.gnome.Mutter.ScreenCast", stream_path,
            "org.gnome.Mutter.ScreenCast.Stream", None)
        def on_signal(proxy, sender, sig, params):
            if sig == "PipeWireStreamAdded":
                node_holder["id"] = params.unpack()[0]
        stream.connect("g-signal", on_signal)
        self._sc_session.call_sync("Start", None, Gio.DBusCallFlags.NONE, -1, None)
        # Pump the main loop until the node id arrives.
        ctx = GLib.MainContext.default()
        for _ in range(200):
            if "id" in node_holder:
                break
            ctx.iteration(False)
            time.sleep(0.02)
        if "id" not in node_holder:
            raise RuntimeError("no PipeWireStreamAdded — virtual monitor not offered")
        node = node_holder["id"]
        self._sc_node = node
        log(f"ScreenCast virtual stream offered: pipewire node {node}")
        # A PipeWire consumer materializes the monitor. mutter's virtual stream
        # offers a size *range* (it has no intrinsic size) — the monitor is only
        # created once the consumer pins a concrete size via caps negotiation
        # (create_virtual_monitor reads video_format->size). So fix W/H in caps.
        # videoconvert is required for the caps to negotiate (fakesink alone
        # cannot reconcile the producer's format) — without it the monitor never
        # materializes. Pinning width/height fixes the negotiated size.
        cons = subprocess.Popen(
            ["gst-launch-1.0", "pipewiresrc", f"path={node}", "do-timestamp=true",
             "!", f"video/x-raw,width={w},height={h}",
             "!", "videoconvert", "!", "fakesink", "sync=false"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self._consumers.append(cons)
        log("PipeWire consumer attached (pinned size) — waiting for wl_output ...")
        if self.wait_for_output():
            log("wl_output materialized — late monitor is live")
            return True
        log("!! wl_output never appeared — negotiation incomplete")
        return False

    def resize_storm(self, sizes, node):
        """Renegotiate the virtual monitor size repeatedly while clients render.
        Each new consumer caps size drives ensure_virtual_monitor -> set_mode ->
        monitor_manager_reload, which resends layer-surface configures. Doing this
        in a tight loop reproduces the devkit's configure storm (the client acks
        configure N and attaches a buffer while mutter has already sent N+1)."""
        for i, (w, h) in enumerate(sizes):
            for c in self._consumers:
                c.terminate()
            for c in self._consumers:
                try:
                    c.wait(timeout=1)
                except Exception:
                    c.kill()
            self._consumers.clear()
            cons = subprocess.Popen(
                ["gst-launch-1.0", "pipewiresrc", f"path={node}", "do-timestamp=true",
                 "!", f"video/x-raw,width={w},height={h}",
                 "!", "videoconvert", "!", "fakesink", "sync=false"],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            self._consumers.append(cons)
            log(f"storm step {i}: renegotiate {w}x{h}")
            # short, jittered settle so a configure lands mid client-render
            time.sleep(0.4 if i % 2 else 0.25)
            if self.crashed():
                return False
        return True

    # --- input injection via mutter RemoteDesktop -----------------------------
    # evdev keycodes (linux/input-event-codes.h) — what NotifyKeyboardKeycode and
    # Clutter's virtual device expect (raw, no XKB +8 offset).
    KEYS = {
        "super": 125, "ctrl": 29, "control": 29, "alt": 56, "shift": 42,
        "space": 57, "return": 28, "enter": 28, "escape": 1, "esc": 1,
        "tab": 15, "backspace": 14, "delete": 111, "up": 103, "down": 108,
        "left": 105, "right": 106,
        **{f"f{i}": kc for i, kc in enumerate(
            [59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 87, 88], start=1)},
        **{c: kc for c, kc in zip("1234567890", range(2, 12))},
        **{c: kc for c, kc in zip(
            "qwertyuiop", [16,17,18,19,20,21,22,23,24,25])},
        **{c: kc for c, kc in zip(
            "asdfghjkl", [30,31,32,33,34,35,36,37,38])},
        **{c: kc for c, kc in zip("zxcvbnm", [44,45,46,47,48,49,50])},
    }

    def start_remote_desktop(self, pointer=False):
        if self._rd_session is None:
            rd = Gio.DBusProxy.new_sync(
                self.conn, Gio.DBusProxyFlags.NONE, None,
                "org.gnome.Mutter.RemoteDesktop", "/org/gnome/Mutter/RemoteDesktop",
                "org.gnome.Mutter.RemoteDesktop", None)
            sess_path = rd.call_sync("CreateSession", None,
                                     Gio.DBusCallFlags.NONE, -1, None).unpack()[0]
            self._rd_session = Gio.DBusProxy.new_sync(
                self.conn, Gio.DBusProxyFlags.NONE, None,
                "org.gnome.Mutter.RemoteDesktop", sess_path,
                "org.gnome.Mutter.RemoteDesktop.Session", None)
        # Pointer-absolute needs a linked ScreenCast stream of the EXISTING monitor
        # so clicks land where the dock/topbar actually are; the stream's coords
        # only transform once a PipeWire consumer has configured it. Keyboard needs
        # none of this. The linked screencast streams only start after RD Start(),
        # so: RecordMonitor -> Start -> wait for node -> attach consumer.
        if pointer and self._rd_stream_path is None:
            sid_v = self._rd_session.get_cached_property("SessionId")
            sid = sid_v.unpack() if sid_v else None
            if not sid:
                raise RuntimeError("no RemoteDesktop SessionId for pointer linkage")
            sc = Gio.DBusProxy.new_sync(
                self.conn, Gio.DBusProxyFlags.NONE, None,
                "org.gnome.Mutter.ScreenCast", "/org/gnome/Mutter/ScreenCast",
                "org.gnome.Mutter.ScreenCast", None)
            props = {"remote-desktop-session-id": GLib.Variant("s", sid)}
            sc_path = sc.call_sync("CreateSession", GLib.Variant("(a{sv})", (props,)),
                                   Gio.DBusCallFlags.NONE, -1, None).unpack()[0]
            self._sc_for_rd = Gio.DBusProxy.new_sync(
                self.conn, Gio.DBusProxyFlags.NONE, None,
                "org.gnome.Mutter.ScreenCast", sc_path,
                "org.gnome.Mutter.ScreenCast.Session", None)
            stream_path = self._sc_for_rd.call_sync(
                "RecordMonitor", GLib.Variant("(sa{sv})", ("Meta-0", {})),
                Gio.DBusCallFlags.NONE, -1, None).unpack()[0]
            self._rd_stream_path = stream_path
            holder = {}
            stream = Gio.DBusProxy.new_sync(
                self.conn, Gio.DBusProxyFlags.NONE, None,
                "org.gnome.Mutter.ScreenCast", stream_path,
                "org.gnome.Mutter.ScreenCast.Stream", None)
            stream.connect("g-signal", lambda p, s, sig, par:
                           holder.update(id=par.unpack()[0])
                           if sig == "PipeWireStreamAdded" else None)
            self._rd_session.call_sync("Start", None, Gio.DBusCallFlags.NONE, -1, None)
            self._rd_started = True
            ctx = GLib.MainContext.default()
            for _ in range(200):
                if "id" in holder:
                    break
                ctx.iteration(False)
                time.sleep(0.02)
            if "id" not in holder:
                raise RuntimeError("no PipeWireStreamAdded for the monitor stream")
            w, h = (int(x) for x in MONITOR.split("x"))
            cons = subprocess.Popen(
                ["gst-launch-1.0", "pipewiresrc", f"path={holder['id']}",
                 "!", f"video/x-raw,width={w},height={h}", "!", "videoconvert",
                 "!", "fakesink", "sync=false"],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            self._rd_consumers.append(cons)
            # The stream must finish PipeWire format negotiation before
            # transform_position works (else motion is silently dropped). Pump
            # the loop and give it time to reach STREAMING.
            ctx = GLib.MainContext.default()
            t0 = time.time()
            while time.time() - t0 < 3.5:
                ctx.iteration(False)
                time.sleep(0.05)
            log("RemoteDesktop+ScreenCast ready (pointer injection on Meta-0)")
            return
        if pointer and self._rd_started and self._rd_stream_path is None:
            # The screencast stream must be linked BEFORE Start(); once a
            # keyboard-only session has started we cannot add it. Callers that
            # need pointer must not inject keys first (or use a fresh harness).
            raise RuntimeError(
                "pointer requested after a keyboard-only session already started; "
                "do pointer actions before keyboard, or use a separate run")
        if not self._rd_started:
            self._rd_session.call_sync("Start", None, Gio.DBusCallFlags.NONE, -1, None)
            self._rd_started = True
            log("RemoteDesktop session started (keyboard injection ready)")

    def move(self, x, y):
        self.start_remote_desktop(pointer=True)
        self._rd_session.call_sync(
            "NotifyPointerMotionAbsolute",
            GLib.Variant("(sdd)", (self._rd_stream_path, float(x), float(y))),
            Gio.DBusCallFlags.NONE, -1, None)

    def click(self, x, y, button=272):  # 272 = BTN_LEFT, 273 = BTN_RIGHT
        # Send the motion twice: the first transform right after the stream
        # configures can still be dropped, the second reliably lands.
        self.move(x, y)
        time.sleep(0.1)
        self.move(x, y)
        time.sleep(0.1)
        for pressed in (True, False):
            self._rd_session.call_sync(
                "NotifyPointerButton", GLib.Variant("(ib)", (button, pressed)),
                Gio.DBusCallFlags.NONE, -1, None)
            time.sleep(0.05)
        log(f"clicked ({x},{y}) button={button}")

    def scroll(self, x, y, steps):
        """Scroll the wheel by `steps` discrete notches at (x, y). steps > 0
        scrolls down (content moves up), < 0 scrolls up. Used to reach content
        below the fold in a capped/scrolling popout."""
        self.move(x, y)
        time.sleep(0.05)
        self.move(x, y)
        time.sleep(0.05)
        axis = 0  # 0 = vertical, 1 = horizontal
        direction = 1 if steps >= 0 else -1
        for _ in range(abs(int(steps))):
            # org.gnome.Mutter.RemoteDesktop.Session.NotifyPointerAxisDiscrete(u axis, i steps)
            self._rd_session.call_sync(
                "NotifyPointerAxisDiscrete",
                GLib.Variant("(ui)", (axis, direction)),
                Gio.DBusCallFlags.NONE, -1, None)
            time.sleep(0.04)
        log(f"scrolled ({x},{y}) steps={steps}")

    def stop_remote_desktop(self):
        """Close the current RemoteDesktop session and its linked ScreenCast stream."""
        had_session = self._rd_session is not None or self._rd_consumers
        for c in self._rd_consumers:
            c.terminate()
        for c in self._rd_consumers:
            try:
                c.wait(timeout=2)
            except Exception:
                c.kill()
        self._rd_consumers.clear()
        if self._rd_session is not None and self._rd_started:
            try:
                self._rd_session.call_sync("Stop", None, Gio.DBusCallFlags.NONE, 2000, None)
            except Exception as e:
                log(f"RemoteDesktop Stop ignored: {e}")
        if self._sc_for_rd is not None:
            try:
                self._sc_for_rd.call_sync("Stop", None, Gio.DBusCallFlags.NONE, 2000, None)
            except Exception:
                pass
        self._rd_session = None
        self._rd_started = False
        self._rd_stream_path = None
        self._sc_for_rd = None
        if had_session:
            log("RemoteDesktop session stopped")

    def _key(self, keycode, pressed):
        self._rd_session.call_sync(
            "NotifyKeyboardKeycode",
            GLib.Variant("(ub)", (keycode, pressed)),
            Gio.DBusCallFlags.NONE, -1, None)

    def send_combo(self, combo):
        """Press a chord like 'Super+Space' (modifiers first, released in reverse)."""
        self.start_remote_desktop()
        parts = [p.strip().lower() for p in combo.split("+")]
        codes = []
        for p in parts:
            if p not in self.KEYS:
                raise RuntimeError(f"unknown key '{p}' in combo '{combo}'")
            codes.append(self.KEYS[p])
        for c in codes:
            self._key(c, True)
            time.sleep(0.01)
        for c in reversed(codes):
            self._key(c, False)
            time.sleep(0.01)
        log(f"sent combo: {combo}")

    def type_text(self, text):
        """Type a literal string (lowercase letters/digits/space)."""
        self.start_remote_desktop()
        for ch in text.lower():
            key = "space" if ch == " " else ch
            if key not in self.KEYS:
                continue
            kc = self.KEYS[key]
            self._key(kc, True)
            time.sleep(0.01)
            self._key(kc, False)
            time.sleep(0.02)
        log(f"typed: {text!r}")

    # --- window management via dev.gnoblin.Shell ------------------------------
    def shell_proxy(self):
        return Gio.DBusProxy.new_sync(
            self.conn, Gio.DBusProxyFlags.NONE, None,
            "dev.gnoblin.Shell", "/dev/gnoblin/Shell", "dev.gnoblin.Shell", None)

    def dispatch(self, action, arg=""):
        """Invoke a gnoblin action (close/maximize/snap/minimize/spawn/...)."""
        self.shell_proxy().call_sync(
            "Dispatch", GLib.Variant("(ss)", (action, arg)),
            Gio.DBusCallFlags.NONE, -1, None)
        log(f"dispatch {action} {arg}".rstrip())

    def list_windows(self):
        """Return [(id, title, app_id, focused, minimized), ...]."""
        return self.shell_proxy().call_sync(
            "ListWindows", None, Gio.DBusCallFlags.NONE, -1, None).unpack()[0]

    def list_window_frames(self):
        """Return [(id, x, y, width, height), ...] for normal taskbar windows."""
        return self.shell_proxy().call_sync(
            "ListWindowFrames", None, Gio.DBusCallFlags.NONE, -1, None).unpack()[0]

    def inspect_scene(self):
        """Return the live scene as a dict: every surface's geometry + effects."""
        raw = self.shell_proxy().call_sync(
            "InspectScene", None, Gio.DBusCallFlags.NONE, -1, None).unpack()[0]
        import json as _json
        try:
            return _json.loads(raw)
        except _json.JSONDecodeError:
            # Defensive net: a non-finite float reaches us as bare inf/-nan (an
            # unallocated actor's box) which is invalid JSON. The compositor emits
            # null for these now; never hard-crash the inspector if a stray one
            # slips through — sanitise value-position inf/nan to null and retry.
            import re
            return _json.loads(re.sub(r"-?(?:inf|nan)(?=[,\]}])", "null", raw, flags=re.I))

    def workspace_state(self):
        return self.shell_proxy().call_sync(
            "WorkspaceState", None, Gio.DBusCallFlags.NONE, -1, None).unpack()

    def spawn_and_wait(self, cmd, timeout=10):
        """Spawn a Wayland app via the dispatcher and wait until it maps."""
        self.dispatch("spawn", cmd)
        for _ in range(timeout):
            time.sleep(1)
            w = self.list_windows()
            if w:
                return w
        return []

    def wait_for_output(self, timeout=15):
        """Poll grim until a wl_output exists (the virtual monitor is live)."""
        env = self._env()
        env["WAYLAND_DISPLAY"] = self.disp
        probe = self.tmp / "probe.png"
        deadline = time.time() + timeout
        while time.time() < deadline:
            if self.shell_proc.poll() is not None:
                return False
            try:
                r = subprocess.run(
                    ["grim", str(probe)],
                    env=env,
                    capture_output=True,
                    text=True,
                    timeout=2,
                )
            except subprocess.TimeoutExpired:
                time.sleep(0.5)
                continue
            if r.returncode == 0 and probe.exists() and probe.stat().st_size > 0:
                probe.unlink(missing_ok=True)
                return True
            time.sleep(0.5)
        return False

    def processes(self, needle):
        """Return process lines whose cmdline contains needle and target this display."""
        display_env = f"WAYLAND_DISPLAY={self.disp}".encode()
        found = []
        for proc in pathlib.Path("/proc").iterdir():
            if not proc.name.isdigit():
                continue
            try:
                env = (proc / "environ").read_bytes()
                if display_env not in env:
                    continue
                raw = (proc / "cmdline").read_bytes()
            except (FileNotFoundError, PermissionError, ProcessLookupError):
                continue
            cmd = raw.replace(b"\0", b" ").decode(errors="replace").strip()
            if needle in cmd:
                found.append(f"{proc.name} {cmd}")
        return found

    def _sandbox_pids(self):
        markers = [
            f"WAYLAND_DISPLAY={self.disp}".encode(),
            f"XDG_DATA_HOME={self.tmp / 'data'}".encode(),
            f"XDG_CONFIG_HOME={self.tmp / 'config'}".encode(),
            f"XDG_CACHE_HOME={self.tmp / 'cache'}".encode(),
        ]
        if self.dbus_addr:
            markers.append(f"DBUS_SESSION_BUS_ADDRESS={self.dbus_addr}".encode())
        skip = {os.getpid()}
        if self.shell_proc:
            skip.add(self.shell_proc.pid)
        if self.dbus_proc:
            skip.add(self.dbus_proc.pid)
        pids = []
        for proc in pathlib.Path("/proc").iterdir():
            if not proc.name.isdigit():
                continue
            pid = int(proc.name)
            if pid in skip:
                continue
            try:
                env = (proc / "environ").read_bytes()
            except (FileNotFoundError, PermissionError, ProcessLookupError):
                continue
            if any(marker in env for marker in markers):
                pids.append(pid)
        return pids

    def _kill_sandbox_processes(self, sig):
        for pid in self._sandbox_pids():
            try:
                os.kill(pid, sig)
            except ProcessLookupError:
                pass
            except PermissionError:
                pass

    def _wayland_client_pids(self):
        marker = f"WAYLAND_DISPLAY={self.disp}".encode()
        skip = {os.getpid()}
        if self.shell_proc:
            skip.add(self.shell_proc.pid)
        pids = []
        for proc in pathlib.Path("/proc").iterdir():
            if not proc.name.isdigit():
                continue
            pid = int(proc.name)
            if pid in skip:
                continue
            try:
                env = (proc / "environ").read_bytes()
            except (FileNotFoundError, PermissionError, ProcessLookupError):
                continue
            if marker in env:
                pids.append(pid)
        return pids

    def _terminate_pids(self, pids, timeout=1.0):
        for pid in pids:
            try:
                os.kill(pid, signal.SIGTERM)
            except (ProcessLookupError, PermissionError):
                pass
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            live = []
            for pid in pids:
                try:
                    os.kill(pid, 0)
                    live.append(pid)
                except (ProcessLookupError, PermissionError):
                    pass
            if not live:
                return
            pids = live
            time.sleep(0.05)
        for pid in pids:
            try:
                os.kill(pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass

    def _remove_tmp(self):
        for _ in range(6):
            shutil.rmtree(self.tmp, ignore_errors=True)
            if not self.tmp.exists():
                return
            time.sleep(0.1)

    # --- teardown (owns every child it spawned) -------------------------------
    def teardown(self):
        self.stop_remote_desktop()
        self._terminate_pids(self._wayland_client_pids(), timeout=1.0)
        for c in self._consumers:
            c.terminate()
        for c in self._consumers:
            try:
                c.wait(timeout=2)
            except Exception:
                c.kill()
        if self.shell_proc and self.shell_proc.poll() is None:
            self.shell_proc.terminate()
            try:
                self.shell_proc.wait(timeout=3)
            except Exception:
                self.shell_proc.kill()
        shutil.copy(self.shell_log, "/tmp/gnoblin-harness-last.log") if self.shell_log.exists() else None
        self._kill_sandbox_processes(signal.SIGTERM)
        time.sleep(0.2)
        self._kill_sandbox_processes(signal.SIGKILL)
        if self.conn and not self.conn.is_closed():
            try:
                self.conn.close_sync(None)
            except Exception:
                pass
        if self.dbus_proc and self.dbus_proc.poll() is None:
            self.dbus_proc.terminate()
            try:
                self.dbus_proc.wait(timeout=2)
            except Exception:
                self.dbus_proc.kill()
        self._remove_tmp()


def cmd_smoke():
    dk = Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(SETTLE)
        c = dk.crashed()
        if c:
            print(f"CRASH: {c}")
            print(dk._tail())
            return 1
        print("PASS: booted, settled, no crash")
        return 0
    finally:
        dk.teardown()


def cmd_shot(out):
    dk = Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(SETTLE)
        dk.shot(out)
        c = dk.crashed()
        if c:
            print(f"CRASH after shot: {c}")
            return 1
        print(f"PASS: {out}")
        return 0
    finally:
        dk.teardown()


def cmd_late(out=None):
    dk = Devkit()
    try:
        # Boot with NO monitor; clients start and wait (the devkit's flow).
        dk.boot(with_monitor=False, per_output=True)
        log("clients waiting with no output; sleeping before late monitor ...")
        time.sleep(5)
        if dk.crashed():
            print(f"CRASH before monitor: {dk.crashed()}"); print(dk._tail()); return 1
        w, h = (int(x) for x in MONITOR.split("x"))
        live = dk.add_monitor_late(w, h)
        # Check for a crash *during* materialization too (the configure-storm
        # fires the instant the output appears).
        c = dk.crashed()
        if c:
            print(f"CRASH on late monitor: {c}")
            print(dk._tail())
            return 1
        time.sleep(SETTLE)   # let clients finish configuring/rendering
        c = dk.crashed()
        if c:
            print(f"CRASH after late monitor settled: {c}")
            print(dk._tail())
            return 1
        if out and live:
            try:
                dk.shot(out)
            except Exception as e:
                log(f"(shot skipped: {e})")
        if not live:
            print("INCONCLUSIVE: monitor never materialized (no late configure-storm)")
            return 2
        print("PASS: late monitor reproduced the devkit flow, no crash")
        return 0
    finally:
        dk.teardown()


def cmd_storm():
    dk = Devkit()
    try:
        dk.boot(with_monitor=False, per_output=True)
        time.sleep(4)
        if dk.crashed():
            print(f"CRASH before monitor: {dk.crashed()}"); print(dk._tail()); return 1
        w, h = (int(x) for x in MONITOR.split("x"))
        if not dk.add_monitor_late(w, h):
            print("INCONCLUSIVE: monitor never materialized"); return 2
        time.sleep(1)
        # Alternate sizes to force repeated mode-set + reload (configure storm)
        # while the four layer clients are actively rendering.
        sizes = []
        for _ in range(8):
            sizes += [(w, h), (w - 160, h - 120), (w, h), (w + 80, h)]
        ok = dk.resize_storm(sizes, dk._sc_node)
        c = dk.crashed()
        if c:
            print(f"CRASH during storm: {c}")
            print(dk._tail())
            return 1
        if CLIENTS:
            missing = []
            for needle in ("gnoblin-topbar", "gnoblin-dock", "gnoblin-notifyd", "gnoblin-wallpaper"):
                procs = dk.processes(needle)
                if procs:
                    log(f"alive after storm: {needle} ({len(procs)})")
                else:
                    missing.append(needle)
            if missing:
                print(f"FAIL: layer-shell clients exited during storm: {', '.join(missing)}")
                print(dk._tail())
                return 1
        print("PASS: survived configure storm, no crash")
        return 0
    finally:
        dk.teardown()


def cmd_run(client_argv):
    """Boot a bare compositor, run an arbitrary layer-shell client against it,
    and report whether the compositor survived. Returns 10 on a compositor
    crash, the client's own rc otherwise. Used for protocol regression tests."""
    global CLIENTS
    CLIENTS = False
    dk = Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(2)
        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        log(f"running client: {' '.join(client_argv)}")
        r = subprocess.run(client_argv, env=env, capture_output=True,
                           text=True, timeout=30)
        if r.stdout.strip():
            print(r.stdout.rstrip())
        if r.stderr.strip():
            print(r.stderr.rstrip())
        time.sleep(1)
        c = dk.crashed()
        if c:
            print(f"COMPOSITOR CRASHED: {c}")
            print(dk._tail())
            return 10
        print("COMPOSITOR SURVIVED")
        return r.returncode
    finally:
        dk.teardown()


def cmd_keys(spec, out=None):
    """Boot with clients, inject a key chord (and optional text), screenshot.
    spec is 'Combo' or 'Combo:text to type' e.g. 'Super+Space:foot'."""
    dk = Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(SETTLE)
        if dk.crashed():
            print(f"CRASH before input: {dk.crashed()}"); print(dk._tail()); return 1
        combo, _, text = spec.partition(":")
        dk.send_combo(combo)
        time.sleep(1.5)
        if text:
            dk.type_text(text)
            time.sleep(1.0)
        c = dk.crashed()
        if c:
            print(f"CRASH after input: {c}"); print(dk._tail()); return 1
        if out:
            dk.shot(out)
        print("PASS: injected input, no crash")
        return 0
    finally:
        dk.teardown()


def cmd_click(spec, out=None):
    """Boot with clients, click at X,Y (append ':right' for right-button), shot.
    spec e.g. '640,752' or '640,752:right'."""
    dk = Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(SETTLE)
        if dk.crashed():
            print(f"CRASH before click: {dk.crashed()}"); print(dk._tail()); return 1
        coords, _, btn = spec.partition(":")
        x, y = (int(v) for v in coords.split(","))
        dk.click(x, y, button=273 if btn == "right" else 272)
        time.sleep(1.5)
        c = dk.crashed()
        if c:
            print(f"CRASH after click: {c}"); print(dk._tail()); return 1
        if out:
            dk.shot(out)
        print("PASS: clicked, no crash")
        return 0
    finally:
        dk.teardown()


def cmd_wm(spec, out=None):
    """Boot, then run a comma-separated op list against the window manager and
    print window state. Ops: 'spawn:foot', 'maximize', 'snap:left', 'minimize',
    'close', etc. e.g.  wm 'spawn:foot,maximize' final.png"""
    dk = Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(SETTLE)
        for op in spec.split(","):
            op = op.strip()
            if not op:
                continue
            action, _, arg = op.partition(":")
            if action == "spawn":
                w = dk.spawn_and_wait(arg or "foot")
                print(f"  spawn {arg}: {'mapped '+str(w) if w else 'NO WINDOW'}")
            else:
                dk.dispatch(action, arg)
                time.sleep(1.2)
                print(f"  {op} -> windows={dk.list_windows()}")
            if dk.crashed():
                print(f"CRASH after {op}: {dk.crashed()}")
                print(dk._tail())
                return 1
        if out:
            dk.shot(out)
        print(f"PASS: ran '{spec}', workspace={dk.workspace_state()}, no crash")
        return 0
    finally:
        dk.teardown()


def _contact_sheet(paths, out, cols=4, thumb_w=380):
    """Tile frame PNGs into one labelled contact sheet for at-a-glance review."""
    from PIL import Image, ImageDraw
    imgs = [(p, Image.open(p).convert("RGB")) for p in paths if os.path.exists(p)]
    if not imgs:
        return None
    thumbs = []
    for p, im in imgs:
        t = im.resize((thumb_w, max(1, int(thumb_w * im.height / im.width))))
        d = ImageDraw.Draw(t)
        label = os.path.basename(p).rsplit("-", 1)[-1].split(".")[0]
        d.rectangle([0, 0, 34, 16], fill=(0, 0, 0))
        d.text((3, 3), label, fill=(255, 255, 0))
        thumbs.append(t)
    th = thumbs[0].height
    rows = (len(thumbs) + cols - 1) // cols
    sheet = Image.new("RGB", (cols * thumb_w, rows * th), (18, 18, 22))
    for i, t in enumerate(thumbs):
        sheet.paste(t, ((i % cols) * thumb_w, (i // cols) * th))
    sheet.save(out)
    return out


def cmd_frames(setup, trigger, out=None):
    """Filmstrip capture: boot, run SETUP ops, dispatch the TRIGGER op, then burst-
    capture a sequence of frames THROUGH the resulting animation — with animations
    forced on and slowed (ANIM_SPEED, default 0.12 = ~8x) so a transition spans many
    frames. Saves frame-NN.png, a per-frame scene snapshot (the target window's
    rounding/border/geometry over time), and a tiled contact sheet. Makes animation
    bugs (maximize ring break, etc.) actually inspectable.

      frames 'spawn:nautilus' 'maximize' [OUT_PREFIX]

    Env: FRAMES (count, 14), FRAME_INTERVAL (s, 0.10), SPAWN_SETTLE (s, 8 for GTK),
         ANIM_SPEED (0.12), plus HIDPI for crisp frames."""
    import json as _json
    os.environ.setdefault("ANIM_SPEED", "0.12")
    n = int(os.environ.get("FRAMES", "14"))
    interval = float(os.environ.get("FRAME_INTERVAL", "0.10"))
    spawn_settle = float(os.environ.get("SPAWN_SETTLE", "8"))
    scratch = os.environ.get("SCRATCH", "/tmp")
    prefix = out or os.path.join(scratch, "frame")
    dk = Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(SETTLE)
        for op in (setup or "").split(","):
            op = op.strip()
            if not op:
                continue
            action, _, arg = op.partition(":")
            if action == "spawn":
                dk.spawn_and_wait(arg or "foot")
                time.sleep(spawn_settle)   # GTK/libadwaita cold start
            else:
                dk.dispatch(action, arg)
                time.sleep(1.0)
        # The window we'll track in the per-frame scene snapshot (last real window).
        wins = dk.list_windows()
        track = wins[-1][0] if wins else None
        taction, _, targ = (trigger or "").partition(":")
        if taction:
            dk.dispatch(taction, targ)
        frames, snaps = [], []
        for i in range(n):
            p = f"{prefix}-{i:02d}.png"
            try:
                dk.shot(p)
                frames.append(p)
            except Exception as e:
                print(f"frame {i}: shot failed: {e}")
            try:
                scene = dk.inspect_scene()
                w = next((s for s in scene["surfaces"] if s["id"] == track), None)
                if w:
                    snaps.append({"f": i, "frame": w["frame"], "actor_size": w["actor"]["size"],
                                  "scale": w["actor"]["scale"], "round": w["rounding"]["enabled"],
                                  "radius": w["rounding"]["radius"],
                                  "fx_on": w.get("enabled", {}).get("rounded"),
                                  "maximized": w["state"]["maximized"], "op": w["actor"]["opacity"]})
            except Exception:
                pass
            time.sleep(interval)
        if dk.crashed():
            print(f"CRASH during filmstrip: {dk.crashed()}")
            print(dk._tail())
        sheet = _contact_sheet(frames, f"{prefix}-sheet.png")
        print(f"FILMSTRIP: {len(frames)} frames, sheet -> {sheet}")
        for s in snaps:
            print(f"  f{s['f']:02d} max={s['maximized']} round-fx-ON={s['fx_on']} r={s['radius']:.0f} "
                  f"op={s['op']} actor={s['actor_size']} scale={s['scale']}")
        return 0
    finally:
        dk.teardown()


def cmd_gallery(out=None):
    """Dogfood gallery: boot once and walk a curated set of shell states (desktop,
    windowed, maximized, restored, snapped, CSD app), capturing each + its key
    effect state, into one labelled contact sheet. A whole-shell glance to spot
    regressions. Env: HIDPI for crisp tiles, SPAWN_SETTLE for GTK cold start."""
    scratch = os.environ.get("SCRATCH", "/tmp")
    prefix = out or os.path.join(scratch, "gallery")
    spawn_settle = float(os.environ.get("SPAWN_SETTLE", "8"))
    # (label, op-to-run-first-or-None). Ops use the `wm` syntax.
    steps = [
        ("01-desktop", None),
        ("02-foot", "spawn:foot"),
        ("03-foot-max", "maximize"),
        ("04-foot-restore", "maximize"),
        ("05-foot-snap-L", "snap:left"),
        ("06-nautilus-csd", "spawn:nautilus"),
        ("07-nautilus-max", "maximize"),
        ("08-nautilus-restore", "maximize"),
    ]
    dk = Devkit()
    frames = []
    try:
        dk.boot(with_monitor=True)
        time.sleep(SETTLE)
        for label, op in steps:
            if op:
                action, _, arg = op.partition(":")
                if action == "spawn":
                    dk.spawn_and_wait(arg or "foot")
                    time.sleep(spawn_settle)
                else:
                    dk.dispatch(action, arg)
                    time.sleep(2.0)   # let any size-change animation finish
            p = f"{prefix}-{label}.png"
            try:
                dk.shot(p)
                frames.append(p)
            except Exception as e:
                print(f"{label}: shot failed: {e}")
            if dk.crashed():
                print(f"CRASH at {label}: {dk.crashed()}")
                print(dk._tail())
                break
            # per-scenario effect summary of the focused/last window
            try:
                scene = dk.inspect_scene()
                apps = [s for s in scene["surfaces"] if not s["layer_ns"]]
                w = apps[-1] if apps else None
                if w:
                    en = w.get("enabled", {})
                    print(f"  {label}: '{w['title'][:24]}' max={w['state']['maximized']} "
                          f"round-on={en.get('rounded')} blur-on={en.get('blur')} "
                          f"frame={w['frame']} ring={w['border']['style']}")
            except Exception:
                pass
        sheet = _contact_sheet(frames, f"{prefix}-sheet.png", cols=4)
        print(f"GALLERY: {len(frames)} states -> {sheet}")
        return 0
    finally:
        dk.teardown()


def _run_ops(dk, spec):
    """Execute a comma-separated op spec, shared by inspect/audit/probe/annotate.
    Ops: spawn:CMD | sleep:N | click:WxH (BTN_LEFT) | rclick:WxH (BTN_RIGHT) |
    <dispatch-action>:<arg> (any gnoblin action, e.g. maximize, snap:left)."""
    for op in (spec or "").split(","):
        op = op.strip()
        if not op:
            continue
        action, _, arg = op.partition(":")
        if action == "spawn":
            dk.spawn_and_wait(arg or "foot")
        elif action == "sleep":
            # Extra settle, e.g. for a CSD/libadwaita app to finish its ~8s GTK4
            # cold start before we inspect its rounded-corner rendering.
            time.sleep(float(arg or "1"))
        elif action in ("click", "rclick"):
            # Pointer click at WxH (e.g. click:780x78) — open a menu/popup.
            cx, cy = (int(v) for v in arg.split("x"))
            dk.click(cx, cy, button=273 if action == "rclick" else 272)
            time.sleep(1.2)
        else:
            dk.dispatch(action, arg)
            time.sleep(1.2)


# ---- final-output (composited screenshot) analysis -------------------------
# The scene dump is geometry + pre-composite textures; these read the FINAL
# framebuffer (grim screenshot) the way a human eyeballs it, but mechanically.

WTYPE = {0: "normal", 1: "desktop", 2: "dock", 3: "dialog", 4: "modal-dialog",
         5: "toolbar", 6: "menu", 7: "utility", 8: "splashscreen", 9: "dropdown-menu",
         10: "popup-menu", 11: "tooltip", 12: "notification", 13: "combo", 14: "dnd",
         15: "override-other"}


def _surf_label(s):
    return (s.get("layer_ns") or s.get("title") or s.get("wm_class")
            or s.get("app_id") or f"id{s.get('id')}")


def _img_scale(scene, img):
    """Screenshot-px per stage-logical-px (sx, sy). grim captures physical px;
    the scene's stage is logical, so this is the output scale (1.0 unless HiDPI)."""
    sw, sh = scene.get("stage", [img.width, img.height])
    return (img.width / sw if sw else 1.0, img.height / sh if sh else 1.0)


def _px(img, x, y):
    """Bounds-clamped RGB pixel read."""
    x = max(0, min(img.width - 1, int(round(x))))
    y = max(0, min(img.height - 1, int(round(y))))
    return img.getpixel((x, y))[:3]


def _median_rgb(img, pts):
    import statistics
    cols = [_px(img, x, y) for (x, y) in pts]
    return tuple(int(statistics.median(c[i] for c in cols)) for i in range(3)) if cols else (0, 0, 0)


# Windows whose composited corners we audit/probe: real app surfaces + menus,
# not the full-screen shell chrome layers (topbar/dock span the whole stage).
def _is_window_surface(s):
    return WTYPE.get(s.get("type"), "") in (
        "normal", "dialog", "modal-dialog", "menu", "dropdown-menu",
        "popup-menu", "combo", "utility")


def _corners(frame):
    """4 corners as (name, corner_x, corner_y, inward_dx, inward_dy)."""
    fx, fy, fw, fh = frame
    return [("TL", fx, fy, +1, +1), ("TR", fx + fw, fy, -1, +1),
            ("BL", fx, fy + fh, +1, -1), ("BR", fx + fw, fy + fh, -1, -1)]


def audit_corner_black(scene, img):
    """The reproduced bug class: an opaque near-BLACK artifact in a rounded
    window's corner (premultiplied transparent-black composited as black, e.g.
    corner-fill sampling a popover's transparent margin). Robust: only fires when
    the corner holds near-black while NEITHER the background just outside the
    frame NOR the window's own edge is dark — i.e. black that should not be there
    and that a human would actually SEE. Dark windows/wallpapers are skipped (a
    black corner against black is invisible, not a bug)."""
    sx, sy = _img_scale(scene, img)
    out = []
    for s in scene["surfaces"]:
        if not (s["rounding"]["enabled"] and _is_window_surface(s)):
            continue
        r = max(8.0, s["rounding"]["radius"])
        rr = int(r) + 2
        for name, cx, cy, dx, dy in _corners(s["frame"]):
            bg = _median_rgb(img, [((cx - dx * o) * sx, (cy - dy * o) * sy) for o in (3, 5, 7)])
            edge = _median_rgb(img, [
                ((cx + dx * 3) * sx, (cy + dy * (r + 7)) * sy),
                ((cx + dx * (r + 7)) * sx, (cy + dy * 3) * sy)])
            black = sum(1 for ox in range(rr) for oy in range(rr)
                        if max(_px(img, (cx + dx * ox) * sx, (cy + dy * oy) * sy)) <= 20)
            if black >= 4 and max(bg) > 40 and max(edge) > 40:
                out.append({
                    "check": "corner-black", "severity": "FAIL", "surface": _surf_label(s),
                    "detail": f"{name} corner: {black} near-black px (bg={bg}, edge={edge})",
                    "box": (cx + (dx - 1) * rr / 2, cy + (dy - 1) * rr / 2, rr, rr)})
    return out


def audit_geometry(scene, img):
    """Cheap, always-correct scene sanity (no pixels): mapped+visible windows must
    have a non-degenerate frame inside a sane range, and not be invisible-but-
    occupying (opacity 0 while mapped+visible). Catches the maximize-strut /
    blank-window failure shapes."""
    out = []
    SW, SH = scene.get("stage", [0, 0])
    for s in scene["surfaces"]:
        a = s["actor"]
        lbl = _surf_label(s)
        if not (a["mapped"] and a["visible"]):
            continue
        fw, fh = s["frame"][2], s["frame"][3]
        if _is_window_surface(s) and (fw <= 0 or fh <= 0):
            out.append({"check": "zero-frame", "severity": "FAIL", "surface": lbl,
                        "detail": f"mapped+visible but frame is {fw}x{fh}"})
        if SW and (fw > SW * 4 or fh > SH * 4):
            out.append({"check": "runaway-frame", "severity": "WARN", "surface": lbl,
                        "detail": f"frame {fw}x{fh} vs stage {SW}x{SH} (strut/alloc bug?)"})
        if a["opacity"] == 0:
            out.append({"check": "invisible-mapped", "severity": "WARN", "surface": lbl,
                        "detail": "opacity 0 while mapped+visible"})
    return out


def audit_hidpi(scene, img):
    """HiDPI 2x regression guard. Under a scaled output the shaped-texture content
    size (logical) must equal buffer/scale; if content == buffer the buffer_scale
    was dropped (the layer-surface 2x bug). Only meaningful when scale != 1."""
    out = []
    sx, _ = _img_scale(scene, img)
    if abs(sx - 1.0) < 0.01:
        return out  # scale-1 capture: nothing to assert
    for s in scene["surfaces"]:
        tree = s.get("tree", {})
        content = tree.get("content")
        bw = s["buffer"][2]
        if content and bw and abs(content[0] - bw) < 2:
            out.append({"check": "hidpi-buffer-scale", "severity": "FAIL", "surface": _surf_label(s),
                        "detail": f"content {content[0]:.0f} == buffer {bw} at scale {sx:.2f} (2x bug)"})
    return out


AUDITS = [audit_geometry, audit_corner_black, audit_hidpi]


def cmd_audit(spec=None, out=None):
    """Boot, run ops, capture the FINAL composited frame, and run automated
    rendering-invariant checks against it (the bug classes hit this session:
    black popup corners, runaway frames, HiDPI 2x). Prints a PASS/FAIL report,
    writes a crop of each pixel finding next to OUT, and exits non-zero if any
    check FAILs — so it gates regressions instead of needing an eyeball."""
    import json as _json
    dk = Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(SETTLE)
        _run_ops(dk, spec)
        if dk.crashed():
            print(f"CRASH: {dk.crashed()}")
            print(dk._tail())
            return 2
        scene = dk.inspect_scene()
        shot = out or os.path.join(
            os.environ.get("XDG_RUNTIME_DIR", "/tmp"), "gnoblin-audit.png")
        dk.shot(shot)
        from PIL import Image, ImageDraw
        img = Image.open(shot).convert("RGB")
        findings = []
        for fn in AUDITS:
            try:
                findings.extend(fn(scene, img))
            except Exception as e:  # a check must never mask the others
                print(f"  [check {fn.__name__} errored: {e}]")
        fails = [f for f in findings if f["severity"] == "FAIL"]
        warns = [f for f in findings if f["severity"] != "FAIL"]
        n_surf = sum(1 for s in scene["surfaces"] if _is_window_surface(s))
        print(f"\n=== audit: {len(scene['surfaces'])} surfaces ({n_surf} windows), "
              f"{len(fails)} FAIL, {len(warns)} WARN ===")
        for f in fails + warns:
            print(f"  [{f['severity']}] {f['check']:18} {f['surface']:22} {f['detail']}")
        if not findings:
            print("  all checks passed")
        # Crop each pixel finding so a failure is inspectable, not just asserted.
        crops = [f for f in findings if "box" in f]
        if crops and out:
            sx, sy = _img_scale(scene, img)
            tiles = []
            for i, f in enumerate(crops):
                bx, by, bw, bh = f["box"]
                pad = 10
                c = img.crop((int(bx * sx) - pad, int(by * sy) - pad,
                              int((bx + bw) * sx) + pad, int((by + bh) * sy) + pad))
                c = c.resize((c.width * 6, c.height * 6), Image.NEAREST)
                d = ImageDraw.Draw(c)
                d.text((3, 3), f"{f['surface']} {f['detail'][:18]}", fill=(255, 80, 80))
                tp = f"{os.path.splitext(out)[0]}-finding{i}.png"
                c.save(tp)
                tiles.append(tp)
            sheet = f"{os.path.splitext(out)[0]}-findings.png"
            _contact_sheet(tiles, sheet, cols=min(3, len(tiles)), thumb_w=300)
            print(f"  finding crops -> {sheet}")
        return 1 if fails else 0
    finally:
        dk.teardown()


def cmd_probe(spec=None, out=None):
    """Boot, run ops, and auto-crop+zoom every app/menu window's FOUR corners into
    one labelled contact sheet — the corner inspection done by hand all session,
    mechanised. Each tile is corner@6x with the window label + corner name, so
    rounding / ring / fill / black artifacts are visible at a glance across every
    window in the scene at once."""
    out = out or os.path.join(
        os.environ.get("XDG_RUNTIME_DIR", "/tmp"), "gnoblin-probe.png")
    dk = Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(SETTLE)
        _run_ops(dk, spec)
        if dk.crashed():
            print(f"CRASH: {dk.crashed()}")
            print(dk._tail())
            return 2
        scene = dk.inspect_scene()
        shot = f"{os.path.splitext(out)[0]}-full.png"
        dk.shot(shot)
        from PIL import Image, ImageDraw
        img = Image.open(shot).convert("RGB")
        sx, sy = _img_scale(scene, img)
        tiles = []
        crop = 28  # logical px box at each corner
        for s in scene["surfaces"]:
            if not _is_window_surface(s):
                continue
            lbl = _surf_label(s)[:14]
            for name, cx, cy, dx, dy in _corners(s["frame"]):
                x0 = min(cx, cx + dx * crop) * sx
                y0 = min(cy, cy + dy * crop) * sy
                c = img.crop((int(x0), int(y0), int(x0) + int(crop * sx), int(y0) + int(crop * sy)))
                if c.width < 2 or c.height < 2:
                    continue
                c = c.resize((c.width * 6, c.height * 6), Image.NEAREST)
                d = ImageDraw.Draw(c)
                d.rectangle([0, 0, len(lbl) * 6 + 24, 13], fill=(0, 0, 0))
                d.text((2, 2), f"{lbl} {name}", fill=(120, 255, 120))
                tp = os.path.join(os.path.dirname(out) or ".",
                                  f".probe-{s['id']}-{name}.png")
                c.save(tp)
                tiles.append(tp)
        if not tiles:
            print("probe: no app/menu windows in scene")
            return 0
        _contact_sheet(tiles, out, cols=4, thumb_w=240)
        for tp in tiles:
            try:
                os.remove(tp)
            except OSError:
                pass
        print(f"probe: {len(tiles)} corners -> {out}")
        return 0
    finally:
        dk.teardown()


def cmd_annotate(spec=None, out=None):
    """Boot, run ops, and overlay each surface's geometry on the screenshot:
    buffer rect (gray), frame rect (cyan), CSD inset / rounded-content rect
    (yellow), corner-probe boxes (magenta). Makes 'where is the frame vs the
    buffer vs the shadow margin' visible instead of mentally mapping numbers."""
    out = out or os.path.join(
        os.environ.get("XDG_RUNTIME_DIR", "/tmp"), "gnoblin-annot.png")
    dk = Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(SETTLE)
        _run_ops(dk, spec)
        if dk.crashed():
            print(f"CRASH: {dk.crashed()}")
            return 2
        scene = dk.inspect_scene()
        dk.shot(out)
        from PIL import Image, ImageDraw
        img = Image.open(out).convert("RGB")
        d = ImageDraw.Draw(img)
        sx, sy = _img_scale(scene, img)

        def R(x, y, w, h, col):
            d.rectangle([x * sx, y * sy, (x + w) * sx, (y + h) * sy], outline=col)

        for s in scene["surfaces"]:
            if not _is_window_surface(s):
                continue
            bx, by, bw, bh = s["buffer"]
            fx, fy, fw, fh = s["frame"]
            R(bx, by, bw, bh, (110, 110, 110))          # buffer rect (incl. CSD shadow margin)
            R(fx, fy, fw, fh, (0, 220, 220))            # frame rect = rounded/ringed content
            r = max(0.0, s["rounding"]["radius"]) if s["rounding"]["enabled"] else 0.0
            if r:                                       # corner-probe boxes (where audit samples)
                for _n, cx, cy, dx, dy in _corners(s["frame"]):
                    R(min(cx, cx + dx * r), min(cy, cy + dy * r), r, r, (230, 0, 230))
            d.text((fx * sx + 2, fy * sy + 2), _surf_label(s)[:18], fill=(0, 255, 255))
        img.save(out)
        print(f"annotate -> {out}")
        return 0
    finally:
        dk.teardown()


def cmd_inspect(spec=None, out=None):
    """Boot, run optional spawn/dispatch ops (same syntax as `wm`), then print the
    live scene: every surface's geometry + the gnoblin effects on it. The actual
    rendering truth — what is rounded/ringed/blurred and where — instead of
    eyeballing screenshots. Optionally also writes a screenshot to OUT."""
    import json as _json
    import glob as _glob
    # Clear stale Slint inspection sidecars so we only read this run's data.
    insp_dir = os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/tmp"), "gnoblin-inspect")
    for f in _glob.glob(os.path.join(insp_dir, "*.jsonl")):
        try:
            os.remove(f)
        except OSError:
            pass
    dk = Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(SETTLE)
        _run_ops(dk, spec)
        if dk.crashed():
            print(f"CRASH: {dk.crashed()}")
            print(dk._tail())
            return 1
        scene = dk.inspect_scene()
        WT = {0: "normal", 1: "desktop", 2: "dock", 3: "dialog"}

        def rgba(c):
            return f"rgba({c[0]:.2f},{c[1]:.2f},{c[2]:.2f},{c[3]:.2f})"

        for s in scene.get("surfaces", []):
            label = s['layer_ns'] or s['title'] or s['wm_class'] or f"id{s['id']}"
            st = s['state']
            flags = "".join(f for f, on in [
                ("F", st['focused']), ("M", st['maximized']),
                ("⛶", st['fullscreen']), ("SSD" if st['ssd'] else "CSD", True)] if on)
            print(f"\n▸ {label}  [{WT.get(s['type'], s['type'])}]  {flags}")
            print(f"    frame  {s['frame']}   buffer {s['buffer']}   csd_inset {s['csd_inset']}")
            a = s['actor']
            print(f"    actor  pos{a['pos']} size{a['size']} op{a['opacity']} "
                  f"scale{a['scale']} vis={a['visible']} mapped={a['mapped']} "
                  f"z={a['z']} clip={a['clip']} kids={a['children']}")
            print(f"    win    wm_class={s['wm_class']!r} app_id={s['app_id']!r} "
                  f"pid={s['pid']} monitor={s['monitor']} stack_layer={s['stack_layer']}")
            # Slint client's self-view (theme/scale/size), correlated by pid.
            win_log = os.path.join(insp_dir, f"window-{s['pid']}.json")
            if s['pid'] and os.path.exists(win_log):
                try:
                    wj = _json.load(open(win_log))
                    print(f"    slint  theme={'dark' if wj['theme_dark'] else 'light'} "
                          f"scale={wj['scale']} logical={wj['logical']} physical={wj['physical']} "
                          f"full_height={wj['full_height']} input_h={wj['input_height']}")
                except (OSError, ValueError, KeyError):
                    pass
            r = s['rounding']
            extra = ""
            if 'corner_fill' in r:
                extra = (f" fill={r['corner_fill']} adaptive={r['adaptive']}"
                         f"({r['adapt_shade']:.2f}/{r['adapt_light']:.2f}) focused={r['focused']}")
            if 'fbo' in r:
                extra += f" fbo={r['fbo']}"
            print(f"    round  enabled={r['enabled']} radius={r['radius']} "
                  f"algo={r['algorithm']} smoothing={r['smoothing']} inset={r['applied_inset']}{extra}")
            b = s['border']
            print(f"    border style={b['style']} bw={b['border_width']} rw={b['ring_width']}")
            print(f"           outer={rgba(b['border_color'])} / {rgba(b['border_color_focused'])}(F)")
            print(f"           inner={rgba(b['ring_color'])} / {rgba(b['ring_color_focused'])}(F)")
            bl = s['blur']
            sh_fx = s['shadow']
            print(f"    blur   enabled={bl['enabled']} radius={bl['radius']} "
                  f"alpha_threshold={bl['alpha_threshold']}")
            print(f"    shadow enabled={sh_fx['enabled']}")
            for ly in sh_fx.get('layers', []):
                print(f"           layer offset{ly['offset']} blur={ly['blur']} "
                      f"spread={ly['spread']} {rgba(ly['color'])}")
            att = "+".join(k for k, v in s['attached'].items() if v) or "none"
            en = s.get('enabled', {})
            onoff = "+".join(k for k, v in en.items() if v) or "none"
            print(f"    attached effects: {att}   enabled: {onoff}")
            if s.get('shadow_actor'):
                sh = s['shadow_actor']
                print(f"    shadow_actor pos{sh['pos']} size{sh['size']} "
                      f"op{sh['opacity']} mapped={sh['mapped']}")

            def tree(node, ind):
                fx = ("  fx=" + "+".join(node['fx'])) if node['fx'] else ""
                nm = f" {node['name']!r}" if node['name'] else ""
                print(f"    {'  ' * ind}└ {node['gtype']}{nm} "
                      f"pos{node['pos']} size{node['size']} op{node['opacity']}"
                      f"{'' if node['mapped'] else ' (unmapped)'}{fx}")
                for c in node['children']:
                    tree(c, ind + 1)
            print("    object tree:")
            tree(s['tree'], 0)
            # Slint-side icon resolutions logged by this surface's client (by pid).
            icon_log = os.path.join(insp_dir, f"icons-{s['pid']}.jsonl")
            if s['pid'] and os.path.exists(icon_log):
                with open(icon_log) as f:
                    icons = [_json.loads(ln) for ln in f if ln.strip()]
                if icons:
                    print(f"    icons ({len(icons)} resolved by this client):")
                    for ic in icons[:24]:
                        mark = ic['dims'] if ic['resolved'] else "MISSING"
                        sz = f"@{ic['req_size']}" if ic['req_size'] else ""
                        print(f"        {ic['name']}{sz} -> {mark}")
            # Slint element tree (per-element geometry/role/type), by pid.
            el_log = os.path.join(insp_dir, f"elements-{s['pid']}.json")
            if s['pid'] and os.path.exists(el_log):
                try:
                    els = _json.load(open(el_log))
                except (OSError, ValueError):
                    els = []
                if els:
                    print(f"    slint element tree ({len(els)} elements):")
                    for el in els[:44]:
                        g = el['geom']
                        ty = el['type'] or el['role']
                        extra = ""
                        if 'radius' in el:
                            extra += f"  r={el['radius']:.0f} bw={el['border_w']:.0f}"
                        if 'bg' in el and el['bg'][3]:  # non-transparent background
                            bg = el['bg']
                            extra += f"  bg=#{bg[0]:02x}{bg[1]:02x}{bg[2]:02x}{bg[3]:02x}"
                        if el.get('border_col', [0, 0, 0, 0])[3]:
                            bc = el['border_col']
                            extra += f"  border=#{bc[0]:02x}{bc[1]:02x}{bc[2]:02x}{bc[3]:02x}"
                        if 'text' in el:
                            c = el['color']
                            txt = el['text'][:24]
                            extra += (f"  text={txt!r} fs={el['font_size']:.0f} "
                                      f"fw={el['font_weight']} #{c[0]:02x}{c[1]:02x}{c[2]:02x}{c[3]:02x}")
                        print(f"        {'  ' * el['depth']}{ty} "
                              f"[{g[0]:.0f},{g[1]:.0f} {g[2]:.0f}x{g[3]:.0f}]{extra}")
                    if len(els) > 44:
                        print(f"        … +{len(els) - 44} more")
        print("\nRAW:", _json.dumps(scene))
        if out:
            dk.shot(out)
        return 0
    finally:
        dk.teardown()


def cmd_boot():
    dk = Devkit()
    stop = {"go": True}
    signal.signal(signal.SIGINT, lambda *_: stop.update(go=False))
    try:
        dk.boot(with_monitor=True)
        print(f"WAYLAND_DISPLAY={dk.disp}  bus={dk.dbus_addr}")
        print("Ctrl-C to stop.")
        while stop["go"] and dk.shell_proc.poll() is None:
            time.sleep(0.3)
        c = dk.crashed()
        print(f"CRASH: {c}" if c else "stopped cleanly")
        return 1 if c else 0
    finally:
        dk.teardown()


def main():
    if not SHELL_BIN.exists():
        sys.exit(f"no gnoblin-shell at {SHELL_BIN} — run `just dev`")
    cmd = sys.argv[1] if len(sys.argv) > 1 else "smoke"
    arg = sys.argv[2] if len(sys.argv) > 2 else None
    if cmd == "smoke":
        sys.exit(cmd_smoke())
    elif cmd == "shot":
        sys.exit(cmd_shot(arg or "/tmp/gnoblin-harness.png"))
    elif cmd == "late":
        sys.exit(cmd_late(arg))
    elif cmd == "storm":
        sys.exit(cmd_storm())
    elif cmd == "run":
        if not arg:
            sys.exit(f"usage: {sys.argv[0]} run CLIENT [args...]")
        sys.exit(cmd_run(sys.argv[2:]))
    elif cmd == "keys":
        if not arg:
            sys.exit(f"usage: {sys.argv[0]} keys 'Super+Space[:text]' [OUT.png]")
        sys.exit(cmd_keys(arg, sys.argv[3] if len(sys.argv) > 3 else None))
    elif cmd == "click":
        if not arg:
            sys.exit(f"usage: {sys.argv[0]} click 'X,Y[:right]' [OUT.png]")
        sys.exit(cmd_click(arg, sys.argv[3] if len(sys.argv) > 3 else None))
    elif cmd == "wm":
        if not arg:
            sys.exit(f"usage: {sys.argv[0]} wm 'spawn:foot,maximize,snap:left' [OUT.png]")
        sys.exit(cmd_wm(arg, sys.argv[3] if len(sys.argv) > 3 else None))
    elif cmd == "inspect":
        sys.exit(cmd_inspect(arg, sys.argv[3] if len(sys.argv) > 3 else None))
    elif cmd == "audit":
        sys.exit(cmd_audit(arg, sys.argv[3] if len(sys.argv) > 3 else None))
    elif cmd == "probe":
        sys.exit(cmd_probe(arg, sys.argv[3] if len(sys.argv) > 3 else None))
    elif cmd == "annotate":
        sys.exit(cmd_annotate(arg, sys.argv[3] if len(sys.argv) > 3 else None))
    elif cmd == "frames":
        if not arg:
            sys.exit(f"usage: {sys.argv[0]} frames 'SETUP_OPS' 'TRIGGER' [OUT_PREFIX]")
        sys.exit(cmd_frames(arg, sys.argv[3] if len(sys.argv) > 3 else "",
                            sys.argv[4] if len(sys.argv) > 4 else None))
    elif cmd == "gallery":
        sys.exit(cmd_gallery(arg))
    elif cmd == "boot":
        sys.exit(cmd_boot())
    else:
        sys.exit(f"usage: {sys.argv[0]} [smoke|shot OUT|late [OUT]|wm OPS|"
                 f"inspect [OPS]|audit [OPS] [OUT]|probe [OPS] [OUT]|"
                 f"annotate [OPS] [OUT]|boot]")


if __name__ == "__main__":
    main()
