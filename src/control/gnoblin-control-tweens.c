/* The compositor's tween player: plays a Lua tween's timeline in the render loop.
 *
 * The Lua runtime compiles gnoblin.tween() into one description: tracks with a start time, a
 * duration, a target window, a property, an end value and a curve, and callback times. One
 * ClutterTimeline on the first target's actor drives every track, so frames follow the
 * compositor's frame clock and do not wait on Lua. Each frame writes the tracks that have
 * started, in order, into the window's visual override (gnoblin-control-window-visuals.c), so a
 * later track on the same property wins and the window keeps the end values. A track whose
 * window closes stops; the tween still finishes.
 *
 *   tween.play  {tween_id, duration_ms, tracks, callbacks}   completes when the tween ends
 *   tween.pause / tween.resume / tween.kill {tween_id}
 *   tween.seek  {tween_id, milliseconds}
 *
 * A tween belongs to the runtime generation that started it. A reload, a stopped worker and a
 * session lock cancel it. */
#include "config.h"

#include "compositor/meta-window-actor-private.h"
#include "core/gnoblin-control-internal.h"
#include "core/gnoblin-easing.h"
#include "core/gnoblin-native-control-private.h"

#define TWEEN_MAX_TRACKS 64
#define TWEEN_MAX_CALLBACKS 64
#define TWEEN_MAX_MS (10 * 60 * 1000)

typedef struct {
    MetaWindow* window; /* Weak; NULL once the window is gone. */
    GnoblinVisualProperty property;
    gboolean pair; /* offset: x and y together */
    double from[2];
    double to[2];
    gint64 start_ms;
    gint64 duration_ms;
    GnoblinEase ease;
    GnoblinEaseDirection direction;
    gboolean started;
} TweenTrack;

typedef struct {
    gint64 at_ms;
    gboolean fired;
} TweenCallback;

typedef struct {
    GnoblinNativeControl* control;
    char* id;
    guint64 owner_generation;
    gint64 operation_id;
    gint64 duration_ms;
    ClutterTimeline* timeline;
    GArray* tracks;    /* TweenTrack */
    GArray* callbacks; /* TweenCallback, index = callback number */
} TweenPlayer;

static void tween_free(gpointer data) {
    TweenPlayer* tween = data;
    if (tween->timeline) {
        g_signal_handlers_disconnect_by_data(tween->timeline, tween);
        clutter_timeline_stop(tween->timeline);
        g_clear_object(&tween->timeline);
    }
    for (guint i = 0; tween->tracks && i < tween->tracks->len; i++) {
        TweenTrack* track = &g_array_index(tween->tracks, TweenTrack, i);
        if (track->window)
            g_object_remove_weak_pointer(G_OBJECT(track->window), (gpointer*)&track->window);
    }
    g_clear_pointer(&tween->tracks, g_array_unref);
    g_clear_pointer(&tween->callbacks, g_array_unref);
    g_free(tween->id);
    g_free(tween);
}

static gboolean number_from_variant(GVariant* value, double* number) {
    if (!value)
        return FALSE;
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARIANT)) {
        g_autoptr(GVariant) inner = g_variant_get_variant(value);
        return number_from_variant(inner, number);
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
        *number = g_variant_get_double(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
        *number = (double)g_variant_get_int64(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32))
        *number = g_variant_get_int32(value);
    else
        return FALSE;
    return isfinite(*number);
}

static gboolean lookup_number(GVariant* dict, const char* key, double* number) {
    g_autoptr(GVariant) value = g_variant_lookup_value(dict, key, NULL);
    return number_from_variant(value, number);
}

