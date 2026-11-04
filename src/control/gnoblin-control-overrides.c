/* Runtime overrides: a value layered over a committed setting, or an effect layered over one
 * window, that the Lua runtime can take back.
 *
 * One list holds every override, oldest first. An override has an owner: the runtime generation
 * that made it, or 0 for a socket client (gnoblinctl input orientation-lock, monitors
 * set_privacy_screen). The rules are the same for both kinds:
 *
 *   - The latest override of one target wins. Making a second override of the same setting, or of
 *     the same effect on the same window, ends the first with the reason "replaced".
 *   - revoke ends an override with "revoked". A closed window ends its effects with
 *     "window_closed".
 *   - A reload that commits ends every override ("config_reloaded", cleared by
 *     gnoblin_control_overrides_clear). A stopped or restarted Lua worker ends the ones its
 *     runtime made. The Lua runtime ends its own handles at the same moment, so these two paths
 *     do not publish.
 *
 * A setting override changes what the compositor reads as the configuration: the runtime cache
 * holds the committed document with the overrides on top (gnoblin-runtime-cache.h), and the
 * settings that can change at runtime are applied again from it. The orientation lock and the
 * privacy screen are the two settings that have their own state in Mutter; they are applied
 * through it. An effect is one more rule, after the configured ones, in the rule engine of
 * gnoblin-control-window-rules.c.
 *
 *   settings.override    {section, key, value}
 *   window.set_effect    {id (the window), name, value}
 *   override.revoke      {id}
 *
 * Mutter calls `gnoblin.override.closed` {id, reason} on the runtime that owns an override that
 * ended for any of these reasons while it was running. */
#include "config.h"

#include "core/gnoblin-control-internal.h"
#include "core/gnoblin-native-control-private.h"
#include "core/gnoblin-overrides.h"

typedef enum {
    OVERRIDE_SETTING,
    OVERRIDE_EFFECT,
} OverrideScope;

typedef struct {
    char* id;
    OverrideScope scope;
    char* section;   /* setting: "window-management" */
    char* key;       /* setting: "focus-mode"; effect: "blur-ignore-shadows" */
    char* window_id; /* effect only */
    GVariant* value;
    guint64 generation; /* owner, 0 for a socket client */
} RuntimeOverride;

static void override_free(RuntimeOverride* override) {
    g_free(override->id);
    g_free(override->section);
    g_free(override->key);
    g_free(override->window_id);
    g_clear_pointer(&override->value, g_variant_unref);
    g_free(override);
}

static gboolean setting_is_orientation_lock(const char* section, const char* key) {
    return g_str_equal(section, "input") && g_str_equal(key, "orientation-lock");
}

static gboolean setting_is_privacy_screen(const char* section, const char* key) {
    return g_str_equal(section, "monitors") && g_str_equal(key, "privacy-screen");
}

static gboolean same_target(const RuntimeOverride* override, OverrideScope scope,
                            const char* section, const char* key, const char* window_id) {
    return override->scope == scope && g_str_equal(override->key, key) &&
           g_strcmp0(override->section, section) == 0 &&
           g_strcmp0(override->window_id, window_id) == 0;
}

/* The newest override of this setting, or NULL. */
static const RuntimeOverride* latest_setting(GnoblinNativeControl* control, const char* section,
                                             const char* key) {
    if (!control->overrides)
        return NULL;
    for (guint i = control->overrides->len; i > 0; i--) {
        const RuntimeOverride* override = g_ptr_array_index(control->overrides, i - 1);
        if (same_target(override, OVERRIDE_SETTING, section, key, NULL))
            return override;
    }
    return NULL;
}

static void publish_closed(GnoblinNativeControl* control, const RuntimeOverride* override,
                           const char* reason) {
    if (!override->generation || override->generation != control->runtime_generation ||
        control->stopping)
        return;
    GVariantBuilder payload;
    g_variant_builder_init(&payload, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&payload, "{sv}", "id", g_variant_new_string(override->id));
    g_variant_builder_add(&payload, "{sv}", "reason", g_variant_new_string(reason));
    g_variant_builder_add(&payload, "{sv}", "sequence",
                          g_variant_new_int64(++control->event_sequence));
    g_variant_builder_add(&payload, "{sv}", "time", g_variant_new_int64(g_get_monotonic_time()));
    g_autoptr(GVariant) event = g_variant_ref_sink(g_variant_builder_end(&payload));
    native_runtime_dispatch_event(control, "gnoblin.override.closed", event);
}

