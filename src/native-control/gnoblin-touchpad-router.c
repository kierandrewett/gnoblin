#include "gnoblin-touchpad-router.h"

#include <math.h>
#include <string.h>

typedef struct {
    double x;
    double y;
} Point;

struct _GnoblinTouchpadRouter {
    GVariant* gestures;
    GPtrArray* candidates;
    GArray* path;
    char* kind;
    double x;
    double y;
    double distance;
    double initial_scale;
    gboolean claimed;
};

static GVariant* unwrap_variant(GVariant* value) {
    if (value && g_variant_is_of_type(value, G_VARIANT_TYPE_VARIANT)) {
        GVariant* unwrapped = g_variant_get_variant(value);
        g_variant_unref(value);
        return unwrapped;
    }
    return value;
}

static GVariant* lookup_value(GVariant* dictionary, const char* key) {
    if (!dictionary || !g_variant_is_of_type(dictionary, G_VARIANT_TYPE_VARDICT))
        return NULL;
    return unwrap_variant(g_variant_lookup_value(dictionary, key, NULL));
}

static gboolean lookup_string(GVariant* dictionary, const char* key, char** result) {
    g_autoptr(GVariant) value = lookup_value(dictionary, key);
    if (!value || !g_variant_is_of_type(value, G_VARIANT_TYPE_STRING))
        return FALSE;
    *result = g_strdup(g_variant_get_string(value, NULL));
    return TRUE;
}

static gboolean variant_number(GVariant* value, double* result) {
    if (!value)
        return FALSE;
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
        *result = g_variant_get_double(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
        *result = (double)g_variant_get_int64(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_UINT64))
        *result = (double)g_variant_get_uint64(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32))
        *result = (double)g_variant_get_int32(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_UINT32))
        *result = (double)g_variant_get_uint32(value);
    else
        return FALSE;
    return isfinite(*result);
}

static gboolean lookup_number(GVariant* dictionary, const char* key, double* result) {
    g_autoptr(GVariant) value = lookup_value(dictionary, key);
    return variant_number(value, result);
}

static gboolean lookup_integer(GVariant* dictionary, const char* key, gint64* result) {
    double number;
    if (!lookup_number(dictionary, key, &number) || number != floor(number) ||
        number < G_MININT64 || number > G_MAXINT64)
        return FALSE;
    *result = (gint64)number;
    return TRUE;
}

static gboolean gesture_has_action(GVariant* gesture) {
    g_autoptr(GVariant) action = lookup_value(gesture, "action");
    if (action && g_variant_is_of_type(action, G_VARIANT_TYPE_STRING)) {
        const char* value = g_variant_get_string(action, NULL);
        return *value && !g_str_has_suffix(value, ".progress");
    }

    g_autoptr(GVariant) command = lookup_value(gesture, "command");
    return command && g_variant_is_of_type(command, G_VARIANT_TYPE("av")) &&
           g_variant_n_children(command) > 0;
}

static gboolean gesture_matches_begin(GVariant* gesture, const char* kind, gint64 fingers,
                                      const char* context) {
    g_autofree char* gesture_kind = NULL;
    const char* when = "normal";
    gint64 configured_fingers;
    if (!lookup_string(gesture, "gesture", &gesture_kind) ||
        !lookup_integer(gesture, "fingers", &configured_fingers) || !gesture_has_action(gesture) ||
        !g_str_equal(gesture_kind, kind) || configured_fingers != fingers)
        return FALSE;

    g_autoptr(GVariant) when_value = lookup_value(gesture, "when");
    if (when_value) {
        if (!g_variant_is_of_type(when_value, G_VARIANT_TYPE_STRING))
            return FALSE;
        when = g_variant_get_string(when_value, NULL);
    }
    return g_str_equal(when, "any") || (context && g_str_equal(when, context));
}

static gboolean append_path_point(GArray* path, double x, double y) {
    Point point = {x, y};
    g_array_append_val(path, point);
    return TRUE;
}

