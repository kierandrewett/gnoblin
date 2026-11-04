/* Native animation inspection and preview control for Gnoblin. */
#include "config.h"

#include "core/display-private.h"
#include "core/gnoblin-animation.h"
#include "core/gnoblin-native-control.h"
#include "meta/meta-backend.h"
#include "meta/compositor.h"
#include "meta/meta-context.h"
#include "meta/meta-plugin.h"
#include "meta/meta-window-actor.h"
#include "meta/meta-workspace-manager.h"
#include "meta/prefs.h"
#include "meta/util.h"
#include "meta/window.h"
#include "meta/workspace.h"
#include "wayland/meta-wayland-layer-shell.h"

#include <math.h>
#include <string.h>

#define ANIMATION_ERROR G_IO_ERROR
#define ANIMATION_ID_KEY "gnoblin-animation-previews"

typedef struct {
    char* name;
    char* event;
    guint duration;
    char* ease;
    GVariant* ease_value;
    GVariant* from;
    GVariant* to;
    GVariant* keyframes;
    GVariant* origin;
    GVariant* target;
    gboolean builtin;
} AnimationSpec;

typedef enum {
    ANIMATION_COMPLETION_NONE,
    ANIMATION_COMPLETION_DESTROY,
    ANIMATION_COMPLETION_MINIMIZE,
    ANIMATION_COMPLETION_UNMINIMIZE,
    ANIMATION_COMPLETION_SIZE_CHANGE,
} AnimationCompletion;

typedef struct {
    char* id;
    char* target_type;
    MetaDisplay* display;
    MetaWindow* window;
    ClutterActor* actor;
    ClutterTimeline* timeline;
    AnimationSpec spec;
    double progress;
    gboolean finished;
    gboolean lifecycle;
    MetaPlugin* lifecycle_plugin;
    AnimationCompletion lifecycle_completion;
    gboolean resize_geometry;
    double resize_x, resize_y, resize_scale_x, resize_scale_y;
    float base_x, base_y, base_z;
    double base_sx, base_sy, base_rotation;
    guint8 base_opacity;
    float base_pivot_x, base_pivot_y;
} AnimationPreview;

typedef struct {
    ClutterActor* actor;
    ClutterActor* parent;
    ClutterActor* previous_sibling;
    ClutterActor* next_sibling;
    gboolean was_visible;
} WorkspaceActorState;

typedef struct {
    MetaDisplay* display;
    MetaPlugin* plugin;
    ClutterStage* stage;
    ClutterActor* outgoing_group;
    ClutterActor* incoming_group;
    ClutterTimeline* timeline;
    GPtrArray* actors;
    AnimationSpec spec;
    MetaMotionDirection direction;
    char* from_workspace_id;
    char* to_workspace_id;
    double width, height;
    gboolean finished;
} WorkspaceSwitch;

typedef struct {
    GHashTable* previews;
    WorkspaceSwitch* workspace_switch;
    guint lock_watch_id;
    gulong shutdown_handler_id;
    MetaBackend* backend;
} AnimationState;

typedef struct {
    MetaPlugin* plugin;
    MtkRectangle old_buffer_rect;
} PendingResize;

#define RESIZE_PENDING_KEY "gnoblin-animation-pending-resize"

typedef struct {
    const char* name;
    const char* event;
} BuiltinAnimation;

static const BuiltinAnimation builtins[] = {{"gnome-minimize", "minimize"},
                                            {"gnome-restore", "restore"},
                                            {"gnome-open", "open"},
                                            {"gnome-close", "close"},
                                            {"gnome-dialog-open", "dialog-open"},
                                            {"gnome-dialog-close", "dialog-close"},
                                            {"gnome-workspace-switch", "workspace-switch"},
                                            {"gnoblin-layer-open", "layer-open"},
                                            {"gnoblin-layer-close", "layer-close"},
                                            {"gnome", NULL},
                                            {"zoom", NULL},
                                            {"fade", NULL},
                                            {"slide", NULL},
                                            {"none", NULL},
                                            {"gnome-resize", "resize"},
                                            {"gnoblin-shadow-change", "shadow-change"},
                                            {NULL, NULL}};

static void animation_spec_clear(AnimationSpec* spec);
static void preview_stop(AnimationPreview* preview, gboolean restore, gboolean report);
static void preview_new_frame(ClutterTimeline* timeline, guint elapsed, gpointer user_data);
static void start_window_open_animation(MetaWindowActor* window_actor);
static void frame_add_number(GVariantBuilder* builder, const char* key, double value);
static void stop_lock_watch(AnimationState* state);
static gboolean is_locked(MetaDisplay* display);
static GVariant* animation_spec_variant(AnimationSpec* spec);
static void workspace_switch_stop(WorkspaceSwitch* workspace_switch, gboolean complete,
                                  gboolean cancelled);
static void workspace_switch_stop_for_display(MetaDisplay* display, gboolean complete);
static void workspace_switch_shutdown(MetaBackend* backend, MetaDisplay* display);

static void animation_spec_clear(AnimationSpec* spec) {
    g_free(spec->name);
    g_free(spec->event);
    g_free(spec->ease);
    g_clear_pointer(&spec->ease_value, g_variant_unref);
    g_clear_pointer(&spec->from, g_variant_unref);
    g_clear_pointer(&spec->to, g_variant_unref);
    g_clear_pointer(&spec->keyframes, g_variant_unref);
    g_clear_pointer(&spec->origin, g_variant_unref);
    g_clear_pointer(&spec->target, g_variant_unref);
    memset(spec, 0, sizeof *spec);
}

static void preview_free(gpointer data) {
    AnimationPreview* preview = data;

    if (!preview->finished)
        preview_stop(preview, TRUE, FALSE);
    animation_spec_clear(&preview->spec);
    g_clear_object(&preview->timeline);
    g_clear_object(&preview->window);
    g_clear_object(&preview->lifecycle_plugin);
    g_free(preview->id);
    g_free(preview->target_type);
    g_free(preview);
}

static void animation_state_free(gpointer data) {
    AnimationState* state = data;
    stop_lock_watch(state);
    if (state->workspace_switch)
        workspace_switch_stop(state->workspace_switch, TRUE, TRUE);
    if (state->backend && state->shutdown_handler_id)
        g_signal_handler_disconnect(state->backend, state->shutdown_handler_id);
    g_clear_object(&state->backend);
    while (g_hash_table_size(state->previews) > 0) {
        GHashTableIter iter;
        gpointer value;
        g_hash_table_iter_init(&iter, state->previews);
        if (!g_hash_table_iter_next(&iter, NULL, &value))
            break;
        preview_stop(value, TRUE, FALSE);
    }
    g_hash_table_unref(state->previews);
    g_free(state);
}

static AnimationState* get_state(MetaDisplay* display) {
    AnimationState* state = g_object_get_data(G_OBJECT(display), ANIMATION_ID_KEY);

    if (!state) {
        state = g_new0(AnimationState, 1);
        state->previews = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, preview_free);
        state->backend = g_object_ref(meta_context_get_backend(meta_display_get_context(display)));
        state->shutdown_handler_id = g_signal_connect(
            state->backend, "prepare-shutdown", G_CALLBACK(workspace_switch_shutdown), display);
        g_object_set_data_full(G_OBJECT(display), ANIMATION_ID_KEY, state, animation_state_free);
    }
    return state;
}

static gboolean lock_watch_cb(gpointer user_data) {
    MetaDisplay* display = user_data;
    AnimationState* state = g_object_get_data(G_OBJECT(display), ANIMATION_ID_KEY);

    if (!state || (g_hash_table_size(state->previews) == 0 && !state->workspace_switch)) {
        if (state)
            state->lock_watch_id = 0;
        return G_SOURCE_REMOVE;
    }
    if (!is_locked(display))
        return G_SOURCE_CONTINUE;

    state->lock_watch_id = 0;
    while (g_hash_table_size(state->previews) > 0) {
        GHashTableIter iter;
        gpointer value;
        g_hash_table_iter_init(&iter, state->previews);
        if (!g_hash_table_iter_next(&iter, NULL, &value))
            break;
        preview_stop(value, TRUE, TRUE);
    }
    workspace_switch_stop_for_display(display, TRUE);
    return G_SOURCE_REMOVE;
}

static void start_lock_watch(MetaDisplay* display, AnimationState* state) {
    if (state->lock_watch_id == 0)
        state->lock_watch_id =
            g_timeout_add_full(G_PRIORITY_HIGH, 50, lock_watch_cb, display, NULL);
}

static void stop_lock_watch(AnimationState* state) {
    if (state->lock_watch_id != 0) {
        g_source_remove(state->lock_watch_id);
        state->lock_watch_id = 0;
    }
}

static void set_error(GError** error, GIOErrorEnum code, const char* message) {
    g_set_error_literal(error, ANIMATION_ERROR, code, message);
}

static gboolean is_locked(MetaDisplay* display) {
    MetaBackend* backend = meta_context_get_backend(meta_display_get_context(display));
    return meta_backend_get_gnoblin_session_lock_active(backend);
}

static GVariant* empty_dict(void) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

static GVariant* lookup_field(GVariant* dict, const char* key) {
    return dict ? g_variant_lookup_value(dict, key, NULL) : NULL;
}

static gboolean only_keys(GVariant* dict, const char* const* allowed, GError** error) {
    GVariantIter iter;
    const char* key;
    GVariant* value;

    if (!dict || !g_variant_is_of_type(dict, G_VARIANT_TYPE_VARDICT)) {
        set_error(error, G_IO_ERROR_INVALID_ARGUMENT, "animation arguments must be a dictionary");
        return FALSE;
    }
    g_variant_iter_init(&iter, dict);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        gboolean found = g_strv_contains(allowed, key);
        g_variant_unref(value);
        if (!found) {
            g_set_error(error, ANIMATION_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "unknown animation argument '%s'", key);
            return FALSE;
        }
    }
    return TRUE;
}

static gboolean get_required_string(GVariant* dict, const char* key, const char** value,
                                    GError** error) {
    if (!g_variant_lookup(dict, key, "&s", value) || !**value) {
        g_set_error(error, ANIMATION_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "animation requires a nonempty '%s' string", key);
        return FALSE;
    }
    return TRUE;
}

static gboolean read_number(GVariant* dict, const char* key, double* result);

static guint32 lookup_duration(GVariant* dict) {
    double value = 0;
    return read_number(dict, "duration", &value) && value >= 0 && value <= 10000 ? (guint32)value
                                                                                 : 0;
}

static GVariant* config_animations(void) {
    g_autoptr(GVariant) document = gnoblin_native_control_get_config_document(NULL);
    return document ? g_variant_lookup_value(document, "animations", NULL) : NULL;
}

static GVariant* find_custom_animation(const char* name, const char* event) {
    g_autoptr(GVariant) animations = config_animations();
    if (!animations || !g_variant_is_container(animations))
        return NULL;

    GVariantIter iter;
    GVariant* candidate;
    g_variant_iter_init(&iter, animations);
    while ((candidate = g_variant_iter_next_value(&iter))) {
        if (g_variant_is_of_type(candidate, G_VARIANT_TYPE_VARIANT)) {
            GVariant* unboxed = g_variant_get_variant(candidate);
            g_variant_unref(candidate);
            candidate = unboxed;
        }
        const char *candidate_name = NULL, *candidate_event = NULL;
        gboolean enabled = TRUE;
        g_variant_lookup(candidate, "name", "&s", &candidate_name);
        g_variant_lookup(candidate, "event", "&s", &candidate_event);
        g_variant_lookup(candidate, "enable", "b", &enabled);
        if (g_strcmp0(candidate_name, name) == 0 &&
            (!event || g_strcmp0(candidate_event, event) == 0))
            return candidate;
        g_variant_unref(candidate);
    }
    return NULL;
}

static const BuiltinAnimation* find_builtin(const char* name) {
    for (guint i = 0; builtins[i].name; i++)
        if (g_str_equal(builtins[i].name, name))
            return &builtins[i];
    return NULL;
}

static GVariant* make_info(const char* name, const char* event, guint duration, const char* ease,
                           gboolean builtin, gboolean previewable) {
    GVariantBuilder info;
    g_variant_builder_init(&info, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&info, "{sv}", "name", g_variant_new_string(name));
    g_variant_builder_add(&info, "{sv}", "enable", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&info, "{sv}", "event", g_variant_new_string(event ? event : ""));
    g_variant_builder_add(&info, "{sv}", "duration", g_variant_new_uint32(duration));
    g_variant_builder_add(&info, "{sv}", "ease", g_variant_new_string(ease ? ease : "linear"));
    g_variant_builder_add(&info, "{sv}", "builtin", g_variant_new_boolean(builtin));
    g_variant_builder_add(&info, "{sv}", "previewable", g_variant_new_boolean(previewable));
    g_variant_builder_add(&info, "{sv}", "from", empty_dict());
    g_variant_builder_add(&info, "{sv}", "to", empty_dict());
    g_variant_builder_add(&info, "{sv}", "origin", g_variant_new_string("center"));
    g_variant_builder_add(&info, "{sv}", "target", g_variant_new_string("none"));
    g_variant_builder_add(&info, "{sv}", "revision",
                          g_variant_new_uint64(gnoblin_native_control_get_config_revision(NULL)));
    return g_variant_ref_sink(g_variant_builder_end(&info));
}

static gboolean builtin_previewable(const char* name) {
    return g_str_equal(name, "gnome") || g_str_equal(name, "gnome-minimize") ||
           g_str_equal(name, "gnome-restore") || g_str_equal(name, "fade") ||
           g_str_equal(name, "zoom") || g_str_equal(name, "slide") ||
           g_str_equal(name, "gnome-open") || g_str_equal(name, "gnome-close") ||
           g_str_equal(name, "gnome-dialog-open") || g_str_equal(name, "gnome-dialog-close") ||
           g_str_equal(name, "gnome-workspace-switch") || g_str_equal(name, "gnome-resize");
}

