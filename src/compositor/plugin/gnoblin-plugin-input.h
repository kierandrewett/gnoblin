/* Private bridge from Mutter's event filter to the executable-owned UI.
 * No exported symbol or shell protocol is involved. The plugin owns the
 * bridge and detaches it before freeing its panels. */
#pragma once

#include <glib-object.h>
#include <clutter/clutter.h>

typedef struct {
    gboolean (*handle_event)(const ClutterEvent* event, gpointer user_data);
    gpointer user_data;
    /* Optional. Called for key events before any keybinding, shortcut or input policy sees them.
     * TRUE consumes the event. */
    gboolean (*handle_key)(const ClutterEvent* event, gpointer user_data);
} GnoblinPluginInput;

#define GNOBLIN_PLUGIN_INPUT_DATA_KEY "gnoblin-plugin-input"

static inline void gnoblin_plugin_input_attach(GObject* display, GnoblinPluginInput* input) {
    g_object_set_data(display, GNOBLIN_PLUGIN_INPUT_DATA_KEY, input);
}

static inline void gnoblin_plugin_input_detach(GObject* display, GnoblinPluginInput* input) {
    if (g_object_get_data(display, GNOBLIN_PLUGIN_INPUT_DATA_KEY) == input)
        g_object_set_data(display, GNOBLIN_PLUGIN_INPUT_DATA_KEY, NULL);
}

/* Called first for key events; the modal console takes every key here so that no keybinding runs
 * while it is open. */
static inline gboolean gnoblin_plugin_input_handle_key(GObject* display,
                                                       const ClutterEvent* event) {
    GnoblinPluginInput* input = g_object_get_data(display, GNOBLIN_PLUGIN_INPUT_DATA_KEY);
    return input && input->handle_key && input->handle_key(event, input->user_data);
}

/* Caller has already applied lock/capture/grab embargoes and updated the
 * Wayland seat. FALSE leaves normal window and client dispatch untouched. */
static inline gboolean gnoblin_plugin_input_handle_event(GObject* display,
                                                         const ClutterEvent* event) {
    GnoblinPluginInput* input = g_object_get_data(display, GNOBLIN_PLUGIN_INPUT_DATA_KEY);
    return input && input->handle_event && input->handle_event(event, input->user_data);
}