static void apply_orientation_lock(GnoblinNativeControl* control) {
    if (!control->orientation_manager)
        return;
    const RuntimeOverride* override = latest_setting(control, "input", "orientation-lock");
    control->orientation_lock_notifications_suppressed = TRUE;
    if (override) {
        control->orientation_lock_runtime_override = TRUE;
        meta_orientation_manager_set_orientation_locked(control->orientation_manager,
                                                        g_variant_get_boolean(override->value));
    } else {
        control->orientation_lock_runtime_override = FALSE;
        if (control->orientation_lock_configured)
            meta_orientation_manager_set_orientation_locked(control->orientation_manager,
                                                            control->orientation_lock_config_value);
        else
            meta_orientation_manager_clear_orientation_lock_override(control->orientation_manager);
    }
    control->orientation_lock_notifications_suppressed = FALSE;
    native_orientation_lock_publish(control);
}

static void apply_privacy_screen(GnoblinNativeControl* control) {
    const RuntimeOverride* override = latest_setting(control, "monitors", "privacy-screen");
    control->monitor_privacy_screen_runtime_override = override != NULL;
    control->monitor_privacy_screen_runtime_value =
        override && g_variant_get_boolean(override->value);
    meta_settings_apply_gnoblin_privacy_screen_runtime_override(
        meta_backend_get_settings(control->backend),
        control->monitor_privacy_screen_runtime_override,
        control->monitor_privacy_screen_runtime_value);
    native_monitor_privacy_screen_publish(control);
}

/* The committed document with every document-backed setting override on top, or NULL when there
 * is none. */
static GVariant* build_layered_document(GnoblinNativeControl* control) {
    if (!control->overrides || !control->runtime_cache)
        return NULL;
    g_autoptr(GVariant) committed =
        gnoblin_runtime_cache_get_committed_document(control->runtime_cache);
    if (!committed)
        return NULL;
    GVariantDict document;
    g_variant_dict_init(&document, committed);
    g_autoptr(GPtrArray) done = g_ptr_array_new();
    gboolean any = FALSE;
    for (guint i = 0; i < control->overrides->len; i++) {
        const RuntimeOverride* first = g_ptr_array_index(control->overrides, i);
        if (first->scope != OVERRIDE_SETTING ||
            setting_is_orientation_lock(first->section, first->key) ||
            setting_is_privacy_screen(first->section, first->key) ||
            g_ptr_array_find_with_equal_func(done, first->section, g_str_equal, NULL))
            continue;
        g_ptr_array_add(done, first->section);
        any = TRUE;
        g_autoptr(GVariant) current =
            g_variant_dict_lookup_value(&document, first->section, G_VARIANT_TYPE_VARDICT);
        GVariantDict section;
        g_variant_dict_init(&section, current);
        for (guint j = i; j < control->overrides->len; j++) {
            const RuntimeOverride* override = g_ptr_array_index(control->overrides, j);
            if (override->scope == OVERRIDE_SETTING &&
                g_str_equal(override->section, first->section))
                g_variant_dict_insert_value(&section, override->key, override->value);
        }
        g_variant_dict_insert_value(&document, first->section, g_variant_dict_end(&section));
    }
    GVariant* layered = g_variant_ref_sink(g_variant_dict_end(&document));
    if (!any) {
        g_variant_unref(layered);
        return NULL;
    }
    return layered;
}

/* Layer the document-backed overrides over the committed document and apply the settings that
 * change at runtime from it. */
static void apply_document_settings(GnoblinNativeControl* control) {
    if (!control->runtime_cache || control->stopping || !control->display)
        return;
    g_autoptr(GVariant) layered = build_layered_document(control);
    gnoblin_runtime_cache_set_overlay(control->runtime_cache, layered);
    g_autoptr(GVariant) effective = gnoblin_runtime_cache_get_document(control->runtime_cache);
    native_apply_runtime_preferences(control, effective);
}

static void apply_setting(GnoblinNativeControl* control, const char* section, const char* key) {
    if (control->stopping || !control->display)
        return;
    if (setting_is_orientation_lock(section, key))
        apply_orientation_lock(control);
    else if (setting_is_privacy_screen(section, key))
        apply_privacy_screen(control);
    else
        apply_document_settings(control);
}

static void apply_effect(GnoblinNativeControl* control, const char* window_id) {
    if (control->stopping || !control->display)
        return;
    MetaWindow* window = native_window_by_stable_id(control, window_id);
    if (window)
        gnoblin_control_apply_window_rules(control, window);
}