static gboolean builtin_supports_event(const char* name, const char* event) {
    static const char* const gnome_events[] = {
        "minimize",     "restore",    "open",        "close",  "dialog-open",
        "dialog-close", "layer-open", "layer-close", "resize", NULL};
    static const char* const fade_events[] = {"minimize",   "restore",     "open",
                                              "close",      "dialog-open", "dialog-close",
                                              "layer-open", "layer-close", NULL};
    static const char* const slide_events[] = {"layer-open", "layer-close", NULL};
    if (!event)
        return FALSE;
    if (g_str_equal(name, "gnome") || g_str_equal(name, "fade"))
        return g_strv_contains(g_str_equal(name, "fade") ? fade_events : gnome_events, event);
    if (g_str_equal(name, "slide"))
        return g_strv_contains(slide_events, event);
    const BuiltinAnimation* builtin = find_builtin(name);
    return builtin && builtin->event && g_str_equal(builtin->event, event) &&
           builtin_previewable(name);
}

static void builtin_event_defaults(const char* event, guint* duration, const char** ease) {
    *duration = 150;
    *ease = "ease-out-expo";
    if (g_str_equal(event, "minimize") || g_str_equal(event, "restore"))
        *duration = 400;
    else if (g_str_equal(event, "close"))
        *ease = "ease-out-quad";
    else if (g_str_equal(event, "dialog-open") || g_str_equal(event, "dialog-close")) {
        *duration = 100;
        *ease = "ease-out-quad";
    } else if (g_str_equal(event, "layer-open") || g_str_equal(event, "layer-close")) {
        *duration = 250;
        *ease = "ease-out-cubic";
    } else if (g_str_equal(event, "workspace-switch")) {
        *duration = 250;
        *ease = "ease-out-cubic";
    } else if (g_str_equal(event, "resize")) {
        *duration = 250;
        *ease = "ease-out-quad";
    }
}

static gboolean actor_event(const char* event) {
    static const char* const names[] = {"minimize",   "restore",     "open",
                                        "close",      "dialog-open", "dialog-close",
                                        "layer-open", "layer-close", "workspace-switch",
                                        "resize",     NULL};
    return g_strv_contains(names, event);
}

static gboolean internal_event(const char* event) {
    static const char* const names[] = {"shadow-change", NULL};
    return g_strv_contains(names, event);
}

static GVariant* animation_list(GError** error) {
    GVariantBuilder entries, result;
    (void)error;
    g_autoptr(GVariant) animations = config_animations();

    g_variant_builder_init(&entries, G_VARIANT_TYPE("aa{sv}"));
    for (guint i = 0; builtins[i].name; i++) {
        g_autoptr(GVariant) custom = find_custom_animation(builtins[i].name, NULL);
        if (custom)
            continue;
        const char* event = builtins[i].event;
        guint duration = 0;
        const char* ease = "linear";
        if (event && builtin_previewable(builtins[i].name))
            builtin_event_defaults(event, &duration, &ease);
        else if (g_str_equal(builtins[i].name, "zoom"))
            builtin_event_defaults("minimize", &duration, &ease);
        else if (g_str_equal(builtins[i].name, "fade"))
            builtin_event_defaults("open", &duration, &ease);
        else if (g_str_equal(builtins[i].name, "slide"))
            builtin_event_defaults("layer-open", &duration, &ease);
        else if (g_str_equal(builtins[i].name, "gnome"))
            builtin_event_defaults("open", &duration, &ease);
        g_variant_builder_add_value(&entries,
                                    make_info(builtins[i].name, event, duration, ease, TRUE,
                                              builtin_previewable(builtins[i].name) &&
                                                  !internal_event(event ? event : "")));
    }
    if (animations && g_variant_is_container(animations)) {
        GVariantIter iter;
        GVariant* candidate;
        g_variant_iter_init(&iter, animations);
        while ((candidate = g_variant_iter_next_value(&iter))) {
            if (g_variant_is_of_type(candidate, G_VARIANT_TYPE_VARIANT)) {
                GVariant* unboxed = g_variant_get_variant(candidate);
                g_variant_unref(candidate);
                candidate = unboxed;
            }
            const char *name = NULL, *event = NULL, *ease = NULL;
            guint32 duration;
            gboolean enabled = TRUE;
            g_variant_lookup(candidate, "name", "&s", &name);
            g_variant_lookup(candidate, "event", "&s", &event);
            g_autoptr(GVariant) duration_value = lookup_field(candidate, "duration");
            if (!duration_value) {
                const char* default_ease;
                builtin_event_defaults(event ?: "open", &duration, &default_ease);
            } else
                duration = lookup_duration(candidate);
            g_variant_lookup(candidate, "ease", "&s", &ease);
            if (!ease) {
                guint ignored_duration;
                builtin_event_defaults(event ?: "open", &ignored_duration, &ease);
            }
            g_variant_lookup(candidate, "enable", "b", &enabled);
            if (name && event && enabled) {
                GVariantBuilder record;
                GVariantIter fields;
                const char* field;
                GVariant* field_value;
                g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
                g_variant_iter_init(&fields, candidate);
                while (g_variant_iter_next(&fields, "{&sv}", &field, &field_value))
                    g_variant_builder_add(&record, "{sv}", field, field_value);
                g_variant_builder_add(&record, "{sv}", "builtin", g_variant_new_boolean(FALSE));
                g_variant_builder_add(&record, "{sv}", "previewable",
                                      g_variant_new_boolean(enabled && duration > 0 &&
                                                            actor_event(event) &&
                                                            !internal_event(event)));
                g_variant_builder_add(
                    &record, "{sv}", "revision",
                    g_variant_new_uint64(gnoblin_native_control_get_config_revision(NULL)));
                g_variant_builder_add_value(&entries, g_variant_builder_end(&record));
            }
            g_variant_unref(candidate);
        }
    }
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "animations", g_variant_builder_end(&entries));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static MetaWindow* find_window(MetaDisplay* display, const char* target, const char* target_type,
                               GError** error) {
    MetaWindow* found = NULL;
    guint matches = 0;
    g_autoptr(GList) windows = meta_display_list_all_windows(display);

    if (g_str_equal(target_type, "window") && g_str_equal(target, "active")) {
        found = meta_display_get_focus_window(display);
        if (found)
            return found;
        set_error(error, G_IO_ERROR_NOT_FOUND, "there is no active window to animate");
        return NULL;
    }
    for (GList* item = windows; item; item = item->next) {
        MetaWindow* window = item->data;
        g_autofree char* id = g_strdup_printf("%u", meta_window_get_stable_sequence(window));
        const char* namespace = meta_gnoblin_layer_namespace(window);
        gboolean match = FALSE;

        if (g_str_equal(target_type, "window"))
            match = g_str_equal(target, id);
        else if (g_str_equal(target_type, "layer"))
            match = namespace && g_str_equal(target, id);
        else if (g_str_equal(target_type, "namespace"))
            match = namespace && g_str_equal(target, namespace);
        if (match) {
            found = window;
            matches++;
        }
    }
    if (matches == 1)
        return found;
    set_error(error, G_IO_ERROR_NOT_FOUND,
              matches ? "animation target is ambiguous; use a stable layer ID"
                      : "animation target window no longer exists");
    return NULL;
}

static gboolean animation_target_type_valid(const char* type) {
    return g_str_equal(type, "window") || g_str_equal(type, "layer") ||
           g_str_equal(type, "namespace");
}

static void animation_layer_offset(MetaWindow* window, double* x, double* y) {
    MtkRectangle frame, monitor;
    int anchor = meta_gnoblin_layer_anchor(window);
    int monitor_index = meta_window_get_monitor(window);
    *x = *y = 0;
    if (anchor < 0 || monitor_index < 0 ||
        monitor_index >= meta_display_get_n_monitors(meta_window_get_display(window)))
        return;
    meta_window_get_frame_rect(window, &frame);
    meta_display_get_monitor_geometry(meta_window_get_display(window), monitor_index, &monitor);
    gboolean top = anchor & 1;
    gboolean bottom = anchor & 2;
    gboolean left = anchor & 4;
    gboolean right = anchor & 8;
    *x = left == right ? 0
         : left        ? monitor.x - frame.x - frame.width
                       : monitor.x + monitor.width - frame.x;
    *y = top == bottom ? 0
         : top         ? monitor.y - frame.y - frame.height
                       : monitor.y + monitor.height - frame.y;
}