static void tween_render(TweenPlayer* tween, gint64 elapsed_ms) {
    GnoblinNativeControl* control = tween->control;
    for (guint i = 0; i < tween->tracks->len; i++) {
        TweenTrack* track = &g_array_index(tween->tracks, TweenTrack, i);
        if (!track->window || elapsed_ms < track->start_ms)
            continue;
        double values[GNOBLIN_VISUAL_COUNT];
        gnoblin_control_window_visual_values(track->window, values);
        if (!track->started) {
            /* A track starts from wherever the window is drawn when it begins. */
            track->from[0] = values[track->property];
            if (track->pair)
                track->from[1] = values[track->property + 1];
            track->started = TRUE;
        }
        double progress = track->duration_ms > 0
                              ? (double)(elapsed_ms - track->start_ms) / track->duration_ms
                              : 1.0;
        double eased = gnoblin_ease(track->ease, track->direction, progress);
        for (int part = 0; part < (track->pair ? 2 : 1); part++)
            values[track->property + part] =
                track->from[part] + (track->to[part] - track->from[part]) * eased;
        gnoblin_control_window_visual_store(track->window, values, tween->owner_generation);
    }
    for (guint i = 0; i < tween->callbacks->len; i++) {
        TweenCallback* callback = &g_array_index(tween->callbacks, TweenCallback, i);
        if (callback->fired || elapsed_ms < callback->at_ms)
            continue;
        callback->fired = TRUE;
        GVariantBuilder payload;
        g_variant_builder_init(&payload, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&payload, "{sv}", "tween_id", g_variant_new_string(tween->id));
        g_variant_builder_add(&payload, "{sv}", "index", g_variant_new_int64(i + 1));
        g_autoptr(GVariant) event = g_variant_ref_sink(g_variant_builder_end(&payload));
        native_runtime_dispatch_event(control, "gnoblin.tween.callback", event);
    }
}

/* End the tween with `outcome` ("finished", "killed" or "cancelled") and complete tween.play.
 * This can run inside the timeline's own signal, so the player is freed from an idle. The window
 * state is published first, so a callback that reads the window sees its end values. */
static void tween_finish(TweenPlayer* tween, const char* outcome) {
    GnoblinNativeControl* control = tween->control;
    gint64 operation_id = tween->operation_id;
    g_autofree char* id = g_strdup(tween->id);
    gpointer key = NULL;
    if (!g_hash_table_steal_extended(control->tweens, id, &key, NULL))
        return;
    g_free(key);
    g_signal_handlers_disconnect_by_data(tween->timeline, tween);
    clutter_timeline_stop(tween->timeline);
    g_idle_add_once((GSourceOnceFunc)tween_free, tween);
    if (control->stopping)
        return;
    publish_windows_now(control);
    JsonObject* object = json_object_new();
    json_object_set_string_member(object, "tween_id", id);
    json_object_set_string_member(object, "outcome", outcome);
    g_autoptr(JsonNode) result = json_node_new(JSON_NODE_OBJECT);
    json_node_take_object(result, object);
    gnoblin_control_dispatch_operation_completion_full(control, operation_id, "tween.play", TRUE,
                                                       result, NULL, NULL, FALSE);
}

static gboolean session_locked(GnoblinNativeControl* control) {
    return control->wayland_compositor &&
           meta_wayland_session_lock_is_active(control->wayland_compositor);
}

static void tween_new_frame(ClutterTimeline* timeline, int elapsed_ms, gpointer user_data) {
    TweenPlayer* tween = user_data;
    if (session_locked(tween->control)) {
        tween_finish(tween, "cancelled");
        return;
    }
    tween_render(tween, elapsed_ms);
    /* Every target has closed: there is nothing left to play. */
    gboolean live = FALSE;
    for (guint i = 0; i < tween->tracks->len; i++)
        live |= g_array_index(tween->tracks, TweenTrack, i).window != NULL;
    if (!live)
        tween_finish(tween, "finished");
}

static void tween_completed(ClutterTimeline* timeline, gpointer user_data) {
    TweenPlayer* tween = user_data;
    tween_render(tween, tween->duration_ms);
    tween_finish(tween, "finished");
}