static GArray* read_path(GVariant* value) {
    if (!value || !g_variant_is_of_type(value, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(value) < 2)
        return NULL;

    GArray* path = g_array_sized_new(FALSE, FALSE, sizeof(Point), g_variant_n_children(value));
    for (gsize i = 0; i < g_variant_n_children(value); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
        g_autoptr(GVariant) point = unwrap_variant(g_steal_pointer(&boxed));
        double x, y;
        if (!point || !lookup_number(point, "x", &x) || !lookup_number(point, "y", &y)) {
            g_array_unref(path);
            return NULL;
        }
        append_path_point(path, x, y);
    }
    return path;
}

static GArray* resample_normalized_path(GArray* path) {
    if (!path || path->len < 2)
        return NULL;

    double scale = 0;
    for (guint i = 0; i < path->len; i++) {
        Point point = g_array_index(path, Point, i);
        scale = MAX(scale, MAX(fabs(point.x), fabs(point.y)));
    }
    if (scale < 1e-6)
        return NULL;

    g_autofree double* lengths = g_new0(double, path->len);
    for (guint i = 1; i < path->len; i++) {
        Point previous = g_array_index(path, Point, i - 1);
        Point point = g_array_index(path, Point, i);
        lengths[i] = lengths[i - 1] + hypot(point.x - previous.x, point.y - previous.y) / scale;
    }
    if (lengths[path->len - 1] < 1e-6)
        return NULL;

    GArray* resampled = g_array_sized_new(FALSE, FALSE, sizeof(Point), 16);
    guint segment = 1;
    const double total = lengths[path->len - 1];
    for (guint i = 0; i < 16; i++) {
        const double target = total * i / 15.0;
        while (segment < path->len - 1 && lengths[segment] < target)
            segment++;
        Point start = g_array_index(path, Point, segment - 1);
        Point end = g_array_index(path, Point, segment);
        const double span = lengths[segment] - lengths[segment - 1];
        const double amount = span == 0 ? 0 : (target - lengths[segment - 1]) / span;
        append_path_point(resampled, (start.x + (end.x - start.x) * amount) / scale,
                          (start.y + (end.y - start.y) * amount) / scale);
    }
    return resampled;
}

static gboolean path_matches(GVariant* template_value, GArray* actual_path, double tolerance) {
    g_autoptr(GArray) expected = read_path(template_value);
    g_autoptr(GArray) actual = resample_normalized_path(actual_path);
    g_autoptr(GArray) normalized_expected = resample_normalized_path(expected);
    if (!normalized_expected || !actual)
        return FALSE;

    double squared_error = 0;
    for (guint i = 0; i < normalized_expected->len; i++) {
        Point expected_point = g_array_index(normalized_expected, Point, i);
        Point actual_point = g_array_index(actual, Point, i);
        squared_error += (expected_point.x - actual_point.x) * (expected_point.x - actual_point.x) +
                         (expected_point.y - actual_point.y) * (expected_point.y - actual_point.y);
    }
    return sqrt(squared_error / normalized_expected->len) <= tolerance;
}

static double gesture_number_or_default(GVariant* gesture, const char* key, double default_value) {
    double value;
    return lookup_number(gesture, key, &value) ? value : default_value;
}

static GVariant* recognize_swipe(GnoblinTouchpadRouter* router) {
    for (guint i = 0; i < router->candidates->len; i++) {
        GVariant* candidate = g_ptr_array_index(router->candidates, i);
        g_autoptr(GVariant) template_value = lookup_value(candidate, "path");
        const double threshold = gesture_number_or_default(candidate, "threshold", 48);
        const double tolerance = gesture_number_or_default(candidate, "tolerance", 0.22);
        if (template_value && router->distance >= threshold &&
            path_matches(template_value, router->path, tolerance))
            return candidate;
    }
    return NULL;
}

static GVariant* recognize_pinch(GnoblinTouchpadRouter* router, double scale) {
    const double delta = scale - router->initial_scale;
    if (fabs(delta) < 0.001)
        return NULL;
    const char* direction = delta > 0 ? "out" : "in";
    for (guint i = 0; i < router->candidates->len; i++) {
        GVariant* candidate = g_ptr_array_index(router->candidates, i);
        g_autofree char* configured_direction = NULL;
        if (lookup_string(candidate, "direction", &configured_direction) &&
            g_str_equal(direction, configured_direction) &&
            fabs(delta) >= gesture_number_or_default(candidate, "threshold", 0.12))
            return candidate;
    }
    return NULL;
}

GnoblinTouchpadRouter* gnoblin_touchpad_router_new(void) {
    return g_new0(GnoblinTouchpadRouter, 1);
}

void gnoblin_touchpad_router_reset(GnoblinTouchpadRouter* router) {
    if (!router)
        return;
    g_clear_pointer(&router->gestures, g_variant_unref);
    g_clear_pointer(&router->candidates, g_ptr_array_unref);
    g_clear_pointer(&router->path, g_array_unref);
    g_clear_pointer(&router->kind, g_free);
    router->x = router->y = router->distance = 0;
    router->initial_scale = 1;
    router->claimed = FALSE;
}

void gnoblin_touchpad_router_free(GnoblinTouchpadRouter* router) {
    if (!router)
        return;
    gnoblin_touchpad_router_reset(router);
    g_free(router);
}

gboolean gnoblin_touchpad_router_handle(GnoblinTouchpadRouter* router, GVariant* gestures,
                                        GVariant* payload, const char* context,
                                        GVariant** matched_gesture) {
    if (matched_gesture)
        *matched_gesture = NULL;
    if (!router || !gestures || !g_variant_is_of_type(gestures, G_VARIANT_TYPE("av")) || !payload ||
        !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT))
        return FALSE;

    g_autofree char* kind = NULL;
    g_autofree char* phase = NULL;
    if (!lookup_string(payload, "gesture", &kind) || !lookup_string(payload, "phase", &phase) ||
        (!g_str_equal(kind, "swipe") && !g_str_equal(kind, "pinch")))
        return FALSE;

    if (router->gestures && !g_variant_equal(router->gestures, gestures))
        gnoblin_touchpad_router_reset(router);

    if (g_str_equal(phase, "begin")) {
        gint64 fingers;
        if (!lookup_integer(payload, "fingers", &fingers))
            return FALSE;
        gnoblin_touchpad_router_reset(router);
        router->gestures = g_variant_ref(gestures);
        router->candidates = g_ptr_array_new_with_free_func((GDestroyNotify)g_variant_unref);
        router->path = g_array_new(FALSE, FALSE, sizeof(Point));
        append_path_point(router->path, 0, 0);
        router->kind = g_strdup(kind);
        router->initial_scale = gesture_number_or_default(payload, "scale", 1);
        for (gsize i = 0; i < g_variant_n_children(gestures); i++) {
            g_autoptr(GVariant) boxed = g_variant_get_child_value(gestures, i);
            g_autoptr(GVariant) gesture = unwrap_variant(g_steal_pointer(&boxed));
            if (gesture_matches_begin(gesture, kind, fingers, context))
                g_ptr_array_add(router->candidates, g_variant_ref(gesture));
        }
        router->claimed = router->candidates->len > 0;
        return router->claimed;
    }

    if (!router->kind || !g_str_equal(router->kind, kind))
        return FALSE;

    GVariant* recognized = NULL;
    if (g_str_equal(kind, "swipe")) {
        double dx = 0, dy = 0;
        lookup_number(payload, "dx", &dx);
        lookup_number(payload, "dy", &dy);
        router->x += dx;
        router->y += dy;
        router->distance += hypot(dx, dy);
        if (dx != 0 || dy != 0)
            append_path_point(router->path, router->x, router->y);
        if (g_str_equal(phase, "end"))
            recognized = recognize_swipe(router);
    } else if (g_str_equal(kind, "pinch") && g_str_equal(phase, "end")) {
        recognized = recognize_pinch(router, gesture_number_or_default(payload, "scale", 1));
    }

    if (g_str_equal(phase, "end")) {
        gboolean claimed = router->claimed;
        if (claimed && recognized && matched_gesture)
            *matched_gesture = g_variant_ref(recognized);
        gnoblin_touchpad_router_reset(router);
        return claimed;
    }
    if (g_str_equal(phase, "cancel")) {
        gboolean claimed = router->claimed;
        gnoblin_touchpad_router_reset(router);
        return claimed;
    }
    return router->claimed;
}
