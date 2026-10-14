#!/usr/bin/env python3
"""gnoblin-vr --devices on fixture machines: which headsets are offered and for which backend."""

import configparser
import os
import subprocess
import sys
import tempfile

binary = sys.argv[1]


def machine(usb=(), drm=(), streamer=None):
    root = tempfile.mkdtemp(prefix="gnoblin-vr-fixture-")
    for index, (vendor, product, name, serial) in enumerate(usb):
        directory = os.path.join(root, "usb", f"1-{index + 1}")
        os.makedirs(directory)
        for file, value in (("idVendor", vendor), ("idProduct", product), ("product", name), ("serial", serial)):
            with open(os.path.join(directory, file), "w") as handle:
                handle.write(value + "\n")
        os.makedirs(os.path.join(root, "usb", f"1-{index + 1}:1.0"))
    with open(os.path.join(root, "drm"), "w") as handle:
        handle.write("".join(f"{line}\n" for line in drm))
    if streamer:
        with open(os.path.join(root, "streamer"), "w") as handle:
            handle.write("[streamer]\n" + "".join(f"{k}={v}\n" for k, v in streamer.items()))
    return root


def devices(root):
    environment = dict(os.environ, GNOBLIN_VR_FIXTURE=root)
    result = subprocess.run([binary, "--devices"], env=environment, capture_output=True, text=True, timeout=10)
    assert result.returncode == 0, result.stderr
    parser = configparser.ConfigParser()
    parser.optionxform = str
    parser.read_string(result.stdout)
    count = int(parser["streamer"]["device_count"])
    return [dict(parser[f"device.{i}"]) for i in range(count)]


# Nothing attached.
assert devices(machine()) == []

# A Steam Deck shares Valve's USB vendor ID and is not a headset.
assert devices(machine(usb=[("28de", "1205", "Steam Deck Controller", "A1")])) == []

# A Quest on USB with no streamer running: detected, with the next step spelled out.
quest = devices(machine(usb=[("2833", "0186", "Quest 3", "1WMHH")]))
assert len(quest) == 1, quest
assert quest[0]["kind"] == "standalone" and quest[0]["backend"] == "wivrn", quest
assert quest[0]["transport"] == "usb" and quest[0]["state"] == "detected", quest
assert quest[0]["serial"] == "1WMHH" and "wivrn" in quest[0]["diagnostic"], quest

# The streamer reports a headset: the USB entry becomes the connected device.
quest = devices(
    machine(
        usb=[("2833", "0186", "Quest 3", "1WMHH")],
        streamer={"available": "true", "headset_connected": "true", "system_name": "Quest 3"},
    )
)
assert [d["state"] for d in quest] == ["connected"], quest
assert [d["transport"] for d in quest] == ["usb"], quest

# A headset reaching the streamer with no USB cable arrives over Wi-Fi.
wifi = devices(machine(streamer={"available": "true", "headset_connected": "true", "system_name": "Pico 4"}))
assert len(wifi) == 1 and wifi[0]["transport"] == "wifi" and wifi[0]["name"] == "Pico 4", wifi
assert wifi[0]["backend"] == "wivrn" and wifi[0]["state"] == "connected", wifi

# A tethered headset is its display connector, even though its USB parts are also present.
index = devices(
    machine(
        usb=[("28de", "2300", "Valve Index HMD", "LHR-1"), ("28de", "2301", "Valve Index Controller", "LHR-2")],
        drm=["DP-2|VLV|Index"],
    )
)
assert len(index) == 1, index
assert index[0]["id"] == "drm:DP-2" and index[0]["transport"] == "drm-lease", index
assert index[0]["kind"] == "tethered" and index[0]["backend"] == "monado", index
assert index[0]["state"] == "ready" and index[0]["name"] == "Valve Index", index

# USB without a display connector: the cable is not carrying video.
waiting = devices(machine(usb=[("28de", "2300", "Valve Index HMD", "LHR-1")]))
assert len(waiting) == 1 and waiting[0]["state"] == "waiting", waiting
assert "display" in waiting[0]["diagnostic"], waiting

# A tethered and a standalone headset together are listed in a stable order.
both = devices(machine(usb=[("2833", "0186", "Quest 3", "1WMHH")], drm=["HDMI-A-1|HVR|VIVE"]))
assert [d["id"] for d in both] == ["drm:HDMI-A-1", "usb:2833:0186:1WMHH"], both
print("ok")