static gboolean parse_spec(AnimationSpec* spec, const char* name, const char* event,
                           MetaWindow* window, GError** error) {
    GVariant* custom = find_custom_animation(name, event);
    const BuiltinAnimation* builtin = find_builtin(name);
    if (!custom && !builtin) {
        g_set_error(error, ANIMATION_ERROR, G_IO_ERROR_NOT_FOUND, "animation not found: %s", name);
        return FALSE;
    }
    if (!event && custom)
        g_variant_lookup(custom, "event", "&s", &event);
    if (!event && builtin && !builtin->event)
        event = "open";
    if (!event)
        event = builtin ? builtin->event : NULL;
    if (!custom && !builtin_previewable(name) && !g_str_equal(name, "gnome")) {
        g_set_error(error, ANIMATION_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                    "animation '%s' needs context supplied by a lifecycle handler", name);
        return FALSE;
    }
    if (!custom && !builtin_supports_event(name, event)) {
        g_set_error(error, ANIMATION_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "animation '%s' does not support the '%s' event", name, event ?: "");
        return FALSE;
    }
    if (!custom && g_str_equal(name, "zoom") &&
        (g_str_equal(event, "minimize") || g_str_equal(event, "restore"))) {
        g_set_error_literal(error, ANIMATION_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "zoom minimize/restore previews need the shell-owned "
                            "dock target geometry");
        return FALSE;
    }
    spec->name = g_strdup(name);
    spec->event = g_strdup(event ? event : (custom ? NULL : builtin->event));
    spec->builtin = builtin != NULL;
    if (custom) {
        gboolean enabled = TRUE;
        const char* custom_event = NULL;
        if (!event && g_variant_lookup(custom, "event", "&s", &custom_event))
            spec->event = g_strdup(custom_event);
        guint32 duration;
        g_autoptr(GVariant) duration_value = lookup_field(custom, "duration");
        if (!duration_value) {
            const char* ignored_ease;
            builtin_event_defaults(spec->event ?: "open", &duration, &ignored_ease);
        } else
            duration = lookup_duration(custom);
        const char* ease = NULL;
        g_variant_lookup(custom, "enable", "b", &enabled);
        if (!enabled) {
            g_variant_unref(custom);
            g_set_error(error, ANIMATION_ERROR, G_IO_ERROR_NOT_FOUND, "animation is disabled: %s",
                        name);
            return FALSE;
        }
        spec->ease_value = lookup_field(custom, "ease");
        if (spec->ease_value && g_variant_is_of_type(spec->ease_value, G_VARIANT_TYPE_STRING))
            ease = g_variant_get_string(spec->ease_value, NULL);
        else if (spec->ease_value && g_variant_is_of_type(spec->ease_value, G_VARIANT_TYPE_VARDICT))
            ease = "cubic-bezier";
        spec->duration = duration;
        if (!ease) {
            guint ignored_duration;
            builtin_event_defaults(spec->event ?: "open", &ignored_duration, &ease);
        }
        spec->ease = g_strdup(ease);
        spec->from = lookup_field(custom, "from");
        spec->to = lookup_field(custom, "to");
        spec->keyframes = lookup_field(custom, "keyframes");
        spec->origin = lookup_field(custom, "origin");
        spec->target = lookup_field(custom, "target");
        g_variant_unref(custom);
    } else {
        gboolean entering =
            g_str_equal(spec->event, "open") || g_str_equal(spec->event, "dialog-open") ||
            g_str_equal(spec->event, "restore") || g_str_equal(spec->event, "layer-open");
        const char* default_ease;
        builtin_event_defaults(spec->event, &spec->duration, &default_ease);
        if (g_str_equal(name, "gnome-open"))
            spec->ease = g_strdup("ease-out-expo");
        else if (g_str_equal(name, "gnome-close") || g_str_equal(name, "gnome-dialog-open") ||
                 g_str_equal(name, "gnome-dialog-close"))
            spec->ease = g_strdup("ease-out-quad");
        else
            spec->ease = g_strdup(default_ease);
        spec->ease_value = g_variant_ref_sink(g_variant_new_string(spec->ease));
        spec->target = g_variant_ref_sink(g_variant_new_string("none"));
        if (!spec->origin)
            spec->origin = g_variant_ref_sink(g_variant_new_string("center"));
        GVariantBuilder from, to;
        g_variant_builder_init(&from, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_init(&to, G_VARIANT_TYPE_VARDICT);
        if (g_str_equal(spec->event, "workspace-switch")) {
            frame_add_number(&from, "progress", 0);
            frame_add_number(&to, "progress", 1);
        } else if (g_str_equal(name, "fade")) {
            frame_add_number(&from, "opacity", entering ? 0 : 1);
            frame_add_number(&to, "opacity", entering ? 1 : 0);
        } else if (g_str_equal(name, "slide")) {
            double offset_x, offset_y;
            animation_layer_offset(window, &offset_x, &offset_y);
            gboolean opening = g_str_equal(spec->event, "layer-open");
            gboolean fade = offset_x == 0 && offset_y == 0;
            frame_add_number(&from, "x", opening ? offset_x : 0);
            frame_add_number(&from, "y", opening ? offset_y : 0);
            frame_add_number(&to, "x", opening ? 0 : offset_x);
            frame_add_number(&to, "y", opening ? 0 : offset_y);
            frame_add_number(&from, "opacity", opening && fade ? 0 : 1);
            frame_add_number(&to, "opacity", !opening && fade ? 0 : 1);
            g_clear_pointer(&spec->target, g_variant_unref);
            spec->target = g_variant_ref_sink(g_variant_new_string("monitor"));
        } else if (g_str_equal(name, "gnome-open") ||
                   (g_str_equal(name, "gnome") && g_str_equal(spec->event, "open"))) {
            g_clear_pointer(&spec->origin, g_variant_unref);
            spec->origin = g_variant_ref_sink(g_variant_new_string("bottom-center"));
            frame_add_number(&from, "scale_x", .01);
            frame_add_number(&from, "scale_y", .05);
            frame_add_number(&from, "opacity", 0);
            frame_add_number(&to, "scale_x", 1);
            frame_add_number(&to, "scale_y", 1);
            frame_add_number(&to, "opacity", 1);
        } else if (g_str_equal(name, "gnome-close") ||
                   (g_str_equal(name, "gnome") && g_str_equal(spec->event, "close"))) {
            frame_add_number(&from, "scale_x", 1);
            frame_add_number(&from, "scale_y", 1);
            frame_add_number(&from, "opacity", 1);
            frame_add_number(&to, "scale_x", .8);
            frame_add_number(&to, "scale_y", .8);
            frame_add_number(&to, "opacity", 0);
        } else if (g_str_equal(name, "gnome-resize") ||
                   (g_str_equal(name, "gnome") && g_str_equal(spec->event, "resize"))) {
            g_clear_pointer(&spec->origin, g_variant_unref);
            spec->origin = g_variant_ref_sink(g_variant_new_string("top-left"));
        } else if (g_str_equal(name, "gnome-dialog-open") ||
                   g_str_equal(name, "gnome-dialog-close") ||
                   (g_str_equal(name, "gnome") && (g_str_equal(spec->event, "dialog-open") ||
                                                   g_str_equal(spec->event, "dialog-close")))) {
            gboolean opening = g_str_equal(spec->event, "dialog-open");
            frame_add_number(&from, "scale_y", opening ? 0 : 1);
            frame_add_number(&from, "opacity", opening ? 0 : 1);
            frame_add_number(&to, "scale_x", 1);
            frame_add_number(&to, "scale_y", opening ? 1 : 0);
            if (opening)
                frame_add_number(&to, "opacity", 1);
        } else if (g_str_equal(spec->event, "layer-open") ||
                   g_str_equal(spec->event, "layer-close")) {
            double offset_x, offset_y;
            animation_layer_offset(window, &offset_x, &offset_y);
            gboolean opening = g_str_equal(spec->event, "layer-open");
            gboolean fade = offset_x == 0 && offset_y == 0;
            frame_add_number(&from, "x", opening ? offset_x : 0);
            frame_add_number(&from, "y", opening ? offset_y : 0);
            frame_add_number(&to, "x", opening ? 0 : offset_x);
            frame_add_number(&to, "y", opening ? 0 : offset_y);
            frame_add_number(&from, "opacity", opening && fade ? 0 : 1);
            frame_add_number(&to, "opacity", !opening && fade ? 0 : 1);
            g_clear_pointer(&spec->target, g_variant_unref);
            spec->target = g_variant_ref_sink(g_variant_new_string("monitor"));
        } else if (g_str_equal(spec->event, "minimize") || g_str_equal(spec->event, "restore")) {
            gboolean minimizing = g_str_equal(spec->event, "minimize");
            MtkRectangle frame, destination, monitor;
            gboolean has_destination = meta_window_get_icon_geometry(window, &destination);
            meta_window_get_frame_rect(window, &frame);
            if (meta_window_is_monitor_sized(window)) {
                frame_add_number(&from, "opacity", minimizing ? 1 : 0);
                frame_add_number(&to, "opacity", minimizing ? 0 : 1);
            } else {
                double x, y, sx, sy;
                if (has_destination && frame.width > 0 && frame.height > 0) {
                    x = destination.x - frame.x;
                    y = destination.y - frame.y;
                    sx = destination.width / (double)frame.width;
                    sy = destination.height / (double)frame.height;
                    g_clear_pointer(&spec->target, g_variant_unref);
                    spec->target = g_variant_ref_sink(g_variant_new_string("icon"));
                } else {
                    int monitor_index = meta_window_get_monitor(window);
                    meta_display_get_monitor_geometry(meta_window_get_display(window),
                                                      monitor_index, &monitor);
                    x = monitor.x - frame.x +
                        (clutter_get_text_direction() == CLUTTER_TEXT_DIRECTION_RTL ? monitor.width
                                                                                    : 0);
                    y = monitor.y - frame.y;
                    sx = sy = 0;
                    g_clear_pointer(&spec->target, g_variant_unref);
                    spec->target = g_variant_ref_sink(g_variant_new_string("monitor"));
                }
                frame_add_number(&from, "x", minimizing ? 0 : x);
                frame_add_number(&from, "y", minimizing ? 0 : y);
                frame_add_number(&from, "scale_x", minimizing ? 1 : sx);
                frame_add_number(&from, "scale_y", minimizing ? 1 : sy);
                frame_add_number(&from, "opacity", 1);
                frame_add_number(&to, "x", minimizing ? x : 0);
                frame_add_number(&to, "y", minimizing ? y : 0);
                frame_add_number(&to, "scale_x", minimizing ? sx : 1);
                frame_add_number(&to, "scale_y", minimizing ? sy : 1);
                frame_add_number(&to, "opacity", minimizing ? 0 : 1);
            }
            g_clear_pointer(&spec->origin, g_variant_unref);
            spec->origin = g_variant_ref_sink(g_variant_new_string("top-left"));
        } else {
            double start_scale = entering ? .92 : 1, end_scale = entering ? 1 : .92;
            frame_add_number(&from, "scale", start_scale);
            frame_add_number(&to, "scale", end_scale);
            frame_add_number(&from, "opacity", entering ? 0 : 1);
            frame_add_number(&to, "opacity", entering ? 1 : 0);
        }
        spec->from = g_variant_ref_sink(g_variant_builder_end(&from));
        spec->to = g_variant_ref_sink(g_variant_builder_end(&to));
    }
    if (!spec->from)
        spec->from = empty_dict();
    if (!spec->to)
        spec->to = empty_dict();
    if (!spec->origin)
        spec->origin = g_variant_ref_sink(
            g_variant_new_string(g_str_equal(spec->event, "resize") ? "top-left" : "center"));
    if (!spec->target)
        spec->target = g_variant_ref_sink(g_variant_new_string("none"));
    if (g_str_equal(spec->event, "workspace-switch")) {
        double from_progress, to_progress;
        if (!read_number(spec->from, "progress", &from_progress) &&
            !read_number(spec->to, "progress", &to_progress)) {
            GVariantBuilder from, to;
            g_variant_builder_init(&from, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_init(&to, G_VARIANT_TYPE_VARDICT);
            frame_add_number(&from, "progress", 0);
            frame_add_number(&to, "progress", 1);
            g_clear_pointer(&spec->from, g_variant_unref);
            g_clear_pointer(&spec->to, g_variant_unref);
            spec->from = g_variant_ref_sink(g_variant_builder_end(&from));
            spec->to = g_variant_ref_sink(g_variant_builder_end(&to));
        }
    }
    return TRUE;
}

static gboolean read_number(GVariant* dict, const char* key, double* result) {
    g_autoptr(GVariant) value = lookup_field(dict, key);
    if (!value)
        return FALSE;
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
        *result = g_variant_get_double(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32))
        *result = g_variant_get_int32(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_UINT32))
        *result = g_variant_get_uint32(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
        *result = g_variant_get_int64(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_UINT64))
        *result = g_variant_get_uint64(value);
    else
        return FALSE;
    return isfinite(*result);
}

static double cubic_bezier(double t, double x1, double y1, double x2, double y2) {
    double lo = 0, hi = 1, u = t;
    for (guint i = 0; i < 24; i++) {
        double x = 3 * (1 - u) * (1 - u) * u * x1 + 3 * (1 - u) * u * u * x2 + u * u * u;
        if (fabs(x - t) < 1e-5)
            break;
        if (x < t)
            lo = u;
        else
            hi = u;
        u = (lo + hi) / 2;
    }
    return 3 * (1 - u) * (1 - u) * u * y1 + 3 * (1 - u) * u * u * y2 + u * u * u;
}

static double ease_progress(const char* ease, double t) {
    t = CLAMP(t, 0, 1);
    if (g_strcmp0(ease, "ease-in-quad") == 0)
        return t * t;
    if (g_strcmp0(ease, "ease-out-quad") == 0)
        return 1 - (1 - t) * (1 - t);
    if (g_strcmp0(ease, "ease-in-cubic") == 0)
        return t * t * t;
    if (g_strcmp0(ease, "ease-out-cubic") == 0)
        return 1 - (1 - t) * (1 - t) * (1 - t);
    if (g_strcmp0(ease, "ease-in-out-cubic") == 0)
        return t < .5 ? 4 * t * t * t : 1 - pow(-2 * t + 2, 3) / 2;
    if (g_strcmp0(ease, "ease-out-expo") == 0)
        return t == 1 ? 1 : 1 - pow(2, -10 * t);
    if (g_strcmp0(ease, "ease-out-back") == 0) {
        double c1 = 1.70158, c3 = c1 + 1;
        return 1 + c3 * pow(t - 1, 3) + c1 * pow(t - 1, 2);
    }
    return t;
}

static double ease_progress_variant(GVariant* ease, const char* fallback, double t) {
    if (ease && g_variant_is_of_type(ease, G_VARIANT_TYPE_STRING))
        return ease_progress(g_variant_get_string(ease, NULL), t);
    if (ease && g_variant_is_of_type(ease, G_VARIANT_TYPE_VARDICT)) {
        const char* type = NULL;
        double x1, y1, x2, y2;
        if (g_variant_lookup(ease, "type", "&s", &type) && g_str_equal(type, "cubic-bezier") &&
            read_number(ease, "x1", &x1) && read_number(ease, "y1", &y1) &&
            read_number(ease, "x2", &x2) && read_number(ease, "y2", &y2) && x1 >= 0 && x1 <= 1 &&
            x2 >= 0 && x2 <= 1)
            return cubic_bezier(CLAMP(t, 0, 1), x1, y1, x2, y2);
    }
    return ease_progress(fallback, t);
}

static void frame_add_number(GVariantBuilder* builder, const char* key, double value) {
    g_variant_builder_add(builder, "{sv}", key, g_variant_new_double(value));
}

static void sample_property(AnimationSpec* spec, const char* key, double progress, double base,
                            double* sampled) {
    double start = base, end = base;
    gboolean has_start = read_number(spec->from, key, &start);
    gboolean has_end = read_number(spec->to, key, &end);
    double left_at = 0, right_at = 1;
    g_autoptr(GVariant) frame_ease = NULL;
    GVariant* right_ease = spec->ease_value;
    if (!has_start && !has_end) {
        *sampled = base;
        return;
    }
    if (!has_start)
        start = base;
    if (!has_end)
        end = start;

    if (spec->keyframes && g_variant_is_container(spec->keyframes)) {
        GVariantIter iter;
        GVariant* frame;
        g_variant_iter_init(&iter, spec->keyframes);
        while ((frame = g_variant_iter_next_value(&iter))) {
            if (g_variant_is_of_type(frame, G_VARIANT_TYPE_VARIANT)) {
                GVariant* unboxed = g_variant_get_variant(frame);
                g_variant_unref(frame);
                frame = unboxed;
            }
            double at, value;
            if (read_number(frame, "at", &at) && read_number(frame, key, &value)) {
                if (at <= progress && at >= left_at) {
                    left_at = at;
                    start = value;
                }
                if (at >= progress && at <= right_at) {
                    right_at = at;
                    end = value;
                    g_clear_pointer(&frame_ease, g_variant_unref);
                    frame_ease = lookup_field(frame, "ease");
                    if (frame_ease)
                        right_ease = frame_ease;
                }
            }
            g_variant_unref(frame);
        }
    }
    double local = right_at <= left_at ? 1 : (progress - left_at) / (right_at - left_at);
    *sampled = start + (end - start) * ease_progress_variant(right_ease, spec->ease, local);
}

static gboolean spec_has_property(AnimationSpec* spec, const char* key) {
    g_autoptr(GVariant) from = lookup_field(spec->from, key);
    g_autoptr(GVariant) to = lookup_field(spec->to, key);
    if (from || to)
        return TRUE;
    if (spec->keyframes && g_variant_is_container(spec->keyframes)) {
        GVariantIter iter;
        GVariant* frame;
        g_variant_iter_init(&iter, spec->keyframes);
        while ((frame = g_variant_iter_next_value(&iter))) {
            if (g_variant_is_of_type(frame, G_VARIANT_TYPE_VARIANT)) {
                GVariant* unboxed = g_variant_get_variant(frame);
                g_variant_unref(frame);
                frame = unboxed;
            }
            GVariant* value = lookup_field(frame, key);
            g_variant_unref(frame);
            if (value) {
                g_variant_unref(value);
                return TRUE;
            }
        }
    }
    return FALSE;
}