static gboolean parse_track(GnoblinNativeControl* control, GVariant* dict, TweenTrack* track,
                            gint64 duration_ms, GError** error) {
    const char* window_id = NULL;
    const char* property = NULL;
    const char* ease = "linear";
    const char* direction = "in_out";
    double start = 0, length = 0;
    if (!g_variant_is_of_type(dict, G_VARIANT_TYPE_VARDICT) ||
        !g_variant_lookup(dict, "window", "&s", &window_id) ||
        !g_variant_lookup(dict, "property", "&s", &property) ||
        !lookup_number(dict, "start_ms", &start) || !lookup_number(dict, "duration_ms", &length)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "tween track needs window, property, start_ms and duration_ms");
        return FALSE;
    }
    g_variant_lookup(dict, "ease", "&s", &ease);
    g_variant_lookup(dict, "direction", "&s", &direction);
    int ease_index = gnoblin_ease_from_name(ease);
    int direction_index = gnoblin_ease_direction_from_name(direction);
    if (ease_index < 0 || direction_index < 0 || start < 0 || length < 0 ||
        start + length > duration_ms) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "tween track has an unknown curve or times outside the tween");
        return FALSE;
    }
    static const struct {
        const char* name;
        GnoblinVisualProperty property;
        gboolean pair;
        double minimum, maximum;
    } properties[] = {
        {"opacity", GNOBLIN_VISUAL_OPACITY, FALSE, 0.0, 1.0},
        {"scale", GNOBLIN_VISUAL_SCALE, FALSE, 0.01, 10.0},
        {"rotation", GNOBLIN_VISUAL_ROTATION, FALSE, -3600.0, 3600.0},
        {"offset", GNOBLIN_VISUAL_OFFSET_X, TRUE, -100000.0, 100000.0},
    };
    gsize found = G_N_ELEMENTS(properties);
    for (gsize i = 0; i < G_N_ELEMENTS(properties); i++)
        if (g_str_equal(properties[i].name, property))
            found = i;
    if (found == G_N_ELEMENTS(properties)) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "a window has no animatable property %s", property);
        return FALSE;
    }
    track->property = properties[found].property;
    track->pair = properties[found].pair;
    const char* keys[2] = {track->pair ? "to_x" : "to", "to_y"};
    for (int part = 0; part < (track->pair ? 2 : 1); part++) {
        if (!lookup_number(dict, keys[part], &track->to[part]) ||
            track->to[part] < properties[found].minimum ||
            track->to[part] > properties[found].maximum) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "tween %s must end between %g and %g", property, properties[found].minimum,
                        properties[found].maximum);
            return FALSE;
        }
    }
    track->window = native_window_by_stable_id(control, window_id);
    if (!track->window || !meta_window_actor_from_window(track->window)) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND, "tween target %s is unavailable",
                    window_id);
        return FALSE;
    }
    track->start_ms = (gint64)start;
    track->duration_ms = (gint64)length;
    track->ease = ease_index;
    track->direction = direction_index;
    return TRUE;
}

