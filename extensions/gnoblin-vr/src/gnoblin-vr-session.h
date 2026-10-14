#pragma once

#include <glib.h>

/* Command line modes that manage headsets rather than render a scene. Each returns a process
 * exit code. */

/* --devices: print the current headset list as ini and exit. */
int gnoblin_vr_devices_main(void);
/* --watch: keep $XDG_RUNTIME_DIR/gnoblin-vr/devices.ini current while headsets come and go. */
int gnoblin_vr_watch_main(void);
/* --connect [ID]: bring up the link to a standalone headset on USB through adb. */
int gnoblin_vr_connect_main(const char* id);
/* --runtime monado|wivrn: replace this process with the OpenXR runtime for a backend. */
int gnoblin_vr_runtime_main(const char* backend);
/* Points the OpenXR loader at the runtime manifest of @backend ("monado", "wivrn"). */
gboolean gnoblin_vr_select_backend(const char* backend);