static void render_preview(AnimationPreview* preview, double progress) {
    double x, y, sx, sy, rotation, opacity;
    if (preview->finished)
        return;
    progress = CLAMP(progress, 0, 1);
    if (preview->resize_geometry) {
        double resize_progress;
        sample_property(&preview->spec, "progress", progress, 0, &resize_progress);
        resize_progress = CLAMP(resize_progress, 0, 1);
        x = preview->base_x + preview->resize_x * (1 - resize_progress);
        y = preview->base_y + preview->resize_y * (1 - resize_progress);
        sx = preview->base_sx *
             (preview->resize_scale_x + (1 - preview->resize_scale_x) * resize_progress);
        sy = preview->base_sy *
             (preview->resize_scale_y + (1 - preview->resize_scale_y) * resize_progress);
        rotation = preview->base_rotation;
        opacity = preview->base_opacity / 255.0;
    } else {
        sample_property(&preview->spec, "x", progress, preview->base_x, &x);
        sample_property(&preview->spec, "y", progress, preview->base_y, &y);
        if (spec_has_property(&preview->spec, "scale_x"))
            sample_property(&preview->spec, "scale_x", progress, preview->base_sx, &sx);
        else
            sample_property(&preview->spec, "scale", progress, preview->base_sx, &sx);
        if (spec_has_property(&preview->spec, "scale_y"))
            sample_property(&preview->spec, "scale_y", progress, preview->base_sy, &sy);
        else
            sample_property(&preview->spec, "scale", progress, preview->base_sy, &sy);
        sample_property(&preview->spec, "rotation", progress, preview->base_rotation, &rotation);
        sample_property(&preview->spec, "opacity", progress, preview->base_opacity / 255.0,
                        &opacity);
    }
    float tx, ty, tz;
    clutter_actor_get_translation(preview->actor, &tx, &ty, &tz);
    clutter_actor_set_translation(preview->actor, (float)x, (float)y, tz);
    clutter_actor_set_scale(preview->actor, sx, sy);
    clutter_actor_set_rotation_angle(preview->actor, CLUTTER_Z_AXIS, rotation);
    clutter_actor_set_opacity(preview->actor, (guint8)CLAMP(round(opacity * 255), 0, 255));
    preview->progress = progress;
}

static GVariant* preview_result(AnimationPreview* preview, gboolean playing) {
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(preview->id));
    g_variant_builder_add(&result, "{sv}", "session", g_variant_new_string(preview->id));
    g_variant_builder_add(&result, "{sv}", "name", g_variant_new_string(preview->spec.name));
    g_variant_builder_add(&result, "{sv}", "event", g_variant_new_string(preview->spec.event));
    g_autofree char* target =
        g_strdup_printf("%u", meta_window_get_stable_sequence(preview->window));
    g_variant_builder_add(&result, "{sv}", "target", g_variant_new_string(target));
    g_variant_builder_add(&result, "{sv}", "target_type",
                          g_variant_new_string(preview->target_type));
    g_variant_builder_add(&result, "{sv}", "progress", g_variant_new_double(preview->progress));
    g_variant_builder_add(&result, "{sv}", "playing", g_variant_new_boolean(playing));
    g_variant_builder_add(&result, "{sv}", "revision",
                          g_variant_new_uint64(gnoblin_native_control_get_config_revision(NULL)));
    g_variant_builder_add(&result, "{sv}", "paused", g_variant_new_boolean(!playing));
    g_variant_builder_add(&result, "{sv}", "spec", animation_spec_variant(&preview->spec));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

typedef struct {
    MetaDisplay* display;
    char* animation;
    char* target;
    char* event;
    char* from_workspace;
    char* to_workspace;
    char* direction;
    gboolean cancelled;
    gboolean started;
} CompletionEvent;

static gboolean publish_completion(gpointer user_data) {
    CompletionEvent* completion = user_data;
    GVariantBuilder payload;
    g_variant_builder_init(&payload, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload, "{sv}", "animation",
                          g_variant_new_string(completion->animation));
    g_variant_builder_add(&payload, "{sv}", "target", g_variant_new_string(completion->target));
    g_variant_builder_add(&payload, "{sv}", "event", g_variant_new_string(completion->event));
    g_variant_builder_add(&payload, "{sv}", "cancelled",
                          g_variant_new_boolean(completion->cancelled));
    if (completion->from_workspace) {
        g_variant_builder_add(&payload, "{sv}", "from_workspace",
                              g_variant_new_string(completion->from_workspace));
        g_variant_builder_add(&payload, "{sv}", "to_workspace",
                              g_variant_new_string(completion->to_workspace));
        g_variant_builder_add(&payload, "{sv}", "direction",
                              g_variant_new_string(completion->direction));
    }
    g_autoptr(GVariant) value = g_variant_ref_sink(g_variant_builder_end(&payload));
    const char* event_name =
        completion->started ? "gnoblin.animation.started" : "gnoblin.animation.finished";
    meta_display_dispatch_gnoblin_event(completion->display, event_name, value);
    meta_display_dispatch_gnoblin_native_event(completion->display, event_name, value);
    return G_SOURCE_REMOVE;
}

static void completion_event_free(gpointer user_data) {
    CompletionEvent* completion = user_data;
    g_object_unref(completion->display);
    g_free(completion->animation);
    g_free(completion->target);
    g_free(completion->event);
    g_free(completion->from_workspace);
    g_free(completion->to_workspace);
    g_free(completion->direction);
    g_free(completion);
}

static void emit_animation_event(AnimationPreview* preview, gboolean started, gboolean cancelled) {
    CompletionEvent* completion = g_new0(CompletionEvent, 1);
    completion->display = g_object_ref(preview->display);
    completion->animation = g_strdup(preview->spec.name);
    completion->target = g_strdup_printf("%u", meta_window_get_stable_sequence(preview->window));
    completion->event = g_strdup(preview->spec.event);
    completion->cancelled = cancelled;
    completion->started = started;
    g_idle_add_full(G_PRIORITY_DEFAULT_IDLE, publish_completion, completion, completion_event_free);
}

static const char* workspace_direction_name(MetaMotionDirection direction) {
    switch (direction) {
    case META_MOTION_LEFT:
        return "left";
    case META_MOTION_RIGHT:
        return "right";
    case META_MOTION_UP:
        return "up";
    case META_MOTION_DOWN:
        return "down";
    case META_MOTION_UP_LEFT:
        return "up-left";
    case META_MOTION_UP_RIGHT:
        return "up-right";
    case META_MOTION_DOWN_LEFT:
        return "down-left";
    case META_MOTION_DOWN_RIGHT:
        return "down-right";
    }

    return "unknown";
}

static void emit_workspace_animation_event(WorkspaceSwitch* workspace_switch, gboolean started,
                                           gboolean cancelled) {
    CompletionEvent* completion = g_new0(CompletionEvent, 1);
    completion->display = g_object_ref(workspace_switch->display);
    completion->animation = g_strdup(workspace_switch->spec.name);
    completion->target = g_strdup(workspace_switch->to_workspace_id);
    completion->event = g_strdup(workspace_switch->spec.event);
    completion->from_workspace = g_strdup(workspace_switch->from_workspace_id);
    completion->to_workspace = g_strdup(workspace_switch->to_workspace_id);
    completion->direction = g_strdup(workspace_direction_name(workspace_switch->direction));
    completion->cancelled = cancelled;
    completion->started = started;
    g_idle_add_full(G_PRIORITY_DEFAULT_IDLE, publish_completion, completion, completion_event_free);
}

static void complete_lifecycle(MetaPlugin* plugin, MetaWindowActor* actor,
                               AnimationCompletion completion) {
    switch (completion) {
    case ANIMATION_COMPLETION_NONE:
        break;
    case ANIMATION_COMPLETION_DESTROY:
        meta_plugin_destroy_completed(plugin, actor);
        break;
    case ANIMATION_COMPLETION_MINIMIZE:
        meta_plugin_minimize_completed(plugin, actor);
        break;
    case ANIMATION_COMPLETION_UNMINIMIZE:
        meta_plugin_unminimize_completed(plugin, actor);
        break;
    case ANIMATION_COMPLETION_SIZE_CHANGE:
        meta_plugin_size_change_completed(plugin, actor);
        break;
    default:
        g_assert_not_reached();
    }
}

static void preview_stop(AnimationPreview* preview, gboolean restore, gboolean report) {
    if (preview->finished)
        return;
    preview->finished = TRUE;
    MetaPlugin* lifecycle_plugin = g_steal_pointer(&preview->lifecycle_plugin);
    AnimationCompletion lifecycle_completion = preview->lifecycle_completion;
    preview->lifecycle_completion = ANIMATION_COMPLETION_NONE;
    MetaWindowActor* lifecycle_actor =
        lifecycle_plugin ? g_object_ref(META_WINDOW_ACTOR(preview->actor)) : NULL;
    if (preview->timeline)
        clutter_timeline_stop(preview->timeline);
    if (restore && CLUTTER_IS_ACTOR(preview->actor)) {
        clutter_actor_set_translation(preview->actor, preview->base_x, preview->base_y,
                                      preview->base_z);
        clutter_actor_set_scale(preview->actor, preview->base_sx, preview->base_sy);
        clutter_actor_set_rotation_angle(preview->actor, CLUTTER_Z_AXIS, preview->base_rotation);
        clutter_actor_set_opacity(preview->actor, preview->base_opacity);
        clutter_actor_set_pivot_point(preview->actor, preview->base_pivot_x, preview->base_pivot_y);
    }
    g_signal_handlers_disconnect_by_data(preview->window, preview);
    g_signal_handlers_disconnect_by_data(preview->actor, preview);
    if (preview->timeline)
        g_signal_handlers_disconnect_by_data(preview->timeline, preview);
    if (report)
        emit_animation_event(preview, FALSE, TRUE);
    AnimationState* state = g_object_get_data(G_OBJECT(preview->display), ANIMATION_ID_KEY);
    if (state && g_hash_table_lookup(state->previews, preview->id) == preview) {
        g_hash_table_remove(state->previews, preview->id);
        if (g_hash_table_size(state->previews) == 0 && !state->workspace_switch)
            stop_lock_watch(state);
    }
    if (lifecycle_plugin)
        complete_lifecycle(lifecycle_plugin, lifecycle_actor, lifecycle_completion);
    g_clear_object(&lifecycle_actor);
    g_clear_object(&lifecycle_plugin);
}

static void preview_completed(ClutterTimeline* timeline, gpointer user_data) {
    (void)timeline;
    AnimationPreview* preview = user_data;
    if (preview->finished)
        return;
    render_preview(preview, 1);
    emit_animation_event(preview, FALSE, FALSE);
    preview_stop(preview, TRUE, FALSE);
}

static void window_unmanaged(MetaWindow* window, gpointer user_data) {
    (void)window;
    AnimationPreview* preview = user_data;
    preview_stop(preview, FALSE, TRUE);
}

static void actor_destroyed(ClutterActor* actor, gpointer user_data) {
    (void)actor;
    AnimationPreview* preview = user_data;
    preview_stop(preview, FALSE, TRUE);
}

static void actor_mapped_changed(ClutterActor* actor, GParamSpec* pspec, gpointer user_data) {
    (void)pspec;
    AnimationPreview* preview = user_data;
    if (preview->lifecycle && !clutter_actor_is_mapped(actor))
        preview_stop(preview, TRUE, TRUE);
}

static void save_actor_state(AnimationPreview* preview) {
    clutter_actor_get_translation(preview->actor, &preview->base_x, &preview->base_y,
                                  &preview->base_z);
    clutter_actor_get_scale(preview->actor, &preview->base_sx, &preview->base_sy);
    preview->base_rotation = clutter_actor_get_rotation_angle(preview->actor, CLUTTER_Z_AXIS);
    preview->base_opacity = clutter_actor_get_opacity(preview->actor);
    clutter_actor_get_pivot_point(preview->actor, &preview->base_pivot_x, &preview->base_pivot_y);
}

static gboolean variant_to_double(GVariant* value, double* number) {
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
        *number = g_variant_get_double(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32))
        *number = g_variant_get_int32(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_UINT32))
        *number = g_variant_get_uint32(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
        *number = g_variant_get_int64(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_UINT64))
        *number = g_variant_get_uint64(value);
    else
        return FALSE;
    return isfinite(*number);
}

static void set_origin(AnimationPreview* preview) {
    static const struct {
        const char* name;
        float x, y;
    } pivots[] = {{"center", .5, .5},     {"top-left", 0, 0},    {"top-center", .5, 0},
                  {"top-right", 1, 0},    {"bottom-left", 0, 1}, {"bottom-center", .5, 1},
                  {"bottom-right", 1, 1}, {NULL, 0, 0}};
    const char* name = NULL;
    if (preview->spec.origin && g_variant_is_of_type(preview->spec.origin, G_VARIANT_TYPE_STRING))
        name = g_variant_get_string(preview->spec.origin, NULL);
    for (guint i = 0; pivots[i].name; i++)
        if (name && g_str_equal(name, pivots[i].name)) {
            clutter_actor_set_pivot_point(preview->actor, pivots[i].x, pivots[i].y);
            return;
        }
    if (preview->spec.origin && g_variant_is_container(preview->spec.origin) &&
        g_variant_n_children(preview->spec.origin) == 2) {
        double x, y;
        g_autoptr(GVariant) a_value = g_variant_get_child_value(preview->spec.origin, 0);
        g_autoptr(GVariant) b_value = g_variant_get_child_value(preview->spec.origin, 1);
        g_autoptr(GVariant) a = g_variant_is_of_type(a_value, G_VARIANT_TYPE_VARIANT)
                                    ? g_variant_get_variant(a_value)
                                    : g_variant_ref(a_value);
        g_autoptr(GVariant) b = g_variant_is_of_type(b_value, G_VARIANT_TYPE_VARIANT)
                                    ? g_variant_get_variant(b_value)
                                    : g_variant_ref(b_value);
        if (variant_to_double(a, &x) && variant_to_double(b, &y) && x >= 0 && x <= 1 && y >= 0 &&
            y <= 1)
            clutter_actor_set_pivot_point(preview->actor, (float)x, (float)y);
    }
}

