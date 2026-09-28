/* SPDX-License-Identifier: LGPL-2.1-or-later */
#include "gnoblin-portal-policy.h"
#include "gnoblin-portal-identity.h"

#include <string.h>

#define GNOBLIN_COMPOSITOR_NAME "org.gnoblin.Compositor"
#define GNOBLIN_COMPOSITOR_PATH "/org/gnoblin/Compositor"
#define GNOBLIN_COMPOSITOR_IFACE "org.gnoblin.Compositor"
#define PERMISSION_REGEX_FLAGS (G_REGEX_OPTIMIZE | G_REGEX_JAVASCRIPT_COMPAT)

static const char* capabilities[] = {"screen-cast", "remote-desktop", "input-capture",
                                     "screenshot",  "access",         NULL};
static const char* levels[] = {"default", "ask", "allow", "deny", NULL};
static const char* devices[] = {"keyboard", "pointer", "touchscreen", NULL};

static gboolean string_in(const char* value, const char* const* values) {
    for (guint i = 0; values[i]; i++)
        if (g_str_equal(value, values[i]))
            return TRUE;
    return FALSE;
}

static gboolean has_only_keys(GVariant* dict, const char* const* allowed) {
    GVariantIter iter;
    const char* key;
    GVariant* value;
    if (!g_variant_is_of_type(dict, G_VARIANT_TYPE_VARDICT))
        return FALSE;
    g_variant_iter_init(&iter, dict);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        gboolean found = FALSE;
        for (guint i = 0; allowed[i]; i++)
            found |= g_str_equal(key, allowed[i]);
        g_variant_unref(value);
        if (!found)
            return FALSE;
    }
    return TRUE;
}

static gboolean string_array_valid(GVariant* value, const char* const* choices, gboolean nonempty,
                                   gboolean unique) {
    if (!value || !g_variant_is_of_type(value, G_VARIANT_TYPE("av")) ||
        (nonempty && g_variant_n_children(value) == 0))
        return FALSE;
    g_autoptr(GHashTable) seen =
        unique ? g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL) : NULL;
    for (gsize i = 0; i < g_variant_n_children(value); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
        g_autoptr(GVariant) child = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(child, G_VARIANT_TYPE_STRING))
            return FALSE;
        const char* text = g_variant_get_string(child, NULL);
        if (!*text || !g_utf8_validate(text, -1, NULL) || (choices && !string_in(text, choices)) ||
            (seen && g_hash_table_contains(seen, text)))
            return FALSE;
        if (seen)
            g_hash_table_add(seen, g_strdup(text));
    }
    return TRUE;
}

static gboolean valid_identity(const char* identity) {
    return identity && strlen(identity) <= 4096 && g_utf8_validate(identity, -1, NULL) &&
           ((g_str_has_prefix(identity, "app-id:") && identity[7] != '\0') ||
            (g_str_has_prefix(identity, "host-exe:/") && identity[10] != '\0'));
}

