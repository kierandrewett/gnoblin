#define _GNU_SOURCE
#include "gnoblin-vr-session.h"
#include "gnoblin-vr-devices.h"
#include "gnoblin-vr-status.h"

#include <errno.h>
#include <gio/gio.h>
#include <glib-unix.h>
#include <string.h>
#include <unistd.h>

#ifdef HAVE_LIBUDEV
#include <libudev.h>
#endif

static void free_streamer(GnoblinVrStreamer* streamer) {
    g_clear_pointer(&streamer->system_name, g_free);
}

int gnoblin_vr_devices_main(void) {
    GnoblinVrStreamer streamer;
    g_autoptr(GPtrArray) devices = gnoblin_vr_devices_scan(&streamer);
    g_autofree char* ini = gnoblin_vr_devices_to_ini(devices, &streamer);
    g_print("%s", ini);
    free_streamer(&streamer);
    return 0;
}

typedef struct {
    GMainLoop* loop;
    guint pending;
    char* last;
#ifdef HAVE_LIBUDEV
    struct udev_monitor* monitor;
#endif
} Watch;

static gboolean watch_rescan(gpointer data) {
    Watch* watch = data;
    watch->pending = 0;
    GnoblinVrStreamer streamer;
    g_autoptr(GPtrArray) devices = gnoblin_vr_devices_scan(&streamer);
    g_autofree char* ini = gnoblin_vr_devices_to_ini(devices, &streamer);
    free_streamer(&streamer);
    if (g_strcmp0(ini, watch->last) == 0)
        return G_SOURCE_REMOVE;
    g_autofree char* directory = g_build_filename(g_get_user_runtime_dir(), "gnoblin-vr", NULL);
    g_mkdir_with_parents(directory, 0700);
    g_autofree char* path = gnoblin_vr_devices_path();
    g_autoptr(GError) error = NULL;
    if (!g_file_set_contents(path, ini, -1, &error)) {
        g_printerr("gnoblin-vr: cannot write %s: %s\n", path, error->message);
        return G_SOURCE_REMOVE;
    }
    (void)g_chmod(path, 0600);
    g_free(watch->last);
    watch->last = g_steal_pointer(&ini);
    g_message("gnoblin-vr: %u headset(s) detected", devices->len);
    return G_SOURCE_REMOVE;
}

/* Hotplug storms (a headset enumerates a dozen USB nodes) collapse into one scan. */
static void watch_schedule(Watch* watch) {
    if (!watch->pending)
        watch->pending = g_timeout_add(300, watch_rescan, watch);
}

#ifdef HAVE_LIBUDEV
static gboolean watch_udev_ready(GIOChannel* source, GIOCondition condition, gpointer data) {
    Watch* watch = data;
    struct udev_device* device = udev_monitor_receive_device(watch->monitor);
    if (device)
        udev_device_unref(device);
    watch_schedule(watch);
    return G_SOURCE_CONTINUE;
}
#endif

static void watch_bus_signal(GDBusConnection* bus, const char* sender, const char* path,
                             const char* interface, const char* signal, GVariant* parameters,
                             gpointer data) {
    watch_schedule(data);
}

static gboolean watch_stop(gpointer data) {
    g_main_loop_quit(((Watch*)data)->loop);
    return G_SOURCE_REMOVE;
}

