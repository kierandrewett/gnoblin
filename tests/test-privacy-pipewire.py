#!/usr/bin/env python3
"""Check live PipeWire privacy activity through a fresh Gnoblin devkit."""

import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = Path(__file__).resolve()
PREFIX = Path(os.environ.get("GNOBLIN_TEST_PREFIX", ROOT / "install"))
GNOBLINCTL = PREFIX / "bin/gnoblinctl"
SINK_NAME = "gnoblin_privacy_test_sink"
MIC_NAME = "gnoblin_privacy_test_mic"


def require_tools(names):
    missing = [name for name in names if not shutil.which(name)]
    if missing:
        raise RuntimeError("Missing test tools: " + ", ".join(missing))


def checked(command, env=None, timeout=15):
    result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=timeout)
    if result.returncode:
        raise RuntimeError(
            f"Command failed ({result.returncode}): {shlex.join(map(str, command))}\n{result.stdout}{result.stderr}"
        )
    return result.stdout.strip()


def host_wayland_socket():
    display = os.environ.get("WAYLAND_DISPLAY")
    if not display:
        raise RuntimeError("Start this test from a Wayland desktop")
    if os.path.isabs(display):
        socket = Path(display)
    else:
        host_runtime = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
        socket = host_runtime / display
    if not socket.exists():
        raise RuntimeError(f"Host Wayland socket does not exist: {socket}")
    return str(socket)


def start_service(command, log_path, env):
    log = open(log_path, "wb")
    try:
        process = subprocess.Popen(
            command,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    except OSError:
        log.close()
        raise
    return process, log


def stop_process(process):
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=3)
    except (ProcessLookupError, subprocess.TimeoutExpired):
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait(timeout=3)


def start_private_pipewire(work, runtime, env):
    home = work / "home"
    config = work / "config"
    data = work / "data"
    state = work / "state"
    cache = work / "cache"
    for path in (home, config, data, state, cache):
        path.mkdir(mode=0o700)

    env.update(
        {
            "HOME": str(home),
            "XDG_RUNTIME_DIR": str(runtime),
            "XDG_CONFIG_HOME": str(config),
            "XDG_DATA_HOME": str(data),
            "XDG_STATE_HOME": str(state),
            "XDG_CACHE_HOME": str(cache),
            "PULSE_SERVER": f"unix:{runtime / 'pulse/native'}",
        }
    )
    env.pop("PIPEWIRE_REMOTE", None)

    wp_config = config / "wireplumber/wireplumber.conf.d/90-gnoblin-privacy-test.conf"
    wp_config.parent.mkdir(parents=True)
    wp_config.write_text(
        """wireplumber.profiles = {
  main = {
    support.dbus = disabled
    monitor.alsa = disabled
    monitor.alsa-midi = disabled
    monitor.bluez = disabled
    monitor.libcamera = disabled
    monitor.v4l2 = disabled
  }
}
""",
        encoding="utf-8",
    )
    pw_config = config / "pipewire/pipewire.conf.d/90-gnoblin-privacy-test.conf"
    pw_config.parent.mkdir(parents=True)
    pw_config.write_text(
        """context.objects = [
  {
    factory = adapter
    args = {
      factory.name = support.null-audio-sink
      node.name = gnoblin_privacy_test_mic
      node.description = Gnoblin-Privacy-Test-Microphone
      media.class = Audio/Source/Virtual
      audio.position = [ FL FR ]
      monitor.passthrough = true
    }
  }
]
""",
        encoding="utf-8",
    )

    services = []
    modules = []
    try:
        for name, command in (
            ("pipewire", ["pipewire"]),
            ("wireplumber", ["wireplumber"]),
            ("pipewire-pulse", ["pipewire-pulse"]),
        ):
            process, log = start_service(command, work / f"{name}.log", env)
            services.append((process, log))
            if name == "pipewire":
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline and not (runtime / "pipewire-0").exists():
                    if process.poll() is not None:
                        break
                    time.sleep(0.1)
                if not (runtime / "pipewire-0").exists():
                    detail = (work / "pipewire.log").read_text(encoding="utf-8", errors="replace")
                    raise RuntimeError(f"Private PipeWire did not create its socket:\n{detail}")

        deadline = time.monotonic() + 15
        result = None
        while time.monotonic() < deadline:
            if any(process.poll() is not None for process, _ in services):
                break
            result = subprocess.run(["pactl", "info"], env=env, capture_output=True, text=True, timeout=2)
            if result.returncode == 0:
                break
            time.sleep(0.1)

        if result is None or result.returncode:
            raise RuntimeError(
                "Private PipeWire PulseAudio socket did not become ready:\n"
                + "\n".join(
                    path.read_text(encoding="utf-8", errors="replace")
                    for path in (work / "pipewire.log", work / "wireplumber.log", work / "pipewire-pulse.log")
                )
            )

        nodes = checked(["pw-cli", "ls", "Node"], env=env)
        if f'node.name = "{MIC_NAME}"' not in nodes:
            raise RuntimeError(f"Private PipeWire did not create its virtual microphone node:\n{nodes}")

        sink_module = checked(
            [
                "pactl",
                "load-module",
                "module-null-sink",
                f"sink_name={SINK_NAME}",
                "sink_properties=device.description=Gnoblin-Privacy-Test-Sink",
            ],
            env=env,
        )
        modules.append(sink_module)
        return env, runtime, services, modules
    except Exception:
        for module in reversed(modules):
            subprocess.run(["pactl", "unload-module", module], env=env, capture_output=True)
        for process, log in reversed(services):
            stop_process(process)
            log.close()
        raise