gboolean gnoblin_permission_policy_validate(GVariant* document, GError** error) {
    static const char* const policy_keys[] = {"default", "rules", NULL};
    static const char* const rule_keys[] = {"name",     "match",   "capabilities", "level",
                                            "monitors", "devices", "clipboard",    NULL};
    g_autoptr(GVariant) policy =
        document ? g_variant_lookup_value(document, "permissions", NULL) : NULL;
    if (!policy)
        return TRUE;
    if (!has_only_keys(policy, policy_keys)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "permissions must be a table containing only default and rules");
        return FALSE;
    }
    gboolean valid = TRUE;
    g_autoptr(GVariant) default_value = g_variant_lookup_value(policy, "default", NULL);
    const char* fallback = "default";
    if (default_value) {
        valid = valid && g_variant_is_of_type(default_value, G_VARIANT_TYPE_STRING);
        if (g_variant_is_of_type(default_value, G_VARIANT_TYPE_STRING))
            fallback = g_variant_get_string(default_value, NULL);
    }
    valid = valid && string_in(fallback, levels) && !g_str_equal(fallback, "allow");
    g_autoptr(GVariant) rules = g_variant_lookup_value(policy, "rules", NULL);
    if (!rules) {
        if (!valid)
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "permissions.default must be default, ask or deny");
        return valid;
    }
    valid = valid && g_variant_is_of_type(rules, G_VARIANT_TYPE("av")) &&
            g_variant_n_children(rules) <= 256;
    g_autoptr(GHashTable) names = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    for (gsize i = 0; valid && i < g_variant_n_children(rules); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(rules, i);
        g_autoptr(GVariant) rule = g_variant_get_variant(boxed);
        if (!has_only_keys(rule, rule_keys)) {
            valid = FALSE;
            break;
        }
        g_autoptr(GVariant) name_v = g_variant_lookup_value(rule, "name", NULL);
        g_autoptr(GVariant) match_v = g_variant_lookup_value(rule, "match", NULL);
        g_autoptr(GVariant) level_v = g_variant_lookup_value(rule, "level", NULL);
        g_autoptr(GVariant) capabilities_v = g_variant_lookup_value(rule, "capabilities", NULL);
        const char* name = name_v && g_variant_is_of_type(name_v, G_VARIANT_TYPE_STRING)
                               ? g_variant_get_string(name_v, NULL)
                               : "";
        const char* pattern = match_v && g_variant_is_of_type(match_v, G_VARIANT_TYPE_STRING)
                                  ? g_variant_get_string(match_v, NULL)
                                  : "";
        const char* level = level_v && g_variant_is_of_type(level_v, G_VARIANT_TYPE_STRING)
                                ? g_variant_get_string(level_v, NULL)
                                : "";
        valid = *name && strlen(name) <= 80 &&
                g_regex_match_simple("^[A-Za-z0-9_.-]{1,80}$", name, G_REGEX_OPTIMIZE, 0) &&
                !g_hash_table_contains(names, name) && *pattern && strlen(pattern) <= 512 &&
                g_utf8_validate(pattern, -1, NULL) && string_in(level, levels) && capabilities_v &&
                string_array_valid(capabilities_v, capabilities, TRUE, FALSE);
        if (valid) {
            g_autoptr(GError) regex_error = NULL;
            g_autoptr(GRegex) regex = g_regex_new(pattern, PERMISSION_REGEX_FLAGS, 0, &regex_error);
            valid = regex != NULL;
        }
        if (valid)
            g_hash_table_add(names, g_strdup(name));

        g_autoptr(GVariant) monitors = g_variant_lookup_value(rule, "monitors", NULL);
        if (valid && monitors) {
            valid = string_array_valid(monitors, NULL, TRUE, FALSE);
            for (gsize j = 0; valid && j < g_variant_n_children(monitors); j++) {
                g_autoptr(GVariant) boxed_monitor = g_variant_get_child_value(monitors, j);
                g_autoptr(GVariant) monitor = g_variant_get_variant(boxed_monitor);
                const char* monitor_name = g_variant_get_string(monitor, NULL);
                valid = strlen(monitor_name) <= 80 &&
                        g_regex_match_simple("^[A-Za-z0-9_.:-]{1,80}$", monitor_name,
                                             G_REGEX_OPTIMIZE, 0);
            }
            gboolean supports_monitor = FALSE;
            for (gsize j = 0; j < g_variant_n_children(capabilities_v); j++) {
                g_autoptr(GVariant) boxed_cap = g_variant_get_child_value(capabilities_v, j);
                g_autoptr(GVariant) cap = g_variant_get_variant(boxed_cap);
                const char* cap_name = g_variant_get_string(cap, NULL);
                supports_monitor |=
                    g_str_equal(cap_name, "screen-cast") || g_str_equal(cap_name, "remote-desktop");
            }
            valid = valid && supports_monitor;
        }

        g_autoptr(GVariant) device_list = g_variant_lookup_value(rule, "devices", NULL);
        if (valid && device_list) {
            valid = string_array_valid(device_list, devices, FALSE, FALSE);
            gboolean supports_devices = FALSE;
            for (gsize j = 0; j < g_variant_n_children(capabilities_v); j++) {
                g_autoptr(GVariant) boxed_cap = g_variant_get_child_value(capabilities_v, j);
                g_autoptr(GVariant) cap = g_variant_get_variant(boxed_cap);
                supports_devices |= g_str_equal(g_variant_get_string(cap, NULL), "remote-desktop");
            }
            valid = valid && supports_devices;
        }

        g_autoptr(GVariant) clipboard = g_variant_lookup_value(rule, "clipboard", NULL);
        if (valid && clipboard) {
            valid = g_variant_is_of_type(clipboard, G_VARIANT_TYPE_BOOLEAN);
            gboolean supports_clipboard = FALSE;
            for (gsize j = 0; j < g_variant_n_children(capabilities_v); j++) {
                g_autoptr(GVariant) boxed_cap = g_variant_get_child_value(capabilities_v, j);
                g_autoptr(GVariant) cap = g_variant_get_variant(boxed_cap);
                supports_clipboard |=
                    g_str_equal(g_variant_get_string(cap, NULL), "remote-desktop");
            }
            valid = valid && supports_clipboard;
        }
    }
    if (!valid)
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "permissions must contain a valid default and at most 256 unique, "
                            "well-formed capability rules");
    return valid;
}

