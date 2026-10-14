#define _GNU_SOURCE
#include "gnoblin-vr-devices.h"

#include <gio/gio.h>
#include <fcntl.h>
#include <string.h>
#include <unistd.h>

#ifdef HAVE_LIBDRM
#include <xf86drm.h>
#include <xf86drmMode.h>
#endif

typedef struct {
    guint16 vendor;
    guint16 product;       /* 0 matches any product of the vendor */
    const char* name_part; /* case-insensitive part of the USB product string, or NULL */
    const char* vendor_name;
    const char* kind;
    const char* backend;
} UsbRule;

/* Order matters: the first match wins, so specific rules precede vendor-wide ones. A Valve or
 * HTC USB ID also covers a Steam Deck or a phone, so those vendors only match by product name. */
static const UsbRule usb_rules[] = {
    {0x2833, 0, "Quest", "Meta", "standalone", "wivrn"},
    {0x2833, 0, "Rift", "Meta", "tethered", "monado"},
    {0x2833, 0x0031, NULL, "Meta", "tethered", "monado"},
    {0x2833, 0x0021, NULL, "Meta", "tethered", "monado"},
    {0x2d40, 0, NULL, "Pico", "standalone", "wivrn"},
    {0x0bb4, 0, "Focus", "HTC", "standalone", "wivrn"},
    {0x0bb4, 0, "Vive", "HTC", "tethered", "monado"},
    {0x28de, 0, "Index", "Valve", "tethered", "monado"},
    {0x28de, 0, "HMD", "Valve", "tethered", "monado"},
    {0x054c, 0x09af, NULL, "Sony", "tethered", "monado"},
    {0x054c, 0x0cde, NULL, "Sony", "tethered", "monado"},
    {0x045e, 0x0659, NULL, "Microsoft", "tethered", "monado"},
};

/* Display EDID manufacturer IDs of tethered headsets, for naming a leased connector. */
static const struct {
    const char* pnp;
    const char* vendor;
} drm_vendors[] = {
    {"VLV", "Valve"}, {"HVR", "HTC"},       {"OVR", "Meta"},      {"SNY", "Sony"},
    {"PMX", "Pimax"}, {"MSF", "Microsoft"}, {"BBE", "Bigscreen"}, {"PSM", "Pimax"},
};

void gnoblin_vr_device_free(GnoblinVrDevice* device) {
    if (!device)
        return;
    g_free(device->id);
    g_free(device->transport);
    g_free(device->kind);
    g_free(device->backend);
    g_free(device->vendor);
    g_free(device->name);
    g_free(device->state);
    g_free(device->diagnostic);
    g_free(device->serial);
    g_free(device);
}

gboolean gnoblin_vr_classify_usb(guint16 vendor, guint16 product, const char* product_name,
                                 const char** kind, const char** backend,
                                 const char** vendor_name) {
    /* Controllers, trackers and dongles share a headset vendor's USB ID but are not headsets. */
    static const char* const accessories[] = {"Controller", "Receiver", "Dongle",
                                              "Station",    "Tracker",  "Lighthouse"};
    for (guint i = 0; product_name && i < G_N_ELEMENTS(accessories); i++)
        if (strcasestr(product_name, accessories[i]))
            return FALSE;
    for (guint i = 0; i < G_N_ELEMENTS(usb_rules); i++) {
        const UsbRule* rule = &usb_rules[i];
        if (rule->vendor != vendor)
            continue;
        if (rule->product && rule->product != product)
            continue;
        if (rule->name_part && (!product_name || !strcasestr(product_name, rule->name_part)))
            continue;
        *kind = rule->kind;
        *backend = rule->backend;
        *vendor_name = rule->vendor_name;
        return TRUE;
    }
    return FALSE;
}

gboolean gnoblin_vr_parse_edid(const guint8* edid, gsize length, char manufacturer[4], char* name,
                               gsize name_size) {
    static const guint8 magic[8] = {0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0};
    if (length < 128 || memcmp(edid, magic, sizeof magic) != 0)
        return FALSE;
    guint16 packed = (guint16)((edid[8] << 8) | edid[9]);
    manufacturer[0] = (char)('@' + ((packed >> 10) & 0x1f));
    manufacturer[1] = (char)('@' + ((packed >> 5) & 0x1f));
    manufacturer[2] = (char)('@' + (packed & 0x1f));
    manufacturer[3] = 0;
    if (name_size)
        name[0] = 0;
    for (guint block = 54; block + 18 <= 126 && name_size; block += 18) {
        if (edid[block] || edid[block + 1] || edid[block + 3] != 0xfc)
            continue;
        gsize out = 0;
        for (guint i = 0; i < 13 && out + 1 < name_size; i++) {
            guint8 c = edid[block + 5 + i];
            if (c == '\n' || c == 0)
                break;
            name[out++] = (char)c;
        }
        name[out] = 0;
        g_strstrip(name);
        break;
    }
    return TRUE;
}

