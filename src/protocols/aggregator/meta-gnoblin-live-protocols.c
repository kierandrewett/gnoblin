#include "config.h"

#include "wayland/meta-gnoblin-live-protocols.h"

#include "core/gnoblin-native-control.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/meta-wayland-session-lock.h"

#define PROTOCOL_GLOBALS_KEY "gnoblin-protocol-global-registrations"

typedef struct {
    MetaWaylandCompositor *compositor;
    const char *config_name;
    const struct wl_interface *interface;
    uint32_t version;
    void *data;
    wl_global_bind_func_t bind;
    MetaGnoblinProtocolGlobalChangedFunc changed;
    gpointer changed_data;
    struct wl_global *global;
} GnoblinProtocolGlobal;

static void
protocol_global_withdraw (GnoblinProtocolGlobal *registration)
{
    if (!registration->global)
        return;

    /* Failed startup may finalize without prepare-shutdown. The Wayland
     * display already destroyed its globals in that case; never dereference
     * their stale pointers or invoke filter callbacks from qdata cleanup. */
    if (!meta_wayland_compositor_get_wayland_display (registration->compositor))
      {
        registration->global = NULL;
        return;
      }

    /* The protocol hook may hold the global in Mutter's filter manager. It
     * must be removed before the compositor finalizes that manager. */
    if (registration->changed)
        registration->changed (registration->compositor, registration->global, FALSE,
                               registration->changed_data);
    wl_global_remove (registration->global);
    wl_global_destroy (registration->global);
    registration->global = NULL;
}

static void
protocol_global_free (gpointer data)
{
    GnoblinProtocolGlobal *registration = data;

    protocol_global_withdraw (registration);
    g_free (registration);
}

static void
protocol_globals_prepare_shutdown (MetaWaylandCompositor *compositor,
                                   gpointer               data)
{
    GPtrArray *registrations = data;

    /* prepare-shutdown runs before MetaWaylandCompositor finalizes its filter
     * manager. Clearing the pointer makes the later qdata finalizer a no-op. */
    for (guint i = 0; i < registrations->len; i++)
        protocol_global_withdraw (g_ptr_array_index (registrations, i));
}

static GPtrArray *
protocol_globals (MetaWaylandCompositor *compositor)
{
    GPtrArray *registrations = g_object_get_data (G_OBJECT (compositor),
                                                   PROTOCOL_GLOBALS_KEY);
    if (!registrations)
      {
        registrations = g_ptr_array_new_with_free_func (protocol_global_free);
        g_object_set_data_full (G_OBJECT (compositor), PROTOCOL_GLOBALS_KEY, registrations,
                                (GDestroyNotify) g_ptr_array_unref);
        g_signal_connect (compositor, "prepare-shutdown",
                          G_CALLBACK (protocol_globals_prepare_shutdown), registrations);
      }
    return registrations;
}

static gboolean set_protocol_global(GnoblinProtocolGlobal *registration, gboolean enabled,
                                    GError **error) {
    if (enabled == (registration->global != NULL))
        return TRUE;

    if (!enabled) {
        protocol_global_withdraw (registration);
        return TRUE;
    }

    MetaWaylandCompositor *compositor = registration->compositor;
    registration->global = wl_global_create(
        meta_wayland_compositor_get_wayland_display(compositor), registration->interface,
        registration->version, registration->data, registration->bind);
    if (!registration->global) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                    "could not enable Wayland protocol %s", registration->config_name);
        return FALSE;
    }
    if (registration->changed)
        registration->changed(registration->compositor, registration->global, TRUE,
                              registration->changed_data);
    return TRUE;
}

gboolean meta_gnoblin_register_protocol_global(MetaWaylandCompositor *compositor,
                                               const char *config_name,
                                               const struct wl_interface *interface,
                                               uint32_t version,
                                               void *data,
                                               wl_global_bind_func_t bind,
                                               GError **error) {
    return meta_gnoblin_register_protocol_global_full(compositor, config_name, interface, version,
                                                       data, bind, NULL, NULL, error);
}

gboolean meta_gnoblin_register_protocol_global_full(
    MetaWaylandCompositor *compositor, const char *config_name,
    const struct wl_interface *interface, uint32_t version, void *data,
    wl_global_bind_func_t bind, MetaGnoblinProtocolGlobalChangedFunc changed,
    gpointer changed_data, GError **error) {
    g_return_val_if_fail(META_IS_WAYLAND_COMPOSITOR(compositor), FALSE);
    g_return_val_if_fail(config_name && *config_name, FALSE);
    g_return_val_if_fail(interface && bind, FALSE);

    GnoblinProtocolGlobal *registration = g_new0(GnoblinProtocolGlobal, 1);
    registration->compositor = compositor;
    registration->config_name = config_name;
    registration->interface = interface;
    registration->version = version;
    registration->data = data;
    registration->bind = bind;
    registration->changed = changed;
    registration->changed_data = changed_data;
    g_ptr_array_add(protocol_globals(compositor), registration);

    return set_protocol_global(registration,
                               gnoblin_native_control_protocol_enabled(config_name), error);
}

gboolean meta_gnoblin_apply_protocol_configuration(MetaWaylandCompositor *compositor,
                                                   GError **error) {
    g_return_val_if_fail(META_IS_WAYLAND_COMPOSITOR(compositor), FALSE);
    GPtrArray *registrations = g_object_get_data(G_OBJECT(compositor), PROTOCOL_GLOBALS_KEY);
    if (registrations) {
        for (guint i = 0; i < registrations->len; i++) {
            GnoblinProtocolGlobal *registration = g_ptr_array_index(registrations, i);
            if (!set_protocol_global(registration,
                                     gnoblin_native_control_protocol_enabled(
                                         registration->config_name), error))
                return FALSE;
        }
    }
    return meta_wayland_session_lock_configure_protocol(
        compositor, gnoblin_native_control_protocol_enabled("ext-session-lock"), error);
}