gboolean gnoblin_permission_capability_supported(const char* capability) {
    return capability && string_in(capability, capabilities);
}

static GnoblinPermissionLevel parse_level(const char* level) {
    if (g_strcmp0(level, "ask") == 0)
        return GNOBLIN_PERMISSION_ASK;
    if (g_strcmp0(level, "allow") == 0)
        return GNOBLIN_PERMISSION_ALLOW;
    if (g_strcmp0(level, "deny") == 0)
        return GNOBLIN_PERMISSION_DENY;
    return GNOBLIN_PERMISSION_DEFAULT;
}

static void read_string_array(GVariant* value, char*** output) {
    if (!value || !g_variant_is_of_type(value, G_VARIANT_TYPE("av")))
        return;
    GPtrArray* strings = g_ptr_array_new_with_free_func(g_free);
    for (gsize i = 0; i < g_variant_n_children(value); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
        g_autoptr(GVariant) child = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(child, G_VARIANT_TYPE_STRING)) {
            g_ptr_array_unref(strings);
            return;
        }
        g_ptr_array_add(strings, g_variant_dup_string(child, NULL));
    }
    g_ptr_array_add(strings, NULL);
    *output = (char**)g_ptr_array_free(strings, FALSE);
}

GnoblinPermission gnoblin_permission_policy_evaluate(GVariant* document, const char* capability,
                                                     const char* identity) {
    GnoblinPermission permission = {.level = GNOBLIN_PERMISSION_DENY};
    if (!document || !string_in(capability, capabilities))
        return permission;
    g_autoptr(GVariant) policy = g_variant_lookup_value(document, "permissions", NULL);
    permission.level = GNOBLIN_PERMISSION_DEFAULT;
    const char* fallback = "default";
    if (policy)
        g_variant_lookup(policy, "default", "&s", &fallback);
    permission.level = parse_level(fallback);
    if (!valid_identity(identity)) {
        if (permission.level != GNOBLIN_PERMISSION_DENY)
            permission.level = GNOBLIN_PERMISSION_ASK;
        permission.rule = g_strdup("unverified-identity");
        return permission;
    }
    g_autoptr(GVariant) rules = policy ? g_variant_lookup_value(policy, "rules", NULL) : NULL;
    if (!rules || !g_variant_is_of_type(rules, G_VARIANT_TYPE("av")))
        return permission;
    for (gsize i = 0; i < g_variant_n_children(rules); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(rules, i);
        g_autoptr(GVariant) rule = g_variant_get_variant(boxed);
        const char *pattern = NULL, *name = NULL, *level = NULL;
        g_variant_lookup(rule, "match", "&s", &pattern);
        g_variant_lookup(rule, "name", "&s", &name);
        g_variant_lookup(rule, "level", "&s", &level);
        g_autoptr(GVariant) rule_caps = g_variant_lookup_value(rule, "capabilities", NULL);
        gboolean applies = FALSE;
        if (!pattern || !name || !level || !rule_caps ||
            !g_variant_is_of_type(rule_caps, G_VARIANT_TYPE("av")))
            continue;
        for (gsize j = 0; j < g_variant_n_children(rule_caps); j++) {
            g_autoptr(GVariant) boxed_cap = g_variant_get_child_value(rule_caps, j);
            g_autoptr(GVariant) cap = g_variant_get_variant(boxed_cap);
            if (g_variant_is_of_type(cap, G_VARIANT_TYPE_STRING) &&
                g_str_equal(g_variant_get_string(cap, NULL), capability)) {
                applies = TRUE;
                break;
            }
        }
        if (!applies)
            continue;
        g_autoptr(GError) regex_error = NULL;
        g_autoptr(GRegex) regex = g_regex_new(pattern, PERMISSION_REGEX_FLAGS, 0, &regex_error);
        if (!regex || !g_regex_match(regex, identity, 0, NULL))
            continue;
        permission.level = parse_level(level);
        g_free(permission.rule);
        permission.rule = g_strdup(name);
        g_clear_pointer(&permission.monitors, g_strfreev);
        permission.devices = 0;
        permission.clipboard = FALSE;
        g_autoptr(GVariant) monitors = g_variant_lookup_value(rule, "monitors", NULL);
        read_string_array(monitors, &permission.monitors);
        g_autoptr(GVariant) device_list = g_variant_lookup_value(rule, "devices", NULL);
        if (device_list && g_variant_is_of_type(device_list, G_VARIANT_TYPE("av"))) {
            for (gsize j = 0; j < g_variant_n_children(device_list); j++) {
                g_autoptr(GVariant) boxed_device = g_variant_get_child_value(device_list, j);
                g_autoptr(GVariant) device = g_variant_get_variant(boxed_device);
                if (!g_variant_is_of_type(device, G_VARIANT_TYPE_STRING))
                    continue;
                const char* device_name = g_variant_get_string(device, NULL);
                permission.devices |= g_str_equal(device_name, "keyboard")      ? 1
                                      : g_str_equal(device_name, "pointer")     ? 2
                                      : g_str_equal(device_name, "touchscreen") ? 4
                                                                                : 0;
            }
        }
        g_variant_lookup(rule, "clipboard", "b", &permission.clipboard);
        if (permission.level == GNOBLIN_PERMISSION_DENY)
            break;
    }
    return permission;
}

