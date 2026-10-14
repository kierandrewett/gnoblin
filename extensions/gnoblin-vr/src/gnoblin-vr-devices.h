#pragma once

#include <glib.h>

/* Headset discovery for the VR extension.
 *
 * A headset reaches the computer in one of two ways, and each way needs a different OpenXR
 * runtime:
 *
 *   tethered    a cable carries video: a DRM connector flagged "non-desktop" that the compositor
 *               leaves out of the desktop and offers for lease, plus USB tracking. Backend: monado.
 *   standalone  the headset runs its own system and receives the scene as a video stream over
 *               USB (adb reverse) or Wi-Fi. Backend: wivrn.
 *
 * Discovery reports what is plugged in and which backend can drive it. It does not start anything.
 * Policy is Lua's job: it reads the list and decides what to start. */

typedef struct {
    char* id;        /* stable: "drm:DP-2", "usb:2833:0186:SERIAL", "wifi:Quest 3" */
    char* transport; /* "drm-lease", "usb" or "wifi" */
    char* kind;      /* "tethered" or "standalone" */
    char* backend;   /* "monado" or "wivrn" */
    char* vendor;
    char* name;
    char* state;      /* "ready", "detected", "waiting" or "connected" */
    char* diagnostic; /* what to do next when the state is not ready */
    char* serial;     /* USB serial, used to address the headset through adb */
    guint16 usb_vendor;
    guint16 usb_product;
} GnoblinVrDevice;

typedef struct {
    gboolean available; /* the WiVRn server owns its D-Bus name */
    gboolean headset_connected;
    char* system_name; /* the connected headset, as the streamer reports it */
} GnoblinVrStreamer;

void gnoblin_vr_device_free(GnoblinVrDevice* device);

/* Classifies a USB device. Returns FALSE for anything that is not a known headset. The table
 * is a hint for what to offer; the OpenXR runtime has the final say on a session. */
gboolean gnoblin_vr_classify_usb(guint16 vendor, guint16 product, const char* product_name,
                                 const char** kind, const char** backend, const char** vendor_name);

/* Reads the EDID blob of a display into its three letter manufacturer ID and monitor name. */
gboolean gnoblin_vr_parse_edid(const guint8* edid, gsize length, char manufacturer[4], char* name,
                               gsize name_size);

/* Scans USB (sysfs), DRM (non-desktop connectors) and the streamer. Returns a
 * GPtrArray of GnoblinVrDevice sorted by id. When GNOBLIN_VR_FIXTURE names a directory the
 * scan reads it instead of the machine, which is how the tests run without hardware.
 * @streamer receives the streamer state. */
GPtrArray* gnoblin_vr_devices_scan(GnoblinVrStreamer* streamer);

/* The ini text for a scan: one [streamer] group and one [device.N] group per headset. */
char* gnoblin_vr_devices_to_ini(const GPtrArray* devices, const GnoblinVrStreamer* streamer);

char* gnoblin_vr_devices_path(void);
