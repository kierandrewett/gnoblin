#!/usr/bin/env python3
# Regression test for the wallpaper client (gnoblin-wallpaper).
#
# gnoblin-wallpaper is a background layer-shell client that draws an image over
# the compositor's fallback colour. This test boots a bare compositor with no
# wallpaper configured, then starts the wallpaper client manually with its own
# GNOBLIN_CONFIG. That keeps the compositor fallback from masking a broken
# wallpaper layer and lets us prove live wallpaper reloads.
import sys, time, importlib.util, pathlib, subprocess

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

try:
    from PIL import Image
except Exception:
    print("SKIP: no PIL")
    sys.exit(0)

WALL = "/tmp/gnoblin-wallpaper-test-src.png"
WALL_RELOAD = "/tmp/gnoblin-wallpaper-test-reload.png"
WALL_SCALED = "/tmp/gnoblin-wallpaper-test-scaled.png"
WALL_MISSING = "/tmp/gnoblin-wallpaper-test-missing.png"
SHOT = "/tmp/gnoblin-wallpaper-test.png"
SHOT_RELOAD = "/tmp/gnoblin-wallpaper-reload-test.png"
SHOT_MISSING = "/tmp/gnoblin-wallpaper-missing-test.png"
SHOT_SCALED = "/tmp/gnoblin-wallpaper-scaled-test.png"
SHOT_WINDOW = "/tmp/gnoblin-wallpaper-window-test.png"
COLOR = (220, 30, 170)   # distinctive magenta, unlike any default chrome
RELOAD_COLOR = (28, 176, 220)   # distinctive cyan, unlike the initial image
MISSING_COLOR = (18, 52, 86)   # #123456, used when the image path is missing
SCALED_COLOR = (238, 196, 20)   # distinctive yellow for aspect-ratio checks


def close_enough(actual, expected):
    return all(abs(a - b) <= 8 for a, b in zip(actual, expected))


def is_initial_wallpaper_pixel(c):
    return c[0] > 150 and c[2] > 100 and c[1] < 90