int gnoblin_vr_watch_main(void) {
    Watch watch = {.loop = g_main_loop_new(NULL, FALSE)};
    g_unix_signal_add(SIGTERM, watch_stop, &watch);
    g_unix_signal_add(SIGINT, watch_stop, &watch);
    watch_rescan(&watch);

#ifdef HAVE_LIBUDEV
    struct udev* udev = udev_new();
    struct udev_monitor* monitor = udev ? udev_monitor_new_from_netlink(udev, "udev") : NULL;
    watch.monitor = monitor;
    if (monitor) {
        udev_monitor_filter_add_match_subsystem_devtype(monitor, "usb", NULL);
        udev_monitor_filter_add_match_subsystem_devtype(monitor, "drm", NULL);
        udev_monitor_enable_receiving(monitor);
        GIOChannel* channel = g_io_channel_unix_new(udev_monitor_get_fd(monitor));
        g_io_add_watch(channel, G_IO_IN, watch_udev_ready, &watch);
    } else {
        g_printerr("gnoblin-vr: cannot watch udev; headset hotplug will not be noticed\n");
    }
#endif
    /* The streamer announces headsets over D-Bus: a headset joins when it connects over Wi-Fi
     * or the USB link, and leaves when it drops. */
    g_autoptr(GDBusConnection) bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, NULL);
    if (bus) {
        g_dbus_connection_signal_subscribe(
            bus, "io.github.wivrn.Server", "org.freedesktop.DBus.Properties", "PropertiesChanged",
            "/io/github/wivrn/Server", NULL, G_DBUS_SIGNAL_FLAGS_NONE, watch_bus_signal, &watch,
            NULL);
        g_dbus_connection_signal_subscribe(bus, "org.freedesktop.DBus", "org.freedesktop.DBus",
                                           "NameOwnerChanged", "/org/freedesktop/DBus",
                                           "io.github.wivrn.Server", G_DBUS_SIGNAL_FLAGS_NONE,
                                           watch_bus_signal, &watch, NULL);
    }
    g_main_loop_run(watch.loop);
    g_free(watch.last);
    g_main_loop_unref(watch.loop);
    return 0;
}

static gboolean run_adb(const char* serial, const char* const* arguments, char** output) {
    GPtrArray* argv = g_ptr_array_new();
    g_ptr_array_add(argv, (gpointer) "adb");
    if (serial && *serial) {
        g_ptr_array_add(argv, (gpointer) "-s");
        g_ptr_array_add(argv, (gpointer)serial);
    }
    for (guint i = 0; arguments[i]; i++)
        g_ptr_array_add(argv, (gpointer)arguments[i]);
    g_ptr_array_add(argv, NULL);
    int status = -1;
    g_autoptr(GError) error = NULL;
    char* standard_output = NULL;
    gboolean spawned = g_spawn_sync(NULL, (char**)argv->pdata, NULL,
                                    G_SPAWN_SEARCH_PATH | G_SPAWN_STDERR_TO_DEV_NULL, NULL, NULL,
                                    &standard_output, NULL, &status, &error);
    g_ptr_array_free(argv, TRUE);
    gboolean ok = spawned && g_spawn_check_wait_status(status, NULL);
    if (output)
        *output = standard_output;
    else
        g_free(standard_output);
    return ok;
}

/* adb needs the headset's USB serial, which matches the sysfs serial the scan recorded. The
 * serial can be absent from an unauthorized device; adb then lists it as "unauthorized". */
int gnoblin_vr_connect_main(const char* id) {
    g_autoptr(GPtrArray) devices = gnoblin_vr_devices_scan(NULL);
    const GnoblinVrDevice* target = NULL;
    for (guint i = 0; i < devices->len && !target; i++) {
        const GnoblinVrDevice* device = g_ptr_array_index(devices, i);
        if (g_str_equal(device->transport, "usb") && g_str_equal(device->kind, "standalone") &&
            (!id || g_str_equal(device->id, id)))
            target = device;
    }
    if (!target) {
        g_print("gnoblin-vr: no standalone headset is attached over USB\n");
        return 1;
    }
    g_print("headset: %s (%s)\n", target->name, target->id);

    g_autofree char* listing = NULL;
    const char* const list[] = {"devices", NULL};
    if (!run_adb(NULL, list, &listing)) {
        g_print("gnoblin-vr: adb is missing or not running; install android-tools\n");
        return 1;
    }
    gboolean listed = FALSE, unauthorized = FALSE;
    g_auto(GStrv) lines = g_strsplit(listing ? listing : "", "\n", -1);
    for (char** line = lines; *line; line++) {
        if (!target->serial || !*target->serial || !g_str_has_prefix(*line, target->serial))
            continue;
        listed = TRUE;
        unauthorized = strstr(*line, "unauthorized") != NULL;
    }
    if (!listed) {
        g_print("gnoblin-vr: adb does not list this headset. Enable developer mode for it in "
                "the phone app, then replug the cable\n");
        return 1;
    }
    if (unauthorized) {
        g_print("gnoblin-vr: put the headset on and accept the USB debugging prompt, then "
                "run this again\n");
        return 1;
    }

    const char* serial = target->serial;
    const char* const reverse[] = {"reverse", "tcp:9757", "tcp:9757", NULL};
    if (!run_adb(serial, reverse, NULL)) {
        g_print("gnoblin-vr: adb reverse failed\n");
        return 1;
    }
    g_print("usb link: tcp:9757 forwarded\n");

    g_autofree char* packages = NULL;
    const char* const query[] = {"shell", "pm", "list", "packages", "org.meumeu.wivrn", NULL};
    run_adb(serial, query, &packages);
    if (!packages || !strstr(packages, "org.meumeu.wivrn")) {
        const char* apk = g_getenv("GNOBLIN_VR_WIVRN_APK");
        if (apk && g_file_test(apk, G_FILE_TEST_IS_REGULAR)) {
            const char* const install[] = {"install", "-r", apk, NULL};
            if (!run_adb(serial, install, NULL)) {
                g_print("gnoblin-vr: installing %s failed\n", apk);
                return 1;
            }
            g_print("installed the WiVRn app from %s\n", apk);
        } else {
            g_print("gnoblin-vr: the WiVRn app is not installed on the headset. Install it "
                    "from the headset store or set GNOBLIN_VR_WIVRN_APK to an APK\n");
            return 1;
        }
    }
    const char* const start[] = {"shell",
                                 "am",
                                 "start",
                                 "-a",
                                 "android.intent.action.VIEW",
                                 "-d",
                                 "wivrn+tcp://localhost",
                                 "org.meumeu.wivrn",
                                 NULL};
    if (!run_adb(serial, start, NULL)) {
        g_print("gnoblin-vr: could not start the WiVRn app on the headset\n");
        return 1;
    }
    g_print("connecting\n");
    return 0;
}