static gboolean validate_previewable(AnimationSpec* spec, GError** error) {
    static const char* const actor_keys[] = {"x",       "y",        "scale",   "scale_x",
                                             "scale_y", "rotation", "opacity", NULL};
    static const char* const workspace_keys[] = {"progress", NULL};
    static const char* const resize_keys[] = {"progress", NULL};
    const char* const* allowed_keys = g_str_equal(spec->event, "workspace-switch") ? workspace_keys
                                      : g_str_equal(spec->event, "resize")         ? resize_keys
                                                                                   : actor_keys;
    GVariant* maps[] = {spec->from, spec->to};
    for (guint i = 0; i < G_N_ELEMENTS(maps); i++)
        if (maps[i] && !only_keys(maps[i], allowed_keys, error))
            return FALSE;
    if (!actor_event(spec->event)) {
        set_error(error, G_IO_ERROR_NOT_SUPPORTED,
                  "this event animates compositor state that is not an actor preview");
        return FALSE;
    }
    if (spec->duration > 10000) {
        set_error(error, G_IO_ERROR_INVALID_ARGUMENT,
                  "animation duration exceeds 10000 milliseconds");
        return FALSE;
    }
    if (spec->duration == 0) {
        set_error(error, G_IO_ERROR_INVALID_ARGUMENT,
                  "zero-duration animation cannot be previewed");
        return FALSE;
    }
    return TRUE;
}

static GVariant* animation_surfaces(MetaDisplay* display) {
    GVariantBuilder surfaces, result;
    g_autoptr(GList) windows = meta_display_list_all_windows(display);
    g_variant_builder_init(&surfaces, G_VARIANT_TYPE("aa{sv}"));
    for (GList* item = windows; item; item = item->next) {
        MetaWindow* window = item->data;
        const char* namespace = meta_gnoblin_layer_namespace(window);
        if (!namespace)
            continue;
        g_autofree char* id = g_strdup_printf("%u", meta_window_get_stable_sequence(window));
        GVariantBuilder entry;
        g_variant_builder_init(&entry, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&entry, "{sv}", "id", g_variant_new_string(id));
        g_variant_builder_add(&entry, "{sv}", "namespace", g_variant_new_string(namespace));
        g_variant_builder_add(&entry, "{sv}", "title",
                              g_variant_new_string(meta_window_get_title(window) ?: ""));
        g_variant_builder_add_value(&surfaces, g_variant_builder_end(&entry));
    }
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "surfaces", g_variant_builder_end(&surfaces));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static GVariant* animation_window_properties(MetaWindow* window) {
    GVariantBuilder properties;
    const char* namespace = meta_gnoblin_layer_namespace(window);
    const char* app_id = meta_window_get_gtk_application_id(window);
    MetaWorkspace* workspace = meta_window_get_workspace(window);
    g_variant_builder_init(&properties, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add (&properties, "{sv}", "app-id",
                         g_variant_new_string (app_id && *app_id
                           ? app_id : meta_window_get_wm_class (window) ?: ""));
    g_variant_builder_add(&properties, "{sv}", "title",
                          g_variant_new_string(meta_window_get_title(window) ?: ""));
    g_variant_builder_add(&properties, "{sv}", "layer",
                          namespace ? g_variant_new_string(namespace)
                                    : g_variant_new_maybe(G_VARIANT_TYPE_STRING, NULL));
    g_variant_builder_add(&properties, "{sv}", "type",
                          g_variant_new_string(namespace ? "layer" : "window"));
    g_variant_builder_add(&properties, "{sv}", "focused",
                          g_variant_new_boolean(meta_window_has_focus(window)));
    g_variant_builder_add(&properties, "{sv}", "workspace-id",
                          g_variant_new_maybe(G_VARIANT_TYPE_STRING, NULL));
    g_variant_builder_add(&properties, "{sv}", "workspace-number",
                          workspace ? g_variant_new_int32(meta_workspace_index(workspace) + 1)
                                    : g_variant_new_maybe(G_VARIANT_TYPE_INT32, NULL));
    return g_variant_ref_sink(g_variant_builder_end(&properties));
}

static GVariant* animation_public_context(MetaDisplay* display, MetaWindow* window,
                                          ClutterActor* actor, const char* event) {
    GVariantBuilder context, actor_info, monitor_info, offset, target_geometry;
    MtkRectangle monitor;
    double offset_x = 0, offset_y = 0;
    MtkRectangle target;
    gboolean has_target = (g_str_equal(event, "minimize") || g_str_equal(event, "restore")) &&
                          meta_window_get_icon_geometry(window, &target);
    int monitor_index = meta_window_get_monitor(window);
    g_variant_builder_init(&context, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&actor_info, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&actor_info, "{sv}", "x",
                          g_variant_new_double(clutter_actor_get_x(actor)));
    g_variant_builder_add(&actor_info, "{sv}", "y",
                          g_variant_new_double(clutter_actor_get_y(actor)));
    g_variant_builder_add(&actor_info, "{sv}", "width",
                          g_variant_new_double(clutter_actor_get_width(actor)));
    g_variant_builder_add(&actor_info, "{sv}", "height",
                          g_variant_new_double(clutter_actor_get_height(actor)));
    g_variant_builder_add(&context, "{sv}", "actor", g_variant_builder_end(&actor_info));
    if (monitor_index >= 0 && monitor_index < meta_display_get_n_monitors(display)) {
        meta_display_get_monitor_geometry(display, monitor_index, &monitor);
        g_variant_builder_init(&monitor_info, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&monitor_info, "{sv}", "x", g_variant_new_int32(monitor.x));
        g_variant_builder_add(&monitor_info, "{sv}", "y", g_variant_new_int32(monitor.y));
        g_variant_builder_add(&monitor_info, "{sv}", "width", g_variant_new_int32(monitor.width));
        g_variant_builder_add(&monitor_info, "{sv}", "height", g_variant_new_int32(monitor.height));
        g_variant_builder_add(&context, "{sv}", "monitor", g_variant_builder_end(&monitor_info));
    } else
        g_variant_builder_add(&context, "{sv}", "monitor",
                              g_variant_new_maybe(G_VARIANT_TYPE_VARDICT, NULL));
    g_variant_builder_init(&offset, G_VARIANT_TYPE("ad"));
    if (g_str_equal(event, "layer-open") || g_str_equal(event, "layer-close"))
        animation_layer_offset(window, &offset_x, &offset_y);
    g_variant_builder_add(&offset, "d", offset_x);
    g_variant_builder_add(&offset, "d", offset_y);
    g_variant_builder_add(&context, "{sv}", "offset", g_variant_builder_end(&offset));
    g_variant_builder_init(&target_geometry, G_VARIANT_TYPE("av"));
    g_variant_builder_add_value(&target_geometry,
                                g_variant_new_variant(g_variant_new_boolean(has_target)));
    if (has_target) {
        GVariantBuilder target_info;
        g_variant_builder_init(&target_info, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&target_info, "{sv}", "x", g_variant_new_int32(target.x));
        g_variant_builder_add(&target_info, "{sv}", "y", g_variant_new_int32(target.y));
        g_variant_builder_add(&target_info, "{sv}", "width", g_variant_new_int32(target.width));
        g_variant_builder_add(&target_info, "{sv}", "height", g_variant_new_int32(target.height));
        g_variant_builder_add_value(&target_geometry,
                                    g_variant_new_variant(g_variant_builder_end(&target_info)));
    } else
        g_variant_builder_add_value(&target_geometry, g_variant_new_variant(g_variant_new_maybe(
                                                          G_VARIANT_TYPE_VARDICT, NULL)));
    g_variant_builder_add(&context, "{sv}", "targetGeom", g_variant_builder_end(&target_geometry));
    g_variant_builder_add(
        &context, "{sv}", "rtl",
        g_variant_new_boolean(clutter_get_text_direction() == CLUTTER_TEXT_DIRECTION_RTL));
    return g_variant_ref_sink(g_variant_builder_end(&context));
}

static GVariant* animation_spec_variant(AnimationSpec* spec) {
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "name", g_variant_new_string(spec->name));
    g_variant_builder_add(&result, "{sv}", "event", g_variant_new_string(spec->event));
    g_variant_builder_add(&result, "{sv}", "duration", g_variant_new_uint32(spec->duration));
    g_variant_builder_add(&result, "{sv}", "ease",
                          spec->ease_value ? spec->ease_value : g_variant_new_string(spec->ease));
    g_variant_builder_add(&result, "{sv}", "from", spec->from);
    g_variant_builder_add(&result, "{sv}", "to", spec->to);
    if (spec->keyframes)
        g_variant_builder_add(&result, "{sv}", "keyframes", spec->keyframes);
    g_variant_builder_add(&result, "{sv}", "origin", spec->origin);
    g_variant_builder_add(&result, "{sv}", "target", spec->target);
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static GVariant* animation_inspect(MetaDisplay* display, GVariant* arguments,
                                   gboolean start_preview, GError** error) {
    static const char* const keys[] = {"name",       "target",   "event", "target_type",
                                       "targetType", "autoplay", NULL};
    if (!only_keys(arguments, keys, error))
        return NULL;
    const char *name = NULL, *target = NULL, *event = NULL, *target_type = NULL;
    if (!get_required_string(arguments, "name", &name, error))
        return NULL;
    if (strlen(name) > 80 || !g_regex_match_simple("^[A-Za-z0-9_-]+$", name, 0, 0)) {
        set_error(error, G_IO_ERROR_INVALID_ARGUMENT, "invalid animation name");
        return NULL;
    }
    if (!g_variant_lookup(arguments, "target", "&s", &target))
        target = "active";
    if (!g_variant_lookup(arguments, "event", "&s", &event))
        event = NULL;
    if (!g_variant_lookup(arguments, "target_type", "&s", &target_type) &&
        !g_variant_lookup(arguments, "targetType", "&s", &target_type))
        target_type = "window";
    if (!animation_target_type_valid(target_type)) {
        set_error(error, G_IO_ERROR_INVALID_ARGUMENT,
                  "target_type must be window, layer, or namespace");
        return NULL;
    }
    MetaWindow* window = find_window(display, target, target_type, error);
    if (!window)
        return NULL;
    if (is_locked(display)) {
        set_error(error, G_IO_ERROR_PERMISSION_DENIED,
                  "animations are unavailable while the session is locked");
        return NULL;
    }
    if (!meta_gnoblin_layer_namespace(window) &&
        (meta_window_is_skip_taskbar(window) || meta_window_is_override_redirect(window))) {
        set_error(error, G_IO_ERROR_NOT_FOUND, "animation target window is not public");
        return NULL;
    }
    MetaWindowActor* window_actor = META_WINDOW_ACTOR(meta_window_get_compositor_private(window));
    if (!window_actor) {
        set_error(error, G_IO_ERROR_NOT_FOUND, "animation target has no compositor actor");
        return NULL;
    }
    ClutterActor* actor = CLUTTER_ACTOR(window_actor);
    gboolean minimized = FALSE;
    g_object_get(window, "minimized", &minimized, NULL);
    if (!clutter_actor_is_visible(actor) || minimized ||
        !meta_window_showing_on_its_workspace(window)) {
        set_error(error, G_IO_ERROR_NOT_FOUND, "animation target must be visible and unminimized");
        return NULL;
    }
    AnimationSpec spec = {0};
    if (!parse_spec(&spec, name, event, window, error))
        return NULL;
    if ((g_str_equal(spec.event, "layer-open") || g_str_equal(spec.event, "layer-close")) &&
        !meta_gnoblin_layer_namespace(window)) {
        animation_spec_clear(&spec);
        set_error(error, G_IO_ERROR_INVALID_ARGUMENT,
                  "layer animation requires a layer-shell target");
        return NULL;
    }
    if (!validate_previewable(&spec, error)) {
        animation_spec_clear(&spec);
        return NULL;
    }

    if (start_preview) {
        AnimationState* state = get_state(display);
        GHashTableIter iter;
        gpointer value;
        g_hash_table_iter_init(&iter, state->previews);
        while (g_hash_table_iter_next(&iter, NULL, &value))
            if (((AnimationPreview*)value)->actor == actor)
                preview_stop(value, TRUE, ((AnimationPreview*)value)->lifecycle);

        AnimationPreview* preview = g_new0(AnimationPreview, 1);
        preview->id = g_uuid_string_random();
        preview->target_type = g_strdup(target_type);
        preview->display = display;
        preview->window = g_object_ref(window);
        preview->actor = actor;
        preview->spec = spec;
        save_actor_state(preview);
        set_origin(preview);
        render_preview(preview, 0);
        preview->timeline = clutter_timeline_new_for_actor(actor, spec.duration);
        clutter_timeline_set_progress_mode(preview->timeline, CLUTTER_LINEAR);
        g_signal_connect(preview->timeline, "new-frame", G_CALLBACK(preview_new_frame), preview);
        g_signal_connect(preview->timeline, "completed", G_CALLBACK(preview_completed), preview);
        g_signal_connect(window, "unmanaged", G_CALLBACK(window_unmanaged), preview);
        g_signal_connect(actor, "destroy", G_CALLBACK(actor_destroyed), preview);
        g_hash_table_insert(state->previews, g_strdup(preview->id), preview);
        start_lock_watch(display, state);
        gboolean autoplay = FALSE;
        g_variant_lookup(arguments, "autoplay", "b", &autoplay);
        if (autoplay)
            clutter_timeline_start(preview->timeline);
        return preview_result(preview, autoplay);
    }

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "name", g_variant_new_string(spec.name));
    g_variant_builder_add(&result, "{sv}", "event", g_variant_new_string(spec.event));
    g_autofree char* stable_id = g_strdup_printf("%u", meta_window_get_stable_sequence(window));
    g_variant_builder_add(&result, "{sv}", "target", g_variant_new_string(stable_id));
    g_variant_builder_add(&result, "{sv}", "target_type", g_variant_new_string(target_type));
    g_variant_builder_add(&result, "{sv}", "duration", g_variant_new_uint32(spec.duration));
    g_variant_builder_add(&result, "{sv}", "ease",
                          spec.ease_value ? spec.ease_value : g_variant_new_string(spec.ease));
    g_variant_builder_add(&result, "{sv}", "from", spec.from);
    g_variant_builder_add(&result, "{sv}", "to", spec.to);
    if (spec.keyframes)
        g_variant_builder_add(&result, "{sv}", "keyframes", spec.keyframes);
    g_autoptr(GVariant) properties = animation_window_properties(window);
    g_autoptr(GVariant) context = animation_public_context(display, window, actor, spec.event);
    g_autoptr(GVariant) public_spec = animation_spec_variant(&spec);
    g_variant_builder_add(&result, "{sv}", "properties", properties);
    g_variant_builder_add(&result, "{sv}", "context", context);
    g_variant_builder_add(&result, "{sv}", "spec", public_spec);
    animation_spec_clear(&spec);
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static void preview_new_frame(ClutterTimeline* timeline, guint elapsed, gpointer user_data) {
    (void)timeline;
    AnimationPreview* preview = user_data;
    if (is_locked(preview->display)) {
        preview_stop(preview, TRUE, TRUE);
        return;
    }
    render_preview(preview, preview->spec.duration ? elapsed / (double)preview->spec.duration : 1);
}

static GVariant* preview_control(MetaDisplay* display, const char* method, GVariant* arguments,
                                 GError** error) {
    static const char* const keys[] = {"session", "progress", "milliseconds", NULL};
    if (!only_keys(arguments, keys, error))
        return NULL;
    const char* id = NULL;
    if (!get_required_string(arguments, "session", &id, error))
        return NULL;
    AnimationState* state = get_state(display);
    AnimationPreview* preview = g_hash_table_lookup(state->previews, id);
    if (!preview) {
        set_error(error, G_IO_ERROR_NOT_FOUND, "animation preview session not found");
        return NULL;
    }
    if (is_locked(display)) {
        preview_stop(preview, TRUE, TRUE);
        set_error(error, G_IO_ERROR_PERMISSION_DENIED,
                  "animations are unavailable while the session is locked");
        return NULL;
    }

    if (g_str_equal(method, "animation.seek")) {
        double progress;
        if (!read_number(arguments, "progress", &progress) || progress < 0 || progress > 1) {
            set_error(error, G_IO_ERROR_INVALID_ARGUMENT, "progress must be between 0 and 1");
            return NULL;
        }
        clutter_timeline_pause(preview->timeline);
        clutter_timeline_advance(
            preview->timeline,
            MIN((guint)round(progress * preview->spec.duration), preview->spec.duration - 1));
        render_preview(preview, progress);
        return preview_result(preview, FALSE);
    }
    if (g_str_equal(method, "animation.step")) {
        gint64 ms;
        if (!g_variant_lookup(arguments, "milliseconds", "x", &ms) || ms < 1 || ms > 60000) {
            set_error(error, G_IO_ERROR_INVALID_ARGUMENT,
                      "step milliseconds must be from 1 to 60000");
            return NULL;
        }
        double progress = MIN(1, preview->progress + ms / (double)preview->spec.duration);
        clutter_timeline_pause(preview->timeline);
        clutter_timeline_advance(
            preview->timeline,
            MIN((guint)round(progress * preview->spec.duration), preview->spec.duration - 1));
        render_preview(preview, progress);
        return preview_result(preview, FALSE);
    }
    if (g_str_equal(method, "animation.play"))
        clutter_timeline_start(preview->timeline);
    else if (g_str_equal(method, "animation.pause"))
        clutter_timeline_pause(preview->timeline);
    else if (g_str_equal(method, "animation.stop")) {
        g_autofree char* copy = g_strdup(preview->id);
        preview_stop(preview, TRUE, TRUE);
        GVariantBuilder result;
        g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&result, "{sv}", "ok", g_variant_new_boolean(TRUE));
        g_variant_builder_add(&result, "{sv}", "session", g_variant_new_string(copy));
        return g_variant_ref_sink(g_variant_builder_end(&result));
    } else {
        set_error(error, G_IO_ERROR_NOT_SUPPORTED, "unsupported animation operation");
        return NULL;
    }
    return preview_result(preview, g_str_equal(method, "animation.play"));
}