char* gnoblin_vr_devices_path(void) {
    return g_build_filename(g_get_user_runtime_dir(), "gnoblin-vr", "devices.ini", NULL);
}

static char* read_trimmed(const char* directory, const char* file) {
    g_autofree char* path = g_build_filename(directory, file, NULL);
    g_autofree char* contents = NULL;
    if (!g_file_get_contents(path, &contents, NULL, NULL))
        return NULL;
    return g_strstrip(g_steal_pointer(&contents));
}

static GnoblinVrDevice* new_device(const char* id, const char* transport, const char* kind,
                                   const char* backend, const char* vendor, const char* name,
                                   const char* state, const char* diagnostic) {
    GnoblinVrDevice* device = g_new0(GnoblinVrDevice, 1);
    device->id = g_strdup(id);
    device->transport = g_strdup(transport);
    device->kind = g_strdup(kind);
    device->backend = g_strdup(backend);
    device->vendor = g_strdup(vendor ? vendor : "");
    device->name = g_strdup(name ? name : "");
    device->state = g_strdup(state);
    device->diagnostic = g_strdup(diagnostic ? diagnostic : "");
    return device;
}

static void scan_usb(GPtrArray* out, const char* directory) {
    g_autoptr(GDir) dir = g_dir_open(directory, 0, NULL);
    if (!dir)
        return;
    const char* entry;
    while ((entry = g_dir_read_name(dir))) {
        if (strchr(entry, ':'))
            continue; /* interface nodes */
        g_autofree char* device_dir = g_build_filename(directory, entry, NULL);
        g_autofree char* vendor_text = read_trimmed(device_dir, "idVendor");
        g_autofree char* product_text = read_trimmed(device_dir, "idProduct");
        if (!vendor_text || !product_text)
            continue;
        guint16 vendor = (guint16)g_ascii_strtoull(vendor_text, NULL, 16);
        guint16 product = (guint16)g_ascii_strtoull(product_text, NULL, 16);
        g_autofree char* product_name = read_trimmed(device_dir, "product");
        g_autofree char* serial = read_trimmed(device_dir, "serial");
        const char *kind, *backend, *vendor_name;
        if (!gnoblin_vr_classify_usb(vendor, product, product_name, &kind, &backend, &vendor_name))
            continue;
        g_autofree char* id = g_strdup_printf("usb:%04x:%04x:%s", vendor, product,
                                              serial && *serial ? serial : entry);
        GnoblinVrDevice* device = new_device(
            id, "usb", kind, backend, vendor_name, product_name ? product_name : "Headset",
            g_str_equal(kind, "tethered") ? "waiting" : "detected", NULL);
        device->usb_vendor = vendor;
        device->usb_product = product;
        device->serial = g_strdup(serial ? serial : "");
        g_ptr_array_add(out, device);
    }
}

static void add_drm_device(GPtrArray* out, const char* connector, const char* pnp,
                           const char* monitor_name) {
    const char* vendor = pnp && *pnp ? pnp : "";
    for (guint i = 0; i < G_N_ELEMENTS(drm_vendors); i++)
        if (pnp && g_str_equal(pnp, drm_vendors[i].pnp))
            vendor = drm_vendors[i].vendor;
    g_autofree char* id = g_strdup_printf("drm:%s", connector);
    g_autofree char* name =
        g_strdup_printf("%s%s%s", vendor, *vendor && monitor_name && *monitor_name ? " " : "",
                        monitor_name ? monitor_name : "");
    if (!*name) {
        g_free(name);
        name = g_strdup("Headset display");
    }
    g_ptr_array_add(out,
                    new_device(id, "drm-lease", "tethered", "monado", vendor, name, "ready", NULL));
}