static void add_string_array(GVariantBuilder* dict, const char* key, const char* const* values) {
    GVariantBuilder array;
    g_variant_builder_init(&array, G_VARIANT_TYPE_STRING_ARRAY);
    for (guint i = 0; values[i]; i++)
        g_variant_builder_add(&array, "s", values[i]);
    g_variant_builder_add(dict, "{sv}", key, g_variant_builder_end(&array));
}

static void append_variant_array(GVariantBuilder* array, GVariant* value) {
    g_variant_builder_add_value(array, g_variant_new_variant(value));
}

GVariant* gnoblin_permission_policy_list(GVariant* document, const char* config_path) {
    GVariantBuilder result, policy_out, rules_out;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&policy_out, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) policy =
        document ? g_variant_lookup_value(document, "permissions", NULL) : NULL;
    const char* fallback = "default";
    if (policy)
        g_variant_lookup(policy, "default", "&s", &fallback);
    g_variant_builder_add(&policy_out, "{sv}", "default", g_variant_new_string(fallback));
    g_variant_builder_init(&rules_out, G_VARIANT_TYPE("av"));
    g_autoptr(GVariant) rules = policy ? g_variant_lookup_value(policy, "rules", NULL) : NULL;
    if (rules && g_variant_is_of_type(rules, G_VARIANT_TYPE("av"))) {
        for (gsize i = 0; i < g_variant_n_children(rules); i++) {
            g_autoptr(GVariant) boxed = g_variant_get_child_value(rules, i);
            g_autoptr(GVariant) rule = g_variant_get_variant(boxed);
            append_variant_array(&rules_out, g_variant_ref(rule));
        }
    }
    g_variant_builder_add(&policy_out, "{sv}", "rules", g_variant_builder_end(&rules_out));
    g_variant_builder_add(&result, "{sv}", "policy", g_variant_builder_end(&policy_out));
    add_string_array(&result, "capabilities", capabilities);
    add_string_array(&result, "levels", levels);
    g_variant_builder_add(&result, "{sv}", "path",
                          g_variant_new_string(config_path ? config_path : ""));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