gboolean gnoblin_control_tween_play(GnoblinNativeControl* control, gint64 operation_id,
                                    GVariant* arguments, guint64 owner_generation, GError** error) {
    const char* id = NULL;
    double duration = 0;
    g_autoptr(GVariant) tracks = g_variant_lookup_value(arguments, "tracks", NULL);
    g_autoptr(GVariant) callbacks = g_variant_lookup_value(arguments, "callbacks", NULL);
    if (owner_generation != control->runtime_generation) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "tween.play belongs to an old runtime generation");
        return FALSE;
    }
    if (session_locked(control)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "a tween cannot play while the session is locked");
        return FALSE;
    }
    if (!g_variant_lookup(arguments, "tween_id", "&s", &id) || !*id ||
        !lookup_number(arguments, "duration_ms", &duration) || duration < 1 ||
        duration > TWEEN_MAX_MS || !tracks || !g_variant_is_container(tracks) ||
        g_variant_n_children(tracks) < 1 || g_variant_n_children(tracks) > TWEEN_MAX_TRACKS ||
        (callbacks && (!g_variant_is_container(callbacks) ||
                       g_variant_n_children(callbacks) > TWEEN_MAX_CALLBACKS))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "tween.play needs tween_id, duration_ms from 1 to 600000 and 1 to 64 "
                            "tracks");
        return FALSE;
    }
    if (!control->tweens)
        control->tweens = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, tween_free);
    if (g_hash_table_contains(control->tweens, id)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_EXISTS, "this tween is already playing");
        return FALSE;
    }

    TweenPlayer* tween = g_new0(TweenPlayer, 1);
    tween->control = control;
    tween->id = g_strdup(id);
    tween->owner_generation = owner_generation;
    tween->operation_id = operation_id;
    tween->duration_ms = (gint64)duration;
    /* Sized once, so the weak pointers into the array never move. */
    tween->tracks =
        g_array_sized_new(FALSE, TRUE, sizeof(TweenTrack), (guint)g_variant_n_children(tracks));
    tween->callbacks = g_array_new(FALSE, TRUE, sizeof(TweenCallback));
    for (gsize i = 0; i < g_variant_n_children(tracks); i++) {
        g_autoptr(GVariant) child = g_variant_get_child_value(tracks, i);
        g_autoptr(GVariant) dict = g_variant_is_of_type(child, G_VARIANT_TYPE_VARIANT)
                                       ? g_variant_get_variant(child)
                                       : g_variant_ref(child);
        TweenTrack track = {0};
        if (!parse_track(control, dict, &track, tween->duration_ms, error)) {
            tween_free(tween);
            return FALSE;
        }
        g_array_append_val(tween->tracks, track);
        TweenTrack* stored = &g_array_index(tween->tracks, TweenTrack, tween->tracks->len - 1);
        g_object_add_weak_pointer(G_OBJECT(stored->window), (gpointer*)&stored->window);
    }
    for (gsize i = 0; callbacks && i < g_variant_n_children(callbacks); i++) {
        g_autoptr(GVariant) child = g_variant_get_child_value(callbacks, i);
        TweenCallback callback = {0};
        double at = 0;
        if (!number_from_variant(child, &at) || at < 0 || at > duration) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "tween callback times must be inside the tween");
            tween_free(tween);
            return FALSE;
        }
        callback.at_ms = (gint64)at;
        g_array_append_val(tween->callbacks, callback);
    }

    /* The stage outlives every window, so a closing target cannot stop the clock. */
    TweenTrack* first = &g_array_index(tween->tracks, TweenTrack, 0);
    ClutterActor* stage = CLUTTER_ACTOR(
        clutter_actor_get_stage(CLUTTER_ACTOR(meta_window_actor_from_window(first->window))));
    tween->timeline = clutter_timeline_new_for_actor(stage, (guint)tween->duration_ms);
    g_signal_connect(tween->timeline, "new-frame", G_CALLBACK(tween_new_frame), tween);
    g_signal_connect(tween->timeline, "completed", G_CALLBACK(tween_completed), tween);
    g_hash_table_insert(control->tweens, g_strdup(id), tween);
    tween_render(tween, 0);
    clutter_timeline_start(tween->timeline);
    return TRUE;
}

GVariant* gnoblin_control_tween_control(GnoblinNativeControl* control, const char* method,
                                        GVariant* arguments, guint64 owner_generation,
                                        GError** error) {
    const char* id = NULL;
    TweenPlayer* tween = g_variant_lookup(arguments, "tween_id", "&s", &id) && control->tweens
                             ? g_hash_table_lookup(control->tweens, id)
                             : NULL;
    if (!tween || tween->owner_generation != owner_generation) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND, "the tween is not playing");
        return NULL;
    }
    if (g_str_equal(method, "tween.pause")) {
        clutter_timeline_pause(tween->timeline);
    } else if (g_str_equal(method, "tween.resume")) {
        if (session_locked(control)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                                "a tween cannot play while the session is locked");
            return NULL;
        }
        clutter_timeline_start(tween->timeline);
    } else if (g_str_equal(method, "tween.seek")) {
        double at = 0;
        if (!lookup_number(arguments, "milliseconds", &at) || at < 0 || at > tween->duration_ms) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "tween.seek milliseconds must be inside the tween");
            return NULL;
        }
        clutter_timeline_advance(tween->timeline, (guint)at);
        tween_render(tween, (gint64)at);
    } else if (g_str_equal(method, "tween.kill")) {
        tween_finish(tween, "killed");
    } else {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT, "unknown tween method");
        return NULL;
    }
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "tween_id", g_variant_new_string(id));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

/* End every tween, for a reload, a stopped worker or compositor shutdown. */
void gnoblin_control_tweens_cancel(GnoblinNativeControl* control, const char* outcome) {
    if (!control || !control->tweens)
        return;
    GHashTableIter iter;
    gpointer value;
    g_autoptr(GPtrArray) tweens = g_ptr_array_new();
    g_hash_table_iter_init(&iter, control->tweens);
    while (g_hash_table_iter_next(&iter, NULL, &value))
        g_ptr_array_add(tweens, value);
    for (guint i = 0; i < tweens->len; i++)
        tween_finish(g_ptr_array_index(tweens, i), outcome);
}