def maximized_window_edges(path, x=300):
    im = Image.open(path).convert("RGB")
    px = im.load()
    _, height = im.size

    top = None
    for y in range(0, height // 2):
        r, g, b = px[x, y]
        if r > 80 and g > 80 and b > 80:
            top = y
            break

    bottom = None
    for y in range(height - 1, height // 2, -1):
        r, g, b = px[x, y]
        if abs(b - r) < 9 and abs(g - r) < 9 and r > 20:
            bottom = y
            break

    return top, bottom, height


def write_cfg(path, wall, style="stretched", background="#000000"):
    path.write_text(
        "[appearance]\n"
        f"wallpaper = {wall}\n"
        f"wallpaper-style = {style}\n"
        f'background = "{background}"\n'
    )


def main():
    Image.new("RGB", (64, 64), COLOR).save(WALL)
    Image.new("RGB", (64, 64), RELOAD_COLOR).save(WALL_RELOAD)
    Image.new("RGB", (64, 32), SCALED_COLOR).save(WALL_SCALED)
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    proc = None
    log = None
    try:
        dk.extra_appearance = (
            f"wallpaper = {WALL}\n"
            "wallpaper-style = stretched\n"
            'background = "#000000"\n'
        )
        dk.boot(with_monitor=True)

        # The compositor itself must only draw the solid fallback colour. The
        # configured image belongs to the background layer-shell client; drawing
        # it in the plugin masks a broken or missing background layer.
        dk.shot(SHOT)
        pre_px = Image.open(SHOT).convert("RGB").load()[640, 400]
        print(f"  before wallpaper client pixel = {pre_px}")
        if is_initial_wallpaper_pixel(pre_px):
            print("FAIL: compositor plugin drew the wallpaper without a background layer client")
            return 1

        cfg = dk.tmp / "config" / "gnoblin" / "gnoblin.conf"
        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["GNOBLIN_CONFIG"] = str(cfg)
        log = open(dk.tmp / "wallpaper.log", "wb")
        proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-wallpaper")],
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
        )
        time.sleep(2.5)
        if proc.poll() is not None:
            print(f"FAIL: wallpaper client exited early rc={proc.returncode}")
            return 1
        dk.shot(SHOT)
        px = Image.open(SHOT).convert("RGB").load()
        c = px[640, 400]   # desktop centre, clear of panels
        print(f"  desktop background pixel = {c} (wallpaper is {COLOR})")
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1
        if not is_initial_wallpaper_pixel(c):
            print("FAIL: wallpaper image did not render (background is not the image)")
            return 1

        # The default config says appearance changes are live. Because the
        # wallpaper is now the real visible background layer, this client must
        # reload its own config instead of relying on compositor fallback paint.
        time.sleep(1.1)  # avoid coarse mtime granularity hiding the edit
        write_cfg(cfg, WALL_RELOAD)
        reloaded = None
        deadline = time.time() + 6
        while time.time() < deadline:
            time.sleep(0.5)
            dk.shot(SHOT_RELOAD)
            reloaded = Image.open(SHOT_RELOAD).convert("RGB").load()[640, 400]
            if close_enough(reloaded, RELOAD_COLOR):
                break
        print(f"  after config reload pixel = {reloaded} (wallpaper is {RELOAD_COLOR})")
        if not close_enough(reloaded, RELOAD_COLOR):
            print("FAIL: wallpaper did not reload after GNOBLIN_CONFIG changed")
            return 1

        # If the configured image disappears or cannot be decoded, the wallpaper
        # client is still the desktop background layer and must paint the
        # configured solid background. Falling back to the compositor plugin here
        # would make the real background layer invisible/broken.
        time.sleep(1.1)
        write_cfg(cfg, WALL_MISSING, background="#123456")
        missing = None
        deadline = time.time() + 6
        while time.time() < deadline:
            time.sleep(0.5)
            dk.shot(SHOT_MISSING)
            missing = Image.open(SHOT_MISSING).convert("RGB").load()[640, 400]
            if close_enough(missing, MISSING_COLOR):
                break
        print(f"  missing wallpaper fallback pixel = {missing} (background is {MISSING_COLOR})")
        if not close_enough(missing, MISSING_COLOR):
            print("FAIL: wallpaper layer did not paint configured background after image load failed")
            return 1

        # `scaled` must preserve aspect ratio and letterbox over the configured
        # background. A 2:1 image on a 1280x800 output should leave a black band
        # above and below the scaled 1280x640 image. If this accidentally behaves
        # like `stretched`, the top sample becomes the wallpaper colour instead.
        time.sleep(1.1)
        write_cfg(cfg, WALL_SCALED, "scaled")
        scaled_top = None
        scaled_mid = None
        deadline = time.time() + 6
        while time.time() < deadline:
            time.sleep(0.5)
            dk.shot(SHOT_SCALED)
            scaled_px = Image.open(SHOT_SCALED).convert("RGB").load()
            scaled_top = scaled_px[640, 50]
            scaled_mid = scaled_px[640, 400]
            if close_enough(scaled_top, (0, 0, 0)) and close_enough(scaled_mid, SCALED_COLOR):
                break
        print(f"  scaled style top pixel = {scaled_top}; centre pixel = {scaled_mid}")
        if not close_enough(scaled_top, (0, 0, 0)):
            print("FAIL: scaled wallpaper did not preserve aspect ratio/letterbox")
            return 1
        if not close_enough(scaled_mid, SCALED_COLOR):
            print("FAIL: scaled wallpaper centre did not render the image")
            return 1

        if not dk.spawn_and_wait("foot"):
            print("FAIL: foot did not map above the wallpaper")
            return 1
        dk.dispatch("maximize")
        time.sleep(1)
        dk.shot(SHOT_WINDOW)
        app_px = Image.open(SHOT_WINDOW).convert("RGB").load()[640, 400]
        print(f"  maximized app centre pixel = {app_px}")
        if close_enough(app_px, RELOAD_COLOR) or is_initial_wallpaper_pixel(app_px):
            print("FAIL: wallpaper is stacked above a normal window")
            return 1
        top, bottom, height = maximized_window_edges(SHOT_WINDOW)
        print(f"  maximized app edges with only wallpaper: top={top} bottom={bottom} screen_h={height}")
        if top is None or bottom is None:
            print("FAIL: could not locate the maximized window edges")
            return 1
        if top > 6:
            print(f"FAIL: background layer reserved top work area (top edge y={top})")
            return 1
        if bottom < height - 7:
            print(f"FAIL: background layer reserved bottom work area (bottom edge y={bottom})")
            return 1

        print("PASS: gnoblin-wallpaper rendered the configured image")
        return 0
    finally:
        if proc and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=2)
            except Exception:
                proc.kill()
        if log:
            log.close()
        dk.teardown()
        dh.CLIENTS = old_clients
        try:
            pathlib.Path(WALL).unlink()
        except FileNotFoundError:
            pass
        try:
            pathlib.Path(WALL_RELOAD).unlink()
        except FileNotFoundError:
            pass
        try:
            pathlib.Path(WALL_SCALED).unlink()
        except FileNotFoundError:
            pass


if __name__ == "__main__":
    sys.exit(main())
