/* PipeWire microphone and camera activity monitor for the native Gnoblin runtime. */
#pragma once

#include <glib.h>

G_BEGIN_DECLS

typedef struct _GnoblinPipewireMonitor GnoblinPipewireMonitor;

typedef void (*GnoblinPipewireMonitorCallback)(GnoblinPipewireMonitor* monitor, gboolean available,
                                               gboolean microphone_in_use, gboolean camera_in_use,
                                               gpointer user_data);

/*
 * The callback is dispatched on @context. If it is NULL, the default main
 * context is used. The callback receives availability independently from
 * microphone and camera activity: an unavailable PipeWire server is not
 * reported as active.
 */
GnoblinPipewireMonitor* gnoblin_pipewire_monitor_new(GMainContext* context,
                                                     GnoblinPipewireMonitorCallback callback,
                                                     gpointer user_data,
                                                     GDestroyNotify user_data_destroy);

/* Start observation and periodic recovery attempts. Returns FALSE if already started. */
gboolean gnoblin_pipewire_monitor_start(GnoblinPipewireMonitor* monitor);
void gnoblin_pipewire_monitor_stop(GnoblinPipewireMonitor* monitor);
void gnoblin_pipewire_monitor_free(GnoblinPipewireMonitor* monitor);

G_DEFINE_AUTOPTR_CLEANUP_FUNC(GnoblinPipewireMonitor, gnoblin_pipewire_monitor_free)

G_END_DECLS