static void scan_drm(GPtrArray* out, const char* fixture) {
    if (fixture) {
        g_autofree char* path = g_build_filename(fixture, "drm", NULL);
        g_autofree char* contents = NULL;
        if (!g_file_get_contents(path, &contents, NULL, NULL))
            return;
        g_auto(GStrv) lines = g_strsplit(contents, "\n", -1);
        for (char** line = lines; *line; line++) {
            g_auto(GStrv) parts = g_strsplit(*line, "|", 3);
            if (g_strv_length(parts) == 3)
                add_drm_device(out, parts[0], parts[1], parts[2]);
        }
        return;
    }
#ifdef HAVE_LIBDRM
    g_autoptr(GDir) dir = g_dir_open("/dev/dri", 0, NULL);
    if (!dir)
        return;
    const char* entry;
    while ((entry = g_dir_read_name(dir))) {
        if (!g_str_has_prefix(entry, "card"))
            continue;
        g_autofree char* node = g_build_filename("/dev/dri", entry, NULL);
        int fd = open(node, O_RDWR | O_CLOEXEC);
        if (fd < 0)
            continue;
        drmModeRes* resources = drmModeGetResources(fd);
        for (int i = 0; resources && i < resources->count_connectors; i++) {
            drmModeConnector* connector = drmModeGetConnector(fd, resources->connectors[i]);
            if (!connector)
                continue;
            gboolean non_desktop = FALSE;
            char pnp[4] = "";
            char monitor_name[32] = "";
            for (int p = 0; p < connector->count_props; p++) {
                drmModePropertyRes* property = drmModeGetProperty(fd, connector->props[p]);
                if (!property)
                    continue;
                if (g_str_equal(property->name, "non-desktop"))
                    non_desktop = connector->prop_values[p] == 1;
                else if (g_str_equal(property->name, "EDID") && connector->prop_values[p]) {
                    drmModePropertyBlobRes* blob =
                        drmModeGetPropertyBlob(fd, (uint32_t)connector->prop_values[p]);
                    if (blob) {
                        gnoblin_vr_parse_edid(blob->data, blob->length, pnp, monitor_name,
                                              sizeof monitor_name);
                        drmModeFreePropertyBlob(blob);
                    }
                }
                drmModeFreeProperty(property);
            }
            /* A disconnected non-desktop connector has no headset behind it. */
            if (non_desktop && connector->connection == DRM_MODE_CONNECTED) {
                const char* type = drmModeGetConnectorTypeName(connector->connector_type);
                g_autofree char* name =
                    g_strdup_printf("%s-%u", type ? type : "Unknown", connector->connector_type_id);
                add_drm_device(out, name, pnp, monitor_name);
            }
            drmModeFreeConnector(connector);
        }
        if (resources)
            drmModeFreeResources(resources);
        close(fd);
    }
#endif
}

static void query_streamer(GnoblinVrStreamer* streamer, const char* fixture) {
    memset(streamer, 0, sizeof *streamer);
    if (fixture) {
        g_autofree char* path = g_build_filename(fixture, "streamer", NULL);
        g_autoptr(GKeyFile) file = g_key_file_new();
        if (!g_key_file_load_from_file(file, path, G_KEY_FILE_NONE, NULL))
            return;
        streamer->available = g_key_file_get_boolean(file, "streamer", "available", NULL);
        streamer->headset_connected =
            g_key_file_get_boolean(file, "streamer", "headset_connected", NULL);
        streamer->system_name = g_key_file_get_string(file, "streamer", "system_name", NULL);
        return;
    }
    g_autoptr(GDBusConnection) bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, NULL);
    if (!bus)
        return;
    g_autoptr(GVariant) owner = g_dbus_connection_call_sync(
        bus, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
        "NameHasOwner", g_variant_new("(s)", "io.github.wivrn.Server"), G_VARIANT_TYPE("(b)"),
        G_DBUS_CALL_FLAGS_NO_AUTO_START, 500, NULL, NULL);
    gboolean has_owner = FALSE;
    if (owner)
        g_variant_get(owner, "(b)", &has_owner);
    if (!has_owner)
        return;
    streamer->available = TRUE;
    g_autoptr(GVariant) all = g_dbus_connection_call_sync(
        bus, "io.github.wivrn.Server", "/io/github/wivrn/Server", "org.freedesktop.DBus.Properties",
        "GetAll", g_variant_new("(s)", "io.github.wivrn.Server"), G_VARIANT_TYPE("(a{sv})"),
        G_DBUS_CALL_FLAGS_NO_AUTO_START, 500, NULL, NULL);
    if (!all)
        return;
    g_autoptr(GVariant) properties = g_variant_get_child_value(all, 0);
    g_variant_lookup(properties, "HeadsetConnected", "b", &streamer->headset_connected);
    const char* system_name = NULL;
    if (g_variant_lookup(properties, "SystemName", "&s", &system_name) && *system_name)
        streamer->system_name = g_strdup(system_name);
}

static gint compare_devices(gconstpointer a, gconstpointer b) {
    const GnoblinVrDevice* const* left = a;
    const GnoblinVrDevice* const* right = b;
    return g_strcmp0((*left)->id, (*right)->id);
}