static gboolean window_rule_matches(GVariant* match, MetaWindow* window,
                                    const char* layer_namespace, gboolean* out_matched,
                                    GError** error) {
    *out_matched = FALSE;
    const char* app_id = meta_window_get_gtk_application_id(window);
    const char* title = meta_window_get_title(window);
    MetaWorkspace* workspace = meta_window_get_workspace(window);
    GVariantIter iter;
    const char* key;
    GVariant* expected;

    if (!app_id || !*app_id)
        app_id = meta_window_get_wm_class(window);
    if (!app_id)
        app_id = "";
    if (!title)
        title = "";

    g_variant_iter_init(&iter, match);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &expected)) {
        gboolean matched = FALSE;

        if (g_str_equal(key, "type")) {
            const char* type = g_variant_is_of_type(expected, G_VARIANT_TYPE_STRING)
                                   ? g_variant_get_string(expected, NULL)
                                   : "";
            matched = g_str_equal(type, "window");
        } else if (g_str_equal(key, "focused")) {
            matched = g_variant_is_of_type(expected, G_VARIANT_TYPE_BOOLEAN) &&
                      g_variant_get_boolean(expected) == meta_window_has_focus(window);
        } else if (g_str_equal(key, "workspace-id")) {
            const char* wanted = g_variant_is_of_type(expected, G_VARIANT_TYPE_STRING)
                                     ? g_variant_get_string(expected, NULL)
                                     : NULL;
            matched =
                workspace && wanted && g_str_equal(wanted, meta_gnoblin_workspace_id(workspace));
        } else if (g_str_equal(key, "workspace-number")) {
            double wanted;
            matched = workspace && variant_to_double(expected, &wanted) &&
                      wanted == meta_workspace_index(workspace) + 1;
        } else if (g_str_equal(key, "app-id") || g_str_equal(key, "title") ||
                   g_str_equal(key, "layer")) {
            const char* pattern = g_variant_is_of_type(expected, G_VARIANT_TYPE_STRING)
                                      ? g_variant_get_string(expected, NULL)
                                      : NULL;
            const char* value = g_str_equal(key, "app-id")  ? app_id
                                : g_str_equal(key, "title") ? title
                                                            : layer_namespace;

            if (pattern && value) {
                matched = meta_gnoblin_window_pattern_match(pattern, value, error);
                if (error && *error) {
                    g_variant_unref(expected);
                    return FALSE;
                }
            }
        }

        g_variant_unref(expected);
        if (!matched)
            return TRUE;
    }

    *out_matched = TRUE;
    return TRUE;
}

static gboolean animation_supports_lifecycle_event(const char* name, const char* event) {
    g_autoptr(GVariant) custom = find_custom_animation(name, NULL);
    if (custom) {
        gboolean enabled = TRUE;
        const char* configured_event = NULL;
        g_variant_lookup(custom, "enable", "b", &enabled);
        g_variant_lookup(custom, "event", "&s", &configured_event);
        return enabled && g_strcmp0(configured_event, event) == 0;
    }

    if (g_str_equal(name, "none"))
        return g_strv_contains((const char* const[]){"minimize", "restore", "open", "close",
                                                     "resize", "workspace-switch", NULL},
                               event);

    return builtin_supports_event(name, event);
}

static char* animation_for_event(const char* event) {
    g_autoptr(GVariant) animations = config_animations();

    if (animations && g_variant_is_container(animations)) {
        GVariantIter iter;
        GVariant* candidate;

        g_variant_iter_init(&iter, animations);
        while ((candidate = g_variant_iter_next_value(&iter))) {
            if (g_variant_is_of_type(candidate, G_VARIANT_TYPE_VARIANT)) {
                GVariant* unboxed = g_variant_get_variant(candidate);
                g_variant_unref(candidate);
                candidate = unboxed;
            }

            const char* name = NULL;
            const char* candidate_event = NULL;
            gboolean enabled = TRUE;
            g_variant_lookup(candidate, "name", "&s", &name);
            g_variant_lookup(candidate, "event", "&s", &candidate_event);
            g_variant_lookup(candidate, "enable", "b", &enabled);

            if (enabled && name && g_strcmp0(candidate_event, event) == 0 &&
                animation_supports_lifecycle_event(name, event)) {
                char* selected = g_strdup(name);
                g_variant_unref(candidate);
                return selected;
            }

            g_variant_unref(candidate);
        }
    }

    return NULL;
}

static gboolean select_window_lifecycle_animation(MetaWindow* window, const char* layer_namespace,
                                                  const char* event, char** selected_name,
                                                  GVariant** selected_overrides, GError** error) {
    g_autoptr(GVariant) document = gnoblin_native_control_get_config_document(NULL);
    g_autoptr(GVariant) rules =
        document ? g_variant_lookup_value(document, "window-rules", NULL) : NULL;
    GVariantIter iter;
    GVariant* rule_value;

    *selected_name = animation_for_event(event);
    *selected_overrides = NULL;
    if (!rules || !g_variant_is_container(rules))
        return TRUE;

    g_variant_iter_init(&iter, rules);
    while ((rule_value = g_variant_iter_next_value(&iter))) {
        g_autoptr(GVariant) rule = NULL;
        g_autoptr(GVariant) match = NULL;
        g_autoptr(GVariant) animation = NULL;
        const char* name = NULL;

        if (g_variant_is_of_type(rule_value, G_VARIANT_TYPE_VARIANT))
            rule = g_variant_get_variant(rule_value);
        else
            rule = g_variant_ref(rule_value);
        g_variant_unref(rule_value);
        if (!g_variant_is_of_type(rule, G_VARIANT_TYPE_VARDICT))
            continue;

        match = lookup_field(rule, "match");
        if (!match || !g_variant_is_of_type(match, G_VARIANT_TYPE_VARDICT))
            continue;
        gboolean rule_matches = FALSE;
        if (!window_rule_matches(match, window, layer_namespace, &rule_matches, error))
            return FALSE;
        if (!rule_matches)
            continue;
        animation = lookup_field(rule, "animation");
        if (!animation)
            continue;

        if (g_variant_is_of_type(animation, G_VARIANT_TYPE_STRING)) {
            name = g_variant_get_string(animation, NULL);
            if ((!g_str_equal(event, "open") && !g_str_equal(event, "resize")) ||
                !animation_supports_lifecycle_event(name, event))
                continue;
            g_free(*selected_name);
            *selected_name = g_strdup(name);
            g_clear_pointer(selected_overrides, g_variant_unref);
            continue;
        }

        if (g_variant_is_of_type(animation, G_VARIANT_TYPE_VARDICT)) {
            g_autoptr(GVariant) selected = lookup_field(animation, event);
            if (!selected || !g_variant_is_of_type(selected, G_VARIANT_TYPE_STRING))
                continue;
            name = g_variant_get_string(selected, NULL);
            if (!animation_supports_lifecycle_event(name, event))
                continue;
            g_free(*selected_name);
            *selected_name = g_strdup(name);
            g_clear_pointer(selected_overrides, g_variant_unref);
            *selected_overrides = g_variant_ref(animation);
        }
    }

    /* No configured animation is a valid choice; only rule matching can fail. */
    return TRUE;
}

static void apply_rule_overrides(AnimationSpec* spec, GVariant* overrides) {
    g_autoptr(GVariant) duration = lookup_field(overrides, "duration");
    g_autoptr(GVariant) ease = lookup_field(overrides, "easing");

    if (duration) {
        double value;
        if (variant_to_double(duration, &value) && value >= 0 && value <= 5000)
            spec->duration = (guint)value;
    }
    if (ease && g_variant_is_of_type(ease, G_VARIANT_TYPE_STRING)) {
        g_free(spec->ease);
        spec->ease = g_variant_dup_string(ease, NULL);
        g_clear_pointer(&spec->ease_value, g_variant_unref);
        spec->ease_value = g_variant_ref(ease);
    }
}

static void workspace_actor_state_free(gpointer data) {
    WorkspaceActorState* actor_state = data;
    g_clear_object(&actor_state->actor);
    g_clear_object(&actor_state->parent);
    g_clear_object(&actor_state->previous_sibling);
    g_clear_object(&actor_state->next_sibling);
    g_free(actor_state);
}

static void render_workspace_switch(WorkspaceSwitch* workspace_switch, double elapsed_progress) {
    double progress = 0;
    double outgoing_x = 0, outgoing_y = 0;
    double direction_x = 0, direction_y = 0;

    if (workspace_switch->finished)
        return;
    sample_property(&workspace_switch->spec, "progress", elapsed_progress, 0, &progress);
    progress = CLAMP(progress, 0, 1);
    switch (workspace_switch->direction) {
    case META_MOTION_LEFT:
        direction_x = 1;
        break;
    case META_MOTION_RIGHT:
        direction_x = -1;
        break;
    case META_MOTION_UP:
        direction_y = 1;
        break;
    case META_MOTION_DOWN:
        direction_y = -1;
        break;
    case META_MOTION_UP_LEFT:
        direction_x = direction_y = 1;
        break;
    case META_MOTION_UP_RIGHT:
        direction_x = -1;
        direction_y = 1;
        break;
    case META_MOTION_DOWN_LEFT:
        direction_x = 1;
        direction_y = -1;
        break;
    case META_MOTION_DOWN_RIGHT:
        direction_x = direction_y = -1;
        break;
    default:
        break;
    }
    outgoing_x = direction_x * workspace_switch->width;
    outgoing_y = direction_y * workspace_switch->height;

    clutter_actor_set_translation(workspace_switch->outgoing_group, (float)(outgoing_x * progress),
                                  (float)(outgoing_y * progress), 0);
    clutter_actor_set_translation(workspace_switch->incoming_group,
                                  (float)(-outgoing_x * (1 - progress)),
                                  (float)(-outgoing_y * (1 - progress)), 0);
}