static void apply_target(GnoblinNativeControl* control, const RuntimeOverride* override) {
    if (override->scope == OVERRIDE_SETTING)
        apply_setting(control, override->section, override->key);
    else
        apply_effect(control, override->window_id);
}

/* Take `override` out of the list and free it, after telling its owner. With `apply`, the target
 * returns to the next layer. */
static void override_end(GnoblinNativeControl* control, RuntimeOverride* override,
                         const char* reason, gboolean apply) {
    g_ptr_array_remove(control->overrides, override);
    publish_closed(control, override, reason);
    if (apply)
        apply_target(control, override);
    override_free(override);
}

static GVariant* override_record(const RuntimeOverride* override) {
    GVariantBuilder record;
    g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&record, "{sv}", "id", g_variant_new_string(override->id));
    return g_variant_ref_sink(g_variant_builder_end(&record));
}

/* Add `override`, which the list now owns. An older override of the same target ends first. */
static GVariant* override_add(GnoblinNativeControl* control, RuntimeOverride* override) {
    if (!control->overrides)
        control->overrides = g_ptr_array_new();
    for (guint i = control->overrides->len; i > 0; i--) {
        RuntimeOverride* older = g_ptr_array_index(control->overrides, i - 1);
        if (same_target(older, override->scope, override->section, override->key,
                        override->window_id))
            override_end(control, older, "replaced", FALSE);
    }
    g_ptr_array_add(control->overrides, override);
    apply_target(control, override);
    return override_record(override);
}

static gboolean value_is_scalar(GVariant* value) {
    return value && (g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN) ||
                     g_variant_is_of_type(value, G_VARIANT_TYPE_STRING) ||
                     g_variant_is_of_type(value, G_VARIANT_TYPE_INT32) ||
                     g_variant_is_of_type(value, G_VARIANT_TYPE_INT64) ||
                     g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE));
}

static gboolean value_number(GVariant* value, double* number) {
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

static gboolean any_monitor_has_privacy_screen(GnoblinNativeControl* control) {
    GList* logical_monitors =
        control->monitor_manager
            ? meta_monitor_manager_get_logical_monitors(control->monitor_manager)
            : NULL;
    for (GList* logical = logical_monitors; logical; logical = logical->next) {
        for (GList* item = meta_logical_monitor_get_monitors(logical->data); item;
             item = item->next) {
            MetaMonitor* monitor = item->data;
            if (meta_monitor_is_active(monitor) &&
                meta_monitor_get_privacy_screen_state(monitor) != META_PRIVACY_SCREEN_UNAVAILABLE)
                return TRUE;
        }
    }
    return FALSE;
}

static gboolean check_setting(GnoblinNativeControl* control, const char* section, const char* key,
                              GVariant* value, GError** error) {
    g_autofree char* refusal = gnoblin_override_setting_refusal(section, key);
    if (refusal) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                    "%s.%s cannot change at runtime: %s", section, key, refusal);
        return FALSE;
    }
    if (!value_is_scalar(value)) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "%s.%s takes a boolean, number or string", section, key);
        return FALSE;
    }
    if (setting_is_orientation_lock(section, key)) {
        if (!g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "input.orientation-lock takes true or false; revoke the override "
                                "to follow the configuration again");
            return FALSE;
        }
        if (!control->orientation_manager) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                                "orientation lock is unavailable on this backend");
            return FALSE;
        }
    } else if (setting_is_privacy_screen(section, key)) {
        if (!g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "monitors.privacy-screen takes true or false; revoke the "
                                "override to follow the configuration again");
            return FALSE;
        }
        if (!any_monitor_has_privacy_screen(control)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                                "no output has a privacy screen");
            return FALSE;
        }
    } else {
        g_autoptr(GVariant) committed =
            control->runtime_cache
                ? gnoblin_runtime_cache_get_committed_document(control->runtime_cache)
                : NULL;
        if (!committed) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                                "the configuration is not loaded");
            return FALSE;
        }
    }
    return TRUE;
}

static RuntimeOverride* setting_override_new(const char* id, const char* section, const char* key,
                                             GVariant* value, guint64 generation) {
    RuntimeOverride* override = g_new0(RuntimeOverride, 1);
    override->id = g_strdup(id);
    override->scope = OVERRIDE_SETTING;
    override->section = g_strdup(section);
    override->key = g_strdup(key);
    override->value = g_variant_ref(value);
    override->generation = generation;
    return override;
}

