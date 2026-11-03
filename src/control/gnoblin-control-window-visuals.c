/* Visual transforms that the Lua runtime sets on a window: opacity, scale, rotation and offset.
 *
 * They change only how the window actor is drawn (and picked): not its frame, workspace, focus
 * or what the client sees. Each override belongs to the runtime generation that set it and is
 * cleared when that runtime reloads or its worker stops, so a config never inherits a stale
 * transform. Scale and rotation pivot on the window's centre. The window rule `opacity` sits on
 * the surface actor, so the two multiply.
 *
 * window.set_visual {id, opacity?, scale?, rotation?, offset_x?, offset_y?, reset?} */
#include "config.h"

#include <math.h>

#include "compositor/meta-window-actor-private.h"
#include "core/gnoblin-control-internal.h"
#include "core/gnoblin-native-control-private.h"
#include "core/window-private.h"

#define WINDOW_VISUAL_KEY "gnoblin-window-visual"
#define VISUAL_OFFSET_LIMIT 100000.0
#define VISUAL_SCALE_LIMIT 10.0
#define VISUAL_ROTATION_LIMIT 3600.0

typedef struct {
    double opacity;
    double scale;
    double rotation;
    double offset_x;
    double offset_y;
    guint64 owner_generation;
} WindowVisual;

static const WindowVisual identity_visual = {1.0, 1.0, 0.0, 0.0, 0.0, 0};

static void apply_visual(MetaWindow* window, const WindowVisual* visual) {
    MetaWindowActor* window_actor = meta_window_actor_from_window(window);
    if (!window_actor)
        return;
    ClutterActor* actor = CLUTTER_ACTOR(window_actor);
    clutter_actor_set_pivot_point(actor, 0.5f, 0.5f);
    clutter_actor_set_opacity(actor, (guint8)lround(CLAMP(visual->opacity, 0.0, 1.0) * 255.0));
    clutter_actor_set_scale(actor, visual->scale, visual->scale);
    clutter_actor_set_rotation_angle(actor, CLUTTER_Z_AXIS, visual->rotation);
    clutter_actor_set_translation(actor, (float)visual->offset_x, (float)visual->offset_y, 0.0f);
}

static gboolean lookup_number(GVariant* arguments, const char* key, double minimum, double maximum,
                              double* value, GError** error) {
    g_autoptr(GVariant) field = g_variant_lookup_value(arguments, key, NULL);
    if (!field)
        return TRUE;
    double number;
    if (g_variant_is_of_type(field, G_VARIANT_TYPE_DOUBLE))
        number = g_variant_get_double(field);
    else if (g_variant_is_of_type(field, G_VARIANT_TYPE_INT64))
        number = (double)g_variant_get_int64(field);
    else if (g_variant_is_of_type(field, G_VARIANT_TYPE_INT32))
        number = g_variant_get_int32(field);
    else
        number = NAN;
    if (!isfinite(number) || number < minimum || number > maximum) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "window.set_visual %s must be a number from %g to %g", key, minimum, maximum);
        return FALSE;
    }
    *value = number;
    return TRUE;
}