static void workspace_switch_new_frame(ClutterTimeline* timeline, guint elapsed,
                                       gpointer user_data) {
    WorkspaceSwitch* workspace_switch = user_data;
    guint duration = clutter_timeline_get_duration(timeline);
    render_workspace_switch(workspace_switch, duration ? elapsed / (double)duration : 1);
}

static void workspace_switch_completed(ClutterTimeline* timeline, gpointer user_data) {
    WorkspaceSwitch* workspace_switch = user_data;
    (void)timeline;
    render_workspace_switch(workspace_switch, 1);
    workspace_switch_stop(workspace_switch, TRUE, FALSE);
}

static void workspace_switch_restore_actors(WorkspaceSwitch* workspace_switch) {
    for (guint i = 0; i < workspace_switch->actors->len; i++) {
        WorkspaceActorState* actor_state = g_ptr_array_index(workspace_switch->actors, i);
        ClutterActor* actor = actor_state->actor;
        if (clutter_actor_get_parent(actor) != workspace_switch->outgoing_group &&
            clutter_actor_get_parent(actor) != workspace_switch->incoming_group)
            continue;
        g_object_ref(actor);
        clutter_actor_remove_child(clutter_actor_get_parent(actor), actor);
        if (actor_state->parent && CLUTTER_IS_ACTOR(actor_state->parent))
            clutter_actor_add_child(actor_state->parent, actor);
        if (actor_state->was_visible)
            clutter_actor_show(actor);
        else
            clutter_actor_hide(actor);
        g_object_unref(actor);
    }

    /* Restore each moved actor against its original neighbors when they remain
     * in the same parent. This preserves local stacking without relying on the
     * order in which workspace actors were returned. */
    for (guint i = 0; i < workspace_switch->actors->len; i++) {
        WorkspaceActorState* actor_state = g_ptr_array_index(workspace_switch->actors, i);
        ClutterActor* actor = actor_state->actor;
        if (clutter_actor_get_parent(actor) != actor_state->parent)
            continue;
        if (actor_state->previous_sibling &&
            clutter_actor_get_parent(actor_state->previous_sibling) == actor_state->parent)
            clutter_actor_insert_child_above(actor_state->parent, actor,
                                             actor_state->previous_sibling);
        else if (actor_state->next_sibling &&
                 clutter_actor_get_parent(actor_state->next_sibling) == actor_state->parent)
            clutter_actor_insert_child_below(actor_state->parent, actor, actor_state->next_sibling);
    }
}

static void workspace_switch_stop(WorkspaceSwitch* workspace_switch, gboolean complete,
                                  gboolean cancelled) {
    if (workspace_switch->finished)
        return;
    workspace_switch->finished = TRUE;
    emit_workspace_animation_event(workspace_switch, FALSE, cancelled);
    AnimationState* state =
        g_object_get_data(G_OBJECT(workspace_switch->display), ANIMATION_ID_KEY);
    if (workspace_switch->timeline) {
        g_signal_handlers_disconnect_by_data(workspace_switch->timeline, workspace_switch);
        clutter_timeline_stop(workspace_switch->timeline);
    }
    workspace_switch_restore_actors(workspace_switch);
    if (workspace_switch->outgoing_group)
        clutter_actor_destroy(workspace_switch->outgoing_group);
    if (workspace_switch->incoming_group)
        clutter_actor_destroy(workspace_switch->incoming_group);
    if (state && state->workspace_switch == workspace_switch)
        state->workspace_switch = NULL;
    if (state && !state->workspace_switch && g_hash_table_size(state->previews) == 0)
        stop_lock_watch(state);
    if (complete)
        meta_plugin_switch_workspace_completed(workspace_switch->plugin);
    animation_spec_clear(&workspace_switch->spec);
    g_clear_object(&workspace_switch->timeline);
    g_clear_object(&workspace_switch->stage);
    g_clear_object(&workspace_switch->outgoing_group);
    g_clear_object(&workspace_switch->incoming_group);
    g_free(workspace_switch->from_workspace_id);
    g_free(workspace_switch->to_workspace_id);
    g_ptr_array_unref(workspace_switch->actors);
    g_clear_object(&workspace_switch->plugin);
    g_free(workspace_switch);
}

static void workspace_switch_stop_for_display(MetaDisplay* display, gboolean complete) {
    AnimationState* state = g_object_get_data(G_OBJECT(display), ANIMATION_ID_KEY);
    if (state && state->workspace_switch)
        workspace_switch_stop(state->workspace_switch, complete, TRUE);
}

static void workspace_switch_shutdown(MetaBackend* backend, MetaDisplay* display) {
    (void)backend;
    workspace_switch_stop_for_display(display, TRUE);
}

gboolean meta_gnoblin_animation_switch_workspace(MetaPlugin* plugin, gint from, gint to,
                                                 MetaMotionDirection direction) {
    g_return_val_if_fail(META_IS_PLUGIN(plugin), FALSE);
    MetaDisplay* display = meta_plugin_get_display(plugin);
    AnimationState* state = g_object_get_data(G_OBJECT(display), ANIMATION_ID_KEY);
    if (state && state->workspace_switch)
        workspace_switch_stop(state->workspace_switch, TRUE, TRUE);
    if (from < 0 || to < 0 || from == to)
        return FALSE;

    MetaWorkspaceManager* workspace_manager = meta_display_get_workspace_manager(display);
    MetaWorkspace* from_workspace =
        meta_workspace_manager_get_workspace_by_index(workspace_manager, from);
    MetaWorkspace* to_workspace =
        meta_workspace_manager_get_workspace_by_index(workspace_manager, to);
    MetaCompositor* compositor = meta_display_get_compositor(display);
    ClutterActor* window_group = meta_compositor_get_window_group(compositor);
    const char* name;
    AnimationSpec spec = {0};
    g_autoptr(GError) error = NULL;
    GList* window_actors;

    if (!from_workspace || !to_workspace || is_locked(display))
        return FALSE;

    state = get_state(display);

    g_autofree char* selected_name = animation_for_event("workspace-switch");
    name = selected_name;
    if (!name || g_str_equal(name, "none") ||
        !parse_spec(&spec, name, "workspace-switch", NULL, &error) ||
        !validate_previewable(&spec, &error) || spec.duration == 0) {
        animation_spec_clear(&spec);
        return FALSE;
    }

    WorkspaceSwitch* workspace_switch = g_new0(WorkspaceSwitch, 1);
    workspace_switch->display = display;
    workspace_switch->plugin = g_object_ref(plugin);
    workspace_switch->stage = g_object_ref(meta_compositor_get_stage(compositor));
    workspace_switch->direction = direction;
    workspace_switch->from_workspace_id = g_strdup(meta_gnoblin_workspace_id(from_workspace));
    workspace_switch->to_workspace_id = g_strdup(meta_gnoblin_workspace_id(to_workspace));
    workspace_switch->spec = spec;
    workspace_switch->actors = g_ptr_array_new_with_free_func(workspace_actor_state_free);
    int width, height;
    meta_display_get_size(display, &width, &height);
    workspace_switch->width = width;
    workspace_switch->height = height;
    if (workspace_switch->width <= 0 || workspace_switch->height <= 0)
        goto fail;

    window_actors = meta_compositor_get_window_actors(compositor);
    for (GList* l = window_actors; l; l = l->next) {
        MetaWindowActor* window_actor = l->data;
        MetaWindow* window = meta_window_actor_get_meta_window(window_actor);
        MetaWorkspace* workspace = window ? meta_window_get_workspace(window) : NULL;
        if (!workspace || meta_window_is_on_all_workspaces(window))
            continue;
        int workspace_index = meta_workspace_index(workspace);
        if (workspace_index != from && workspace_index != to)
            continue;
        ClutterActor* actor = CLUTTER_ACTOR(window_actor);
        /* Override-redirect windows live in the top window group. Let Mutter
         * keep those transient surfaces above the workspace animation. */
        if (clutter_actor_get_parent(actor) != window_group)
            continue;
        WorkspaceActorState* actor_state = g_new0(WorkspaceActorState, 1);
        actor_state->actor = g_object_ref(actor);
        actor_state->parent = g_object_ref(window_group);
        ClutterActor* previous = clutter_actor_get_previous_sibling(actor);
        ClutterActor* next = clutter_actor_get_next_sibling(actor);
        actor_state->previous_sibling = previous ? g_object_ref(previous) : NULL;
        actor_state->next_sibling = next ? g_object_ref(next) : NULL;
        actor_state->was_visible = clutter_actor_is_visible(actor);
        g_ptr_array_add(workspace_switch->actors, actor_state);
    }
    if (workspace_switch->actors->len == 0)
        goto fail;

    workspace_switch->outgoing_group = g_object_ref_sink(clutter_actor_new());
    workspace_switch->incoming_group = g_object_ref_sink(clutter_actor_new());
    clutter_actor_set_size(workspace_switch->outgoing_group, (float)workspace_switch->width,
                           (float)workspace_switch->height);
    clutter_actor_set_size(workspace_switch->incoming_group, (float)workspace_switch->width,
                           (float)workspace_switch->height);
    clutter_actor_add_child(window_group, workspace_switch->outgoing_group);
    clutter_actor_add_child(window_group, workspace_switch->incoming_group);

    for (guint i = workspace_switch->actors->len; i > 0; i--) {
        WorkspaceActorState* actor_state = g_ptr_array_index(workspace_switch->actors, i - 1);
        MetaWindow* window =
            meta_window_actor_get_meta_window(META_WINDOW_ACTOR(actor_state->actor));
        MetaWorkspace* workspace = window ? meta_window_get_workspace(window) : NULL;
        ClutterActor* group = workspace && meta_workspace_index(workspace) == to
                                  ? workspace_switch->incoming_group
                                  : workspace_switch->outgoing_group;
        g_object_ref(actor_state->actor);
        clutter_actor_remove_child(window_group, actor_state->actor);
        clutter_actor_add_child(group, actor_state->actor);
        clutter_actor_set_child_below_sibling(group, actor_state->actor, NULL);
        g_object_unref(actor_state->actor);
    }

    state->workspace_switch = workspace_switch;
    start_lock_watch(display, state);
    workspace_switch->timeline = clutter_timeline_new_for_actor(
        CLUTTER_ACTOR(workspace_switch->stage), workspace_switch->spec.duration);
    clutter_timeline_set_progress_mode(workspace_switch->timeline, CLUTTER_LINEAR);
    g_signal_connect(workspace_switch->timeline, "new-frame",
                     G_CALLBACK(workspace_switch_new_frame), workspace_switch);
    g_signal_connect(workspace_switch->timeline, "completed",
                     G_CALLBACK(workspace_switch_completed), workspace_switch);
    render_workspace_switch(workspace_switch, 0);
    emit_workspace_animation_event(workspace_switch, TRUE, FALSE);
    clutter_timeline_start(workspace_switch->timeline);
    return TRUE;

fail:
    workspace_switch->finished = TRUE;
    workspace_switch_restore_actors(workspace_switch);
    if (workspace_switch->outgoing_group)
        clutter_actor_destroy(workspace_switch->outgoing_group);
    if (workspace_switch->incoming_group)
        clutter_actor_destroy(workspace_switch->incoming_group);
    animation_spec_clear(&workspace_switch->spec);
    g_clear_object(&workspace_switch->stage);
    g_clear_object(&workspace_switch->outgoing_group);
    g_clear_object(&workspace_switch->incoming_group);
    g_free(workspace_switch->from_workspace_id);
    g_free(workspace_switch->to_workspace_id);
    g_ptr_array_unref(workspace_switch->actors);
    g_clear_object(&workspace_switch->plugin);
    g_free(workspace_switch);
    return FALSE;
}

void meta_gnoblin_animation_kill_switch_workspace(MetaPlugin* plugin) {
    if (!META_IS_PLUGIN(plugin))
        return;
    MetaDisplay* display = meta_plugin_get_display(plugin);
    AnimationState* state = g_object_get_data(G_OBJECT(display), ANIMATION_ID_KEY);
    if (state && state->workspace_switch && state->workspace_switch->plugin == plugin)
        workspace_switch_stop(state->workspace_switch, TRUE, TRUE);
}

static GVariant* dict_with_progress_default(GVariant* source, double default_progress) {
    GVariantBuilder builder;
    GVariantIter iter;
    const char* key;
    GVariant* value;
    gboolean has_progress = FALSE;

    if (source && !g_variant_is_of_type(source, G_VARIANT_TYPE_VARDICT))
        return g_variant_ref(source);

    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    if (source) {
        g_variant_iter_init(&iter, source);
        while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
            if (g_str_equal(key, "progress"))
                has_progress = TRUE;
            g_variant_builder_add(&builder, "{sv}", key, value);
            g_variant_unref(value);
        }
    }
    if (!has_progress)
        frame_add_number(&builder, "progress", default_progress);
    return g_variant_ref_sink(g_variant_builder_end(&builder));
}