/* settings.override {section, key, value}: the runtime generation that asked owns the override. */
GVariant* gnoblin_control_override_setting(GnoblinNativeControl* control, gint64 operation_id,
                                           GVariant* arguments, guint64 owner_generation,
                                           GError** error) {
    const char* section = NULL;
    const char* key = NULL;
    g_autoptr(GVariant) value = g_variant_lookup_value(arguments, "value", NULL);
    if (owner_generation != control->runtime_generation) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "settings.override belongs to an old runtime generation");
        return NULL;
    }
    if (!g_variant_lookup(arguments, "section", "&s", &section) ||
        !g_variant_lookup(arguments, "key", "&s", &key) || !*section || !*key || !value) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "settings.override needs a section, a key and a value");
        return NULL;
    }
    if (!check_setting(control, section, key, value, error))
        return NULL;
    g_autofree char* id = g_strdup_printf("override-%" G_GINT64_FORMAT, operation_id);
    return override_add(control, setting_override_new(id, section, key, value, owner_generation));
}

/* A socket client's setting: it has no object to hold, so it stays until it is set again,
 * cleared with a NULL `value`, or a reload ends it. */
gboolean gnoblin_control_override_socket_set(GnoblinNativeControl* control, const char* section,
                                             const char* key, GVariant* value, GError** error) {
    if (!value) {
        if (!control->overrides)
            return TRUE;
        for (guint i = control->overrides->len; i > 0; i--) {
            RuntimeOverride* override = g_ptr_array_index(control->overrides, i - 1);
            if (same_target(override, OVERRIDE_SETTING, section, key, NULL))
                override_end(control, override, "revoked", TRUE);
        }
        return TRUE;
    }
    if (!check_setting(control, section, key, value, error))
        return FALSE;
    static guint64 socket_overrides;
    g_autofree char* id = g_strdup_printf("override-socket-%" G_GUINT64_FORMAT, ++socket_overrides);
    g_autoptr(GVariant) record =
        override_add(control, setting_override_new(id, section, key, value, 0));
    return TRUE;
}

static gboolean check_effect(const char* name, GVariant* value, GError** error) {
    double number = 0;
    gboolean valid = TRUE;
    if (g_str_equal(name, "opacity"))
        valid = value_number(value, &number) && number >= 0 && number <= 1;
    else if (g_str_equal(name, "blur"))
        valid = value_number(value, &number) && number >= 0 && number <= 100;
    else if (g_str_equal(name, "blur_ignore_shadows"))
        valid = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN);
    else if (g_str_equal(name, "shader"))
        valid = g_variant_is_of_type(value, G_VARIANT_TYPE_STRING);
    else
        valid = g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT); /* the tables */
    if (!valid)
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "the %s effect has an invalid value", name);
    return valid;
}

/* window.set_effect {id, name, value}: one effect of one window's rules. */
GVariant* gnoblin_control_override_window_effect(GnoblinNativeControl* control, gint64 operation_id,
                                                 GVariant* arguments, guint64 owner_generation,
                                                 GError** error) {
    const char* window_id = NULL;
    const char* name = NULL;
    g_autoptr(GVariant) value = g_variant_lookup_value(arguments, "value", NULL);
    if (owner_generation != control->runtime_generation) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.set_effect belongs to an old runtime generation");
        return NULL;
    }
    if (!g_variant_lookup(arguments, "id", "&s", &window_id) ||
        !g_variant_lookup(arguments, "name", "&s", &name) || !value) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.set_effect needs a window id, a name and a value");
        return NULL;
    }
    if (!gnoblin_override_effect_known(name)) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "\"%s\" is not a window effect; the effects are %s", name,
                    GNOBLIN_OVERRIDE_EFFECT_LIST);
        return NULL;
    }
    if (!check_effect(name, value, error))
        return NULL;
    MetaWindow* window = native_window_by_stable_id(control, window_id);
    if (!window) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "window.set_effect target is unavailable");
        return NULL;
    }
    RuntimeOverride* override = g_new0(RuntimeOverride, 1);
    override->id = g_strdup_printf("override-%" G_GINT64_FORMAT, operation_id);
    override->scope = OVERRIDE_EFFECT;
    override->key = gnoblin_override_document_name(name);
    override->window_id = g_strdup(window_id);
    override->value = g_variant_ref(value);
    override->generation = owner_generation;
    return override_add(control, override);
}