GPtrArray* gnoblin_vr_devices_scan(GnoblinVrStreamer* streamer) {
    const char* fixture = g_getenv("GNOBLIN_VR_FIXTURE");
    if (fixture && !*fixture)
        fixture = NULL;
    GnoblinVrStreamer local;
    if (!streamer)
        streamer = &local;
    query_streamer(streamer, fixture);

    GPtrArray* display = g_ptr_array_new_with_free_func((GDestroyNotify)gnoblin_vr_device_free);
    GPtrArray* usb = g_ptr_array_new_with_free_func((GDestroyNotify)gnoblin_vr_device_free);
    scan_drm(display, fixture);
    g_autofree char* usb_directory =
        fixture ? g_build_filename(fixture, "usb", NULL) : g_strdup("/sys/bus/usb/devices");
    scan_usb(usb, usb_directory);

    GPtrArray* devices = g_ptr_array_new_with_free_func((GDestroyNotify)gnoblin_vr_device_free);
    gboolean streaming_attached = FALSE;
    gboolean have_display = display->len > 0;
    while (display->len)
        g_ptr_array_add(devices, g_ptr_array_steal_index(display, 0));
    g_autoptr(GHashTable) tethered_vendors = g_hash_table_new(g_direct_hash, g_direct_equal);
    for (guint i = 0; i < usb->len; i++) {
        GnoblinVrDevice* device = g_ptr_array_index(usb, i);
        if (g_str_equal(device->kind, "tethered")) {
            /* The cable's display connector is the device. USB alone means no picture yet. */
            if (have_display ||
                g_hash_table_contains(tethered_vendors, GUINT_TO_POINTER(device->usb_vendor)))
                continue;
            g_hash_table_add(tethered_vendors, GUINT_TO_POINTER(device->usb_vendor));
            g_free(device->diagnostic);
            device->diagnostic =
                g_strdup("Connect the headset's display cable. No display connector is leasable.");
        } else if (streamer->headset_connected && !streaming_attached) {
            streaming_attached = TRUE;
            g_free(device->state);
            device->state = g_strdup("connected");
        } else {
            g_free(device->diagnostic);
            device->diagnostic = g_strdup(
                streamer->available
                    ? "Run gnoblin-vr --connect to start the USB link."
                    : "The streaming runtime is not running. Start gnoblin-vr-runtime@wivrn.");
        }
        g_ptr_array_add(devices, g_ptr_array_steal_index(usb, i--));
    }
    if (streamer->headset_connected && !streaming_attached) {
        g_autofree char* id =
            g_strdup_printf("wifi:%s", streamer->system_name ? streamer->system_name : "headset");
        g_ptr_array_add(devices,
                        new_device(id, "wifi", "standalone", "wivrn", "",
                                   streamer->system_name ? streamer->system_name : "Headset",
                                   "connected", NULL));
    }
    g_ptr_array_unref(display);
    g_ptr_array_unref(usb);
    g_ptr_array_sort(devices, compare_devices);
    return devices;
}

char* gnoblin_vr_devices_to_ini(const GPtrArray* devices, const GnoblinVrStreamer* streamer) {
    g_autoptr(GKeyFile) file = g_key_file_new();
    g_key_file_set_boolean(file, "streamer", "available", streamer->available);
    g_key_file_set_boolean(file, "streamer", "headset_connected", streamer->headset_connected);
    g_key_file_set_integer(file, "streamer", "device_count", (gint)devices->len);
    if (streamer->system_name)
        g_key_file_set_string(file, "streamer", "system_name", streamer->system_name);
    for (guint i = 0; i < devices->len; i++) {
        const GnoblinVrDevice* device = g_ptr_array_index((GPtrArray*)devices, i);
        g_autofree char* group = g_strdup_printf("device.%u", i);
        g_key_file_set_string(file, group, "id", device->id);
        g_key_file_set_string(file, group, "transport", device->transport);
        g_key_file_set_string(file, group, "kind", device->kind);
        g_key_file_set_string(file, group, "backend", device->backend);
        g_key_file_set_string(file, group, "vendor", device->vendor);
        g_key_file_set_string(file, group, "name", device->name);
        g_key_file_set_string(file, group, "state", device->state);
        if (*device->diagnostic)
            g_key_file_set_string(file, group, "diagnostic", device->diagnostic);
        if (device->serial && *device->serial)
            g_key_file_set_string(file, group, "serial", device->serial);
    }
    return g_key_file_to_data(file, NULL, NULL);
}