static const char* const monado_manifests[] = {
    "/usr/share/openxr/1/openxr_monado.json", "/usr/local/share/openxr/1/openxr_monado.json",
    "/usr/share/openxr/1/monado.json", "/usr/local/share/openxr/1/monado.json", NULL};
static const char* const wivrn_manifests[] = {
    "/usr/share/openxr/1/openxr_wivrn.json", "/usr/local/share/openxr/1/openxr_wivrn.json",
    "/usr/lib/wivrn/openxr_wivrn.json", "/usr/lib64/wivrn/openxr_wivrn.json", NULL};

gboolean gnoblin_vr_select_backend(const char* backend) {
    if (g_getenv("XR_RUNTIME_JSON") && *g_getenv("XR_RUNTIME_JSON"))
        return TRUE;
    const char* const* manifests = NULL;
    if (g_str_equal(backend, "monado"))
        manifests = monado_manifests;
    else if (g_str_equal(backend, "wivrn"))
        manifests = wivrn_manifests;
    else
        return FALSE;
    for (guint i = 0; manifests[i]; i++) {
        if (g_file_test(manifests[i], G_FILE_TEST_IS_REGULAR)) {
            g_setenv("XR_RUNTIME_JSON", manifests[i], TRUE);
            g_message("gnoblin-vr: %s runtime %s", backend, manifests[i]);
            return TRUE;
        }
    }
    g_printerr("gnoblin-vr: the %s OpenXR runtime is not installed\n", backend);
    return FALSE;
}

int gnoblin_vr_runtime_main(const char* backend) {
    if (g_str_equal(backend, "monado")) {
        /* Tethered headsets scan out through a DRM lease. Direct mode is requested explicitly
         * so a headset is never opened as a window on the desktop. */
        g_setenv("XRT_COMPOSITOR_FORCE_WAYLAND_DIRECT", "1", FALSE);
        g_setenv("IPC_EXIT_ON_DISCONNECT", "0", FALSE);
        execlp("monado-service", "monado-service", (char*)NULL);
        g_printerr("gnoblin-vr: cannot run monado-service: %s\n", g_strerror(errno));
        return 1;
    }
    if (g_str_equal(backend, "wivrn")) {
        const char* override = g_getenv("GNOBLIN_VR_WIVRN_SERVER");
        if (override && *override)
            execl(override, override, (char*)NULL);
        execlp("wivrn-server", "wivrn-server", (char*)NULL);
        g_printerr("gnoblin-vr: cannot run wivrn-server: %s\n", g_strerror(errno));
        return 1;
    }
    g_printerr("gnoblin-vr: unknown runtime '%s' (use monado or wivrn)\n", backend);
    return 2;
}