/* override.revoke {id}: end an override this runtime made. */
GVariant* gnoblin_control_override_revoke(GnoblinNativeControl* control, GVariant* arguments,
                                          guint64 owner_generation, GError** error) {
    const char* id = NULL;
    if (!g_variant_lookup(arguments, "id", "&s", &id) || !id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "override.revoke needs an id");
        return NULL;
    }
    RuntimeOverride* found = NULL;
    for (guint i = 0; control->overrides && i < control->overrides->len; i++) {
        RuntimeOverride* override = g_ptr_array_index(control->overrides, i);
        if (override->generation == owner_generation && g_str_equal(override->id, id))
            found = override;
    }
    if (!found) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND, "override %s has ended", id);
        return NULL;
    }
    override_end(control, found, "revoked", TRUE);
    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "revoked", g_variant_new_boolean(TRUE));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

/* End the overrides a reload or a stopped worker invalidates: those of a Lua runtime, and with
 * `include_socket` those of socket clients too. Nobody is told: the Lua runtime that held them is
 * gone or ends its own handles. Each target is applied once, after all of them are out. */
void gnoblin_control_overrides_clear(GnoblinNativeControl* control, gboolean include_socket) {
    if (!control->overrides || !control->overrides->len)
        return;
    g_autoptr(GPtrArray) ended = g_ptr_array_new_with_free_func((GDestroyNotify)override_free);
    for (guint i = control->overrides->len; i > 0; i--) {
        RuntimeOverride* override = g_ptr_array_index(control->overrides, i - 1);
        if (override->generation || include_socket) {
            g_ptr_array_remove_index(control->overrides, i - 1);
            g_ptr_array_add(ended, override);
        }
    }
    gboolean documents = FALSE;
    for (guint i = 0; i < ended->len; i++) {
        const RuntimeOverride* override = g_ptr_array_index(ended, i);
        if (override->scope == OVERRIDE_SETTING &&
            (setting_is_orientation_lock(override->section, override->key) ||
             setting_is_privacy_screen(override->section, override->key))) {
            apply_setting(control, override->section, override->key);
        } else if (override->scope == OVERRIDE_SETTING) {
            documents = TRUE;
        } else {
            apply_effect(control, override->window_id);
        }
    }
    if (documents)
        apply_document_settings(control);
}

/* A configuration was applied over the overrides (a new document, or the rollback of one): put
 * the overrides back on top. */
void gnoblin_control_overrides_reapply(GnoblinNativeControl* control, gboolean orientation_lock,
                                       gboolean documents) {
    if (orientation_lock && latest_setting(control, "input", "orientation-lock"))
        apply_orientation_lock(control);
    if (documents)
        apply_document_settings(control);
}

/* The window is gone: its effects end with it. */
void gnoblin_control_overrides_window_closed(GnoblinNativeControl* control, const char* window_id) {
    for (guint i = control->overrides ? control->overrides->len : 0; i > 0; i--) {
        RuntimeOverride* override = g_ptr_array_index(control->overrides, i - 1);
        if (override->scope == OVERRIDE_EFFECT && g_str_equal(override->window_id, window_id))
            override_end(control, override, "window_closed", FALSE);
    }
}

/* The effects of this window as one more window rule, applied after the configured rules, or
 * NULL when it has none. */
GVariant* gnoblin_control_window_effect_rule(GnoblinNativeControl* control, MetaWindow* window) {
    if (!control->overrides || !control->overrides->len)
        return NULL;
    g_autofree char* window_id = native_window_id(window);
    GVariantBuilder rule;
    g_variant_builder_init(&rule, G_VARIANT_TYPE_VARDICT);
    gboolean any = FALSE;
    for (guint i = 0; i < control->overrides->len; i++) {
        const RuntimeOverride* override = g_ptr_array_index(control->overrides, i);
        if (override->scope == OVERRIDE_EFFECT && g_str_equal(override->window_id, window_id)) {
            g_variant_builder_add(&rule, "{sv}", override->key, override->value);
            any = TRUE;
        }
    }
    if (!any) {
        g_variant_builder_clear(&rule);
        return NULL;
    }
    return g_variant_ref_sink(g_variant_builder_end(&rule));
}

void gnoblin_control_overrides_free(GnoblinNativeControl* control) {
    if (!control->overrides)
        return;
    for (guint i = 0; i < control->overrides->len; i++)
        override_free(g_ptr_array_index(control->overrides, i));
    g_clear_pointer(&control->overrides, g_ptr_array_unref);
}