def lua_privacy_state(runtime):
    lua_file = runtime / "gnoblin-privacy-state.lua"
    lua_file.write_text(
        """local state = gnoblin.privacy.state()
print("GNOBLIN_PRIVACY " ..
  tostring(state.available.microphone_in_use) .. " " ..
  tostring(state.available.camera_in_use) .. " " ..
  tostring(state.microphone_in_use) .. " " ..
  tostring(state.camera_in_use))
""",
        encoding="utf-8",
    )
    output = checked([str(GNOBLINCTL), "lua", str(lua_file)], timeout=10)
    for line in output.splitlines():
        if line.startswith("GNOBLIN_PRIVACY "):
            fields = line.split()[1:]
            if len(fields) == 4:
                return tuple(fields)
    raise RuntimeError(f"gnoblinctl lua did not return a privacy snapshot: {output!r}")


def wait_for_state(runtime, predicate, description, timeout=12, processes=()):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        for name, process, log_path in processes:
            if process.poll() is not None:
                detail = Path(log_path).read_text(encoding="utf-8", errors="replace")
                raise RuntimeError(f"{name} exited with {process.returncode}: {detail}")
        last = lua_privacy_state(runtime)
        if predicate(last):
            return last
        time.sleep(0.1)
    details = "\n".join(
        f"--- {name} ---\n{Path(log_path).read_text(encoding='utf-8', errors='replace')}"
        for name, _, log_path in processes
    )
    raise RuntimeError(f"Timed out waiting for {description}; last privacy state was {last}\n{details}")


def run_inside():
    if not os.environ.get("GNOBLIN_COMPOSITOR_SOCKET"):
        raise RuntimeError("The inner check must run inside the supervised devkit")
    runtime = Path(os.environ["XDG_RUNTIME_DIR"])
    pipewire_socket = runtime / "pipewire-0"
    expected_socket = os.environ.get("GNOBLIN_TEST_PIPEWIRE_SOCKET")
    if not pipewire_socket.is_symlink() or not expected_socket:
        raise RuntimeError("Devkit is not connected to the test's isolated PipeWire socket")
    if pipewire_socket.resolve() != Path(expected_socket):
        raise RuntimeError("Devkit PipeWire socket does not point at the isolated test server")

    wait_for_state(
        runtime,
        lambda state: state[0:2] == ("true", "true") and state[2:4] == ("false", "false"),
        "an available, idle PipeWire privacy snapshot",
        timeout=20,
    )

    mic_log = open(runtime / "privacy-microphone.log", "wb")
    mic = subprocess.Popen(
        [
            "pw-cat",
            "--record",
            "--target",
            MIC_NAME,
            "--rate",
            "48000",
            "--channels",
            "2",
            "--format",
            "s16",
            "--raw",
            "-",
        ],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=mic_log,
        start_new_session=True,
    )
    camera_log = None
    camera = None
    try:
        wait_for_state(
            runtime,
            lambda state: state[0:2] == ("true", "true") and state[2:4] == ("true", "false"),
            "microphone activity",
            processes=(("microphone stream", mic, runtime / "privacy-microphone.log"),),
        )

        camera_log = open(runtime / "privacy-camera.log", "wb")
        camera_source = open("/dev/zero", "rb")
        # A running PipeWire stream marked with the Camera media role exercises
        # the compositor's camera-node classification without physical hardware.
        camera = subprocess.Popen(
            [
                "pw-cat",
                "--playback",
                "--target",
                SINK_NAME,
                "--media-role",
                "Camera",
                "--raw",
                "-",
            ],
            stdin=camera_source,
            stdout=subprocess.DEVNULL,
            stderr=camera_log,
            start_new_session=True,
        )
        camera_source.close()
        wait_for_state(
            runtime,
            lambda state: state[0:2] == ("true", "true") and state[2:4] == ("true", "true"),
            "microphone and camera activity",
            processes=(
                ("microphone stream", mic, runtime / "privacy-microphone.log"),
                ("camera stream", camera, runtime / "privacy-camera.log"),
            ),
        )

        stop_process(mic)
        wait_for_state(
            runtime,
            lambda state: state[0:2] == ("true", "true") and state[2:4] == ("false", "true"),
            "microphone stopping while camera stays active",
            processes=(("camera stream", camera, runtime / "privacy-camera.log"),),
        )

        stop_process(camera)
        wait_for_state(
            runtime,
            lambda state: state[0:2] == ("true", "true") and state[2:4] == ("false", "false"),
            "camera stopping after its 500 ms grace interval",
            timeout=8,
        )
        print("PASS: Lua privacy state follows isolated PipeWire microphone and camera activity")
    finally:
        stop_process(camera) if camera else None
        stop_process(mic)
        mic_log.close()
        if camera_log:
            camera_log.close()