static gboolean start_window_lifecycle_animation(MetaPlugin* plugin, AnimationCompletion completion,
                                                 MetaWindowActor* window_actor, const char* event,
                                                 const MtkRectangle* old_buffer_rect) {
    MetaWindow* window = meta_window_actor_get_meta_window(window_actor);
    if (!window)
        return FALSE;
    MetaDisplay* display = meta_window_get_display(window);
    const char* layer_namespace = meta_gnoblin_layer_namespace(window);
    g_autofree char* name = NULL;
    g_autoptr(GVariant) overrides = NULL;
    AnimationSpec spec = {0};
    g_autoptr(GError) error = NULL;

    /* This actor can map again after its window is hidden and shown. */
    if (g_str_equal(event, "open")) {
        if (g_object_get_data(G_OBJECT(window), "gnoblin-window-open-animation-handled"))
            return FALSE;
        g_object_set_data(G_OBJECT(window), "gnoblin-window-open-animation-handled",
                          GINT_TO_POINTER(1));
    }

    /* Normal windows use the lifecycle events as they are. Dialogs only open and
     * close, and use the dialog events so that dialog-open and dialog-close
     * registrations apply to them. Other window types and layer surfaces never
     * start a window lifecycle animation here. */
    const char* selected_event = event;
    MetaWindowType window_type = meta_window_get_window_type(window);
    if (window_type == META_WINDOW_DIALOG || window_type == META_WINDOW_MODAL_DIALOG) {
        if (g_str_equal(event, "open"))
            selected_event = "dialog-open";
        else if (g_str_equal(event, "close"))
            selected_event = "dialog-close";
        else
            return FALSE;
    } else if (window_type != META_WINDOW_NORMAL) {
        return FALSE;
    }
    if (meta_window_is_override_redirect(window) || layer_namespace ||
        meta_gnoblin_layer_anchor(window) >= 0 || is_locked(display))
        return FALSE;

    if (!select_window_lifecycle_animation(window, layer_namespace, selected_event, &name,
                                           &overrides, &error)) {
        g_warning("failed to match Gnoblin window animation rules: %s",
                  error ? error->message : "unknown Lua pattern error");
        return FALSE;
    }
    if (!name || g_str_equal(name, "none"))
        return FALSE;
    if (!parse_spec(&spec, name, selected_event, window, &error))
        return FALSE;
    apply_rule_overrides(&spec, overrides);
    gboolean resize_geometry = g_str_equal(event, "resize");
    double resize_x = 0, resize_y = 0;
    double resize_scale_x = 1, resize_scale_y = 1;
    if (resize_geometry) {
        MtkRectangle new_buffer_rect;
        if (!old_buffer_rect || old_buffer_rect->width <= 0 || old_buffer_rect->height <= 0) {
            animation_spec_clear(&spec);
            return FALSE;
        }
        meta_window_get_buffer_rect(window, &new_buffer_rect);
        if (new_buffer_rect.width <= 0 || new_buffer_rect.height <= 0) {
            animation_spec_clear(&spec);
            return FALSE;
        }

        resize_x = old_buffer_rect->x - new_buffer_rect.x;
        resize_y = old_buffer_rect->y - new_buffer_rect.y;
        resize_scale_x = old_buffer_rect->width / (double)new_buffer_rect.width;
        resize_scale_y = old_buffer_rect->height / (double)new_buffer_rect.height;
        GVariant* from = dict_with_progress_default(spec.from, 0);
        GVariant* to = dict_with_progress_default(spec.to, 1);
        g_clear_pointer(&spec.from, g_variant_unref);
        g_clear_pointer(&spec.to, g_variant_unref);
        spec.from = from;
        spec.to = to;
    }
    if (!validate_previewable(&spec, &error) || spec.duration == 0) {
        animation_spec_clear(&spec);
        return FALSE;
    }

    AnimationState* state = get_state(display);
    while (TRUE) {
        AnimationPreview* existing = NULL;
        GHashTableIter previews;
        gpointer value;

        g_hash_table_iter_init(&previews, state->previews);
        while (g_hash_table_iter_next(&previews, NULL, &value))
            if (((AnimationPreview*)value)->actor == CLUTTER_ACTOR(window_actor)) {
                existing = value;
                break;
            }
        if (!existing)
            break;
        preview_stop(existing, TRUE, existing->lifecycle);
    }

    AnimationPreview* preview = g_new0(AnimationPreview, 1);
    preview->id = g_uuid_string_random();
    preview->target_type = g_strdup("window");
    preview->display = display;
    preview->window = g_object_ref(window);
    preview->actor = CLUTTER_ACTOR(window_actor);
    preview->spec = spec;
    preview->lifecycle = TRUE;
    preview->lifecycle_plugin = plugin ? g_object_ref(plugin) : NULL;
    preview->lifecycle_completion = completion;
    preview->resize_geometry = resize_geometry;
    preview->resize_x = resize_x;
    preview->resize_y = resize_y;
    preview->resize_scale_x = resize_scale_x;
    preview->resize_scale_y = resize_scale_y;
    if (completion == ANIMATION_COMPLETION_UNMINIMIZE)
        clutter_actor_show(preview->actor);
    save_actor_state(preview);
    set_origin(preview);
    render_preview(preview, 0);
    preview->timeline = clutter_timeline_new_for_actor(preview->actor, preview->spec.duration);
    clutter_timeline_set_progress_mode(preview->timeline, CLUTTER_LINEAR);
    g_signal_connect(preview->timeline, "new-frame", G_CALLBACK(preview_new_frame), preview);
    g_signal_connect(preview->timeline, "completed", G_CALLBACK(preview_completed), preview);
    g_signal_connect(window, "unmanaged", G_CALLBACK(window_unmanaged), preview);
    g_signal_connect(preview->actor, "notify::mapped", G_CALLBACK(actor_mapped_changed), preview);
    g_signal_connect(preview->actor, "destroy", G_CALLBACK(actor_destroyed), preview);
    g_hash_table_insert(state->previews, g_strdup(preview->id), preview);

    emit_animation_event(preview, TRUE, FALSE);
    clutter_timeline_start(preview->timeline);
    return TRUE;
}

static void start_window_open_animation(MetaWindowActor* window_actor) {
    start_window_lifecycle_animation(NULL, ANIMATION_COMPLETION_NONE, window_actor, "open", NULL);
}

void meta_gnoblin_animation_window_mapped(MetaWindowActor* window_actor) {
    g_return_if_fail(META_IS_WINDOW_ACTOR(window_actor));
    start_window_open_animation(window_actor);
}

gboolean meta_gnoblin_animation_window_destroying(MetaPlugin* plugin,
                                                  MetaWindowActor* window_actor) {
    g_return_val_if_fail(META_IS_PLUGIN(plugin), FALSE);
    g_return_val_if_fail(META_IS_WINDOW_ACTOR(window_actor), FALSE);
    return start_window_lifecycle_animation(plugin, ANIMATION_COMPLETION_DESTROY, window_actor,
                                            "close", NULL);
}

gboolean meta_gnoblin_animation_window_minimizing(MetaPlugin* plugin,
                                                  MetaWindowActor* window_actor) {
    g_return_val_if_fail(META_IS_PLUGIN(plugin), FALSE);
    g_return_val_if_fail(META_IS_WINDOW_ACTOR(window_actor), FALSE);
    return start_window_lifecycle_animation(plugin, ANIMATION_COMPLETION_MINIMIZE, window_actor,
                                            "minimize", NULL);
}

gboolean meta_gnoblin_animation_window_restoring(MetaPlugin* plugin,
                                                 MetaWindowActor* window_actor) {
    g_return_val_if_fail(META_IS_PLUGIN(plugin), FALSE);
    g_return_val_if_fail(META_IS_WINDOW_ACTOR(window_actor), FALSE);
    return start_window_lifecycle_animation(plugin, ANIMATION_COMPLETION_UNMINIMIZE, window_actor,
                                            "restore", NULL);
}

static void pending_resize_free(gpointer user_data) {
    PendingResize* pending = user_data;
    g_clear_object(&pending->plugin);
    g_free(pending);
}

void meta_gnoblin_animation_size_change(MetaPlugin* plugin, MetaWindowActor* window_actor,
                                        MetaSizeChange which_change, MtkRectangle* old_frame_rect,
                                        MtkRectangle* old_buffer_rect) {
    g_return_if_fail(META_IS_PLUGIN(plugin));
    g_return_if_fail(META_IS_WINDOW_ACTOR(window_actor));
    g_return_if_fail(old_buffer_rect != NULL);
    (void)which_change;
    (void)old_frame_rect;

    PendingResize* previous = g_object_steal_data(G_OBJECT(window_actor), RESIZE_PENDING_KEY);
    if (previous) {
        meta_plugin_size_change_completed(previous->plugin, window_actor);
        pending_resize_free(previous);
    }

    PendingResize* pending = g_new0(PendingResize, 1);
    pending->plugin = g_object_ref(plugin);
    pending->old_buffer_rect = *old_buffer_rect;
    g_object_set_data_full(G_OBJECT(window_actor), RESIZE_PENDING_KEY, pending,
                           pending_resize_free);
}

void meta_gnoblin_animation_size_changed(MetaPlugin* plugin, MetaWindowActor* window_actor) {
    g_return_if_fail(META_IS_PLUGIN(plugin));
    g_return_if_fail(META_IS_WINDOW_ACTOR(window_actor));

    PendingResize* pending = g_object_steal_data(G_OBJECT(window_actor), RESIZE_PENDING_KEY);
    if (!pending)
        return;

    gboolean started =
        start_window_lifecycle_animation(pending->plugin, ANIMATION_COMPLETION_SIZE_CHANGE,
                                         window_actor, "resize", &pending->old_buffer_rect);
    if (!started)
        meta_plugin_size_change_completed(pending->plugin, window_actor);
    pending_resize_free(pending);
}

void meta_gnoblin_animation_cancel_window_effects(MetaPlugin* plugin,
                                                  MetaWindowActor* window_actor) {
    g_return_if_fail(META_IS_PLUGIN(plugin));
    g_return_if_fail(META_IS_WINDOW_ACTOR(window_actor));
    MetaDisplay* display = meta_plugin_get_display(plugin);
    PendingResize* pending = g_object_steal_data(G_OBJECT(window_actor), RESIZE_PENDING_KEY);
    if (pending) {
        meta_plugin_size_change_completed(pending->plugin, window_actor);
        pending_resize_free(pending);
    }
    AnimationState* state = g_object_get_data(G_OBJECT(display), ANIMATION_ID_KEY);
    if (!state)
        return;
    while (TRUE) {
        AnimationPreview* preview = NULL;
        GHashTableIter iter;
        gpointer value;

        g_hash_table_iter_init(&iter, state->previews);
        while (g_hash_table_iter_next(&iter, NULL, &value))
            if (((AnimationPreview*)value)->actor == CLUTTER_ACTOR(window_actor)) {
                preview = value;
                break;
            }
        if (!preview)
            break;
        preview_stop(preview, TRUE, TRUE);
    }
}

GVariant* meta_gnoblin_animation_dispatch(MetaDisplay* display, const char* method,
                                          GVariant* arguments, GError** error) {
    static const char* const no_args[] = {NULL};
    if (is_locked(display)) {
        AnimationState* state = g_object_get_data(G_OBJECT(display), ANIMATION_ID_KEY);
        if (state)
            while (g_hash_table_size(state->previews) > 0) {
                GHashTableIter iter;
                gpointer value;
                g_hash_table_iter_init(&iter, state->previews);
                if (!g_hash_table_iter_next(&iter, NULL, &value))
                    break;
                preview_stop(value, TRUE, TRUE);
            }
        set_error(error, G_IO_ERROR_PERMISSION_DENIED,
                  "animations are unavailable while the session is locked");
        return NULL;
    }
    if (g_str_equal(method, "animation.list")) {
        if (!only_keys(arguments ? arguments : empty_dict(), no_args, error))
            return NULL;
        return animation_list(error);
    }
    if (g_str_equal(method, "animation.get")) {
        static const char* const keys[] = {"name", NULL};
        if (!only_keys(arguments, keys, error))
            return NULL;
        const char* name = NULL;
        if (!get_required_string(arguments, "name", &name, error))
            return NULL;
        GVariant* custom = find_custom_animation(name, NULL);
        if (custom) {
            GVariantBuilder record;
            GVariantIter iter;
            const char* key;
            GVariant* value;
            g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
            g_variant_iter_init(&iter, custom);
            while (g_variant_iter_next(&iter, "{&sv}", &key, &value))
                g_variant_builder_add(&record, "{sv}", key, value);
            g_variant_builder_add(
                &record, "{sv}", "revision",
                g_variant_new_uint64(gnoblin_native_control_get_config_revision(NULL)));
            g_variant_unref(custom);
            return g_variant_ref_sink(g_variant_builder_end(&record));
        }
        const BuiltinAnimation* builtin = find_builtin(name);
        if (builtin) {
            guint duration = 0;
            const char* ease = "linear";
            if (builtin->event && builtin_previewable(name))
                builtin_event_defaults(builtin->event, &duration, &ease);
            return make_info(name, builtin->event, duration, ease, TRUE,
                             builtin_previewable(name) &&
                                 !internal_event(builtin->event ? builtin->event : ""));
        }
        return g_variant_ref_sink(g_variant_new_maybe(G_VARIANT_TYPE_VARDICT, NULL));
    }
    if (g_str_equal(method, "animation.surfaces")) {
        if (!only_keys(arguments ? arguments : empty_dict(), no_args, error))
            return NULL;
        return animation_surfaces(display);
    }
    if (g_str_equal(method, "animation.inspect"))
        return animation_inspect(display, arguments, FALSE, error);
    if (g_str_equal(method, "animation.preview"))
        return animation_inspect(display, arguments, TRUE, error);
    if (g_str_equal(method, "animation.seek") || g_str_equal(method, "animation.step") ||
        g_str_equal(method, "animation.play") || g_str_equal(method, "animation.pause") ||
        g_str_equal(method, "animation.stop"))
        return preview_control(display, method, arguments, error);
    set_error(error, G_IO_ERROR_NOT_SUPPORTED, "unsupported animation API operation");
    return NULL;
}