GVariant* gnoblin_control_window_set_visual(GnoblinNativeControl* control, GVariant* arguments,
                                            guint64 owner_generation, GError** error) {
    static const char* const allowed[] = {"id",       "opacity",  "scale", "rotation",
                                          "offset_x", "offset_y", "reset", NULL};
    GVariantIter iter;
    const char* key;
    g_variant_iter_init(&iter, arguments);
    while (g_variant_iter_next(&iter, "{&sv}", &key, NULL)) {
        if (!g_strv_contains(allowed, key)) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "window.set_visual does not accept %s", key);
            return NULL;
        }
    }
    if (owner_generation != control->runtime_generation) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.set_visual belongs to an old runtime generation");
        return NULL;
    }
    if (control->wayland_compositor &&
        meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window visuals cannot change while the session is locked");
        return NULL;
    }
    const char* id = NULL;
    MetaWindow* window = g_variant_lookup(arguments, "id", "&s", &id)
                             ? native_window_by_stable_id(control, id)
                             : NULL;
    if (!window || !meta_window_actor_from_window(window)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "window.set_visual target is unavailable");
        return NULL;
    }

    WindowVisual* current = g_object_get_data(G_OBJECT(window), WINDOW_VISUAL_KEY);
    gboolean reset = FALSE;
    g_variant_lookup(arguments, "reset", "b", &reset);
    WindowVisual next = current && !reset ? *current : identity_visual;
    if (!lookup_number(arguments, "opacity", 0.0, 1.0, &next.opacity, error) ||
        !lookup_number(arguments, "scale", 0.01, VISUAL_SCALE_LIMIT, &next.scale, error) ||
        !lookup_number(arguments, "rotation", -VISUAL_ROTATION_LIMIT, VISUAL_ROTATION_LIMIT,
                       &next.rotation, error) ||
        !lookup_number(arguments, "offset_x", -VISUAL_OFFSET_LIMIT, VISUAL_OFFSET_LIMIT,
                       &next.offset_x, error) ||
        !lookup_number(arguments, "offset_y", -VISUAL_OFFSET_LIMIT, VISUAL_OFFSET_LIMIT,
                       &next.offset_y, error))
        return NULL;
    next.owner_generation = owner_generation;

    apply_visual(window, &next);
    if (memcmp(&next, &identity_visual, offsetof(WindowVisual, owner_generation)) == 0)
        g_object_set_data(G_OBJECT(window), WINDOW_VISUAL_KEY, NULL);
    else
        g_object_set_data_full(G_OBJECT(window), WINDOW_VISUAL_KEY, g_memdup2(&next, sizeof next),
                               g_free);
    schedule_windows(control);

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(id));
    g_variant_builder_add(&result, "{sv}", "opacity", g_variant_new_double(next.opacity));
    g_variant_builder_add(&result, "{sv}", "scale", g_variant_new_double(next.scale));
    g_variant_builder_add(&result, "{sv}", "rotation", g_variant_new_double(next.rotation));
    g_variant_builder_add(&result, "{sv}", "offset_x", g_variant_new_double(next.offset_x));
    g_variant_builder_add(&result, "{sv}", "offset_y", g_variant_new_double(next.offset_y));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

/* Add the window's visual state to its record, the identity when nothing overrides it. */
void gnoblin_control_window_visual_add_fields(MetaWindow* window, JsonObject* record) {
    const WindowVisual* visual = g_object_get_data(G_OBJECT(window), WINDOW_VISUAL_KEY);
    if (!visual)
        visual = &identity_visual;
    json_object_set_double_member(record, "opacity", visual->opacity);
    json_object_set_double_member(record, "scale", visual->scale);
    json_object_set_double_member(record, "rotation", visual->rotation);
    JsonObject* offset = json_object_new();
    json_object_set_double_member(offset, "x", visual->offset_x);
    json_object_set_double_member(offset, "y", visual->offset_y);
    json_object_set_object_member(record, "offset", offset);
}

/* Clear every override, for a runtime reload or a stopped worker. */
void gnoblin_control_window_visuals_reset(GnoblinNativeControl* control) {
    if (!control || !control->display)
        return;
    gboolean changed = FALSE;
    g_autoptr(GList) windows = meta_display_list_all_windows(control->display);
    for (GList* item = windows; item; item = item->next) {
        MetaWindow* window = item->data;
        if (!g_object_get_data(G_OBJECT(window), WINDOW_VISUAL_KEY))
            continue;
        apply_visual(window, &identity_visual);
        g_object_set_data(G_OBJECT(window), WINDOW_VISUAL_KEY, NULL);
        changed = TRUE;
    }
    if (changed)
        schedule_windows(control);
}