GVariant* gnoblin_permission_policy_snapshot(GVariant* document, guint64 revision) {
    g_autoptr(GVariant) listed = gnoblin_permission_policy_list(document, NULL);
    g_autoptr(GVariant) policy = g_variant_lookup_value(listed, "policy", G_VARIANT_TYPE_VARDICT);
    if (!policy)
        return NULL;
    GVariantDict snapshot;
    g_variant_dict_init(&snapshot, policy);
    g_variant_dict_insert(&snapshot, "revision", "x", (gint64)revision);
    return g_variant_ref_sink(g_variant_dict_end(&snapshot));
}

static gboolean is_gnoblin_session(void) {
    const char* desktop = g_getenv("XDG_CURRENT_DESKTOP");
    g_auto(GStrv) desktops = g_strsplit(desktop ? desktop : "", ":", -1);
    for (guint i = 0; desktops[i]; i++)
        if (g_ascii_strcasecmp(desktops[i], "gnoblin") == 0)
            return TRUE;
    return g_strcmp0(g_getenv("GNOME_SHELL_SESSION_MODE"), "gnoblin") == 0;
}

GnoblinPermission gnoblin_permission_check(GDBusMethodInvocation* invocation,
                                           const char* capability, const char* identity) {
    GnoblinPermission permission = {.level = GNOBLIN_PERMISSION_DEFAULT};
    g_autoptr(GVariant) reply = NULL;
    g_autoptr(GError) error = NULL;
    const char *level, *rule;
    if (!is_gnoblin_session())
        return permission;
    permission.level = GNOBLIN_PERMISSION_DENY;
    reply = g_dbus_connection_call_sync(
        g_dbus_method_invocation_get_connection(invocation), GNOBLIN_COMPOSITOR_NAME,
        GNOBLIN_COMPOSITOR_PATH, GNOBLIN_COMPOSITOR_IFACE, "CheckPermission",
        g_variant_new("(ss)", capability, identity ? identity : ""), G_VARIANT_TYPE("(ssasub)"),
        G_DBUS_CALL_FLAGS_NO_AUTO_START, 2000, NULL, &error);
    if (!reply) {
        g_warning("gnoblin: native permission policy unavailable: %s", error->message);
        return permission;
    }
    g_variant_get(reply, "(&s&s^asub)", &level, &rule, &permission.monitors, &permission.devices,
                  &permission.clipboard);
    if (g_str_equal(level, "default"))
        permission.level = GNOBLIN_PERMISSION_DEFAULT;
    else if (g_str_equal(level, "ask"))
        permission.level = GNOBLIN_PERMISSION_ASK;
    else if (g_str_equal(level, "allow") && valid_identity(identity))
        permission.level = GNOBLIN_PERMISSION_ALLOW;
    else if (g_str_equal(level, "deny"))
        permission.level = GNOBLIN_PERMISSION_DENY;
    permission.rule = g_strdup(rule);
    return permission;
}

GnoblinPermission gnoblin_permission_for_request(GDBusMethodInvocation* invocation,
                                                 const char* capability, const char* app_id,
                                                 const char* handle) {
    g_autofree char* identity = gnoblin_portal_requester_identity(
        g_dbus_method_invocation_get_connection(invocation),
        g_dbus_method_invocation_get_sender(invocation), app_id, handle);
    return gnoblin_permission_check(invocation, capability, identity);
}