def run_outer():
    require_tools(["pipewire", "wireplumber", "pipewire-pulse", "pactl", "pw-cat", "pw-cli"])
    wayland_socket = host_wayland_socket()
    build_tmp = ROOT / "build/tmp"
    build_tmp.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="gnoblin-privacy-pipewire-", dir=build_tmp))
    env = os.environ.copy()
    host_runtime = Path(env.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
    runtime = Path(tempfile.mkdtemp(prefix="pw-", dir=host_runtime))
    services = []
    logs = []
    modules = ()
    passed = False
    try:
        env, runtime, services, modules = start_private_pipewire(work, runtime, env)
        logs = [log for _, log in services]
        env["WAYLAND_DISPLAY"] = wayland_socket
        env["GNOBLIN_PREFIX"] = str(PREFIX)
        env["GNOBLIN_RUNTIME_BIN"] = str(PREFIX / "bin/gnoblin")
        env["GNOBLINCTL"] = str(GNOBLINCTL)
        env["GNOBLIN_TEST_PIPEWIRE_SOCKET"] = str(runtime / "pipewire-0")
        env["GNOBLIN_DEVKIT_EXEC"] = shlex.join([sys.executable, str(SCRIPT), "--inside"])
        devkit = subprocess.Popen(
            [str(ROOT / "scripts/run-gnoblin-devkit.sh")],
            cwd=ROOT,
            env=env,
            start_new_session=True,
        )
        try:
            result = devkit.wait(timeout=180)
        except subprocess.TimeoutExpired as error:
            stop_process(devkit)
            raise RuntimeError("Nested PipeWire privacy test exceeded 180 seconds") from error
        if result:
            print("Private PipeWire service logs:", file=sys.stderr)
            for path in (work / "pipewire.log", work / "wireplumber.log", work / "pipewire-pulse.log"):
                print(f"--- {path.name} ---", file=sys.stderr)
                print(path.read_text(encoding="utf-8", errors="replace"), file=sys.stderr)
        else:
            passed = True
        return result
    finally:
        if modules:
            for module in reversed(modules):
                subprocess.run(["pactl", "unload-module", module], env=env, capture_output=True)
        for process, _ in reversed(services):
            stop_process(process)
        for log in logs:
            log.close()
        shutil.rmtree(runtime, ignore_errors=True)
        if passed:
            shutil.rmtree(work, ignore_errors=True)
        else:
            print(f"PipeWire test artifacts kept at {work}", file=sys.stderr)


def main():
    if len(sys.argv) == 2 and sys.argv[1] == "--inside":
        run_inside()
        return 0
    if len(sys.argv) != 1:
        print(f"usage: {sys.argv[0]} [--inside]", file=sys.stderr)
        return 2
    return run_outer()


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"privacy-pipewire-test: {error}", file=sys.stderr)
        raise SystemExit(1)
