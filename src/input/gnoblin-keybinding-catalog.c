#include "gnoblin-keybinding-catalog.h"

#include <json-glib/json-glib.h>
#include <string.h>

struct _GnoblinKeybindingCatalog {
    GHashTable* groups;
    GHashTable* actions;
};

static const gchar* const keybinding_groups[] = {"wm", "mutter", "wayland"};

static void keybinding_action_free(gpointer data) {
    GnoblinKeybindingAction* action = data;
    if (!action)
        return;
    g_free(action->key);
    g_free(action->description);
    g_strfreev(action->default_bindings);
    g_free(action);
}

void gnoblin_keybinding_catalog_free(GnoblinKeybindingCatalog* catalog) {
    if (!catalog)
        return;
    g_clear_pointer(&catalog->groups, g_hash_table_unref);
    g_clear_pointer(&catalog->actions, g_hash_table_unref);
    g_free(catalog);
}

static gchar* native_keybinding_catalog_path(void) {
    const gchar* prefix = g_getenv("GNOBLIN_PREFIX");
    if (prefix && *prefix) {
        g_autofree gchar* candidate =
            g_build_filename(prefix, "share", "gnoblin", "native-keybindings.json", NULL);
        if (g_file_test(candidate, G_FILE_TEST_IS_REGULAR))
            return g_steal_pointer(&candidate);
    }

    const gchar* const* data_dirs = g_get_system_data_dirs();
    for (guint i = 0; data_dirs[i]; i++) {
        g_autofree gchar* candidate =
            g_build_filename(data_dirs[i], "gnoblin", "native-keybindings.json", NULL);
        if (g_file_test(candidate, G_FILE_TEST_IS_REGULAR))
            return g_steal_pointer(&candidate);
    }
    return NULL;
}

static gboolean catalog_error(GError** error, const gchar* format, const gchar* detail) {
    g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, format, detail);
    return FALSE;
}

static gboolean valid_group(const gchar* group) {
    if (!group)
        return FALSE;
    for (guint i = 0; i < G_N_ELEMENTS(keybinding_groups); i++)
        if (g_str_equal(group, keybinding_groups[i]))
            return TRUE;
    return FALSE;
}

static gchar* action_identity(const gchar* group, const gchar* key) {
    return g_strdup_printf("%s.%s", group, key);
}

static gboolean valid_key(const gchar* key) {
    return key && *key && strlen(key) <= 128 &&
           g_regex_match_simple("^[a-z0-9]+(?:-[a-z0-9]+)*$", key, 0, 0);
}

static gint compare_actions(gconstpointer left, gconstpointer right) {
    const GnoblinKeybindingAction* action_left = *(GnoblinKeybindingAction* const*)left;
    const GnoblinKeybindingAction* action_right = *(GnoblinKeybindingAction* const*)right;
    return g_strcmp0(action_left->key, action_right->key);
}

static gboolean read_description(JsonObject* object, gchar** description, GError** error,
                                 const gchar* action_id) {
    JsonNode* node = json_object_get_member(object, "description");
    if (node && JSON_NODE_HOLDS_NULL(node)) {
        *description = NULL;
        return TRUE;
    }
    if (!node || !JSON_NODE_HOLDS_VALUE(node) || json_node_get_value_type(node) != G_TYPE_STRING)
        return catalog_error(error, "Mutter keybinding action '%s' has an invalid description",
                             action_id);
    const gchar* value = json_node_get_string(node);
    if (value && !g_utf8_validate(value, -1, NULL))
        return catalog_error(error, "Mutter keybinding action '%s' has a non-UTF-8 description",
                             action_id);
    *description = g_strdup(value);
    return TRUE;
}

static GStrv read_default_bindings(JsonObject* object, GError** error, const gchar* action_id) {
    JsonNode* node = json_object_get_member(object, "default_bindings");
    if (!node || !JSON_NODE_HOLDS_ARRAY(node)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "Mutter keybinding action '%s' default_bindings must be an array", action_id);
        return NULL;
    }

    JsonArray* array = json_node_get_array(node);
    gsize length = json_array_get_length(array);
    gchar** bindings = g_new0(gchar*, length + 1);
    for (gsize i = 0; i < length; i++) {
        JsonNode* binding_node = json_array_get_element(array, i);
        if (!binding_node || !JSON_NODE_HOLDS_VALUE(binding_node) ||
            json_node_get_value_type(binding_node) != G_TYPE_STRING) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "Mutter keybinding action '%s' has a non-string default binding",
                        action_id);
            g_strfreev(bindings);
            return NULL;
        }
        const gchar* binding = json_node_get_string(binding_node);
        if (!binding || strlen(binding) > 160 || !g_utf8_validate(binding, -1, NULL)) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "Mutter keybinding action '%s' has an invalid default binding", action_id);
            g_strfreev(bindings);
            return NULL;
        }
        bindings[i] = g_strdup(binding);
    }
    return bindings;
}

static gboolean append_action(GnoblinKeybindingCatalog* catalog, const gchar* group,
                              const gchar* member_key, JsonNode* node, GError** error) {
    if (!valid_key(member_key)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "Mutter keybinding catalog group '%s' has an invalid action key", group);
        return FALSE;
    }
    if (!node || !JSON_NODE_HOLDS_OBJECT(node)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "Mutter keybinding action '%s.%s' must be an object", group, member_key);
        return FALSE;
    }

    JsonObject* object = json_node_get_object(node);
    JsonNode* key_node = json_object_get_member(object, "key");
    const gchar* key = key_node && JSON_NODE_HOLDS_VALUE(key_node) &&
                               json_node_get_value_type(key_node) == G_TYPE_STRING
                           ? json_node_get_string(key_node)
                           : NULL;
    g_autofree gchar* identity = action_identity(group, member_key);
    if (json_object_get_size(object) != 3 || !key || !g_str_equal(key, member_key)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "Mutter keybinding action '%s' has invalid descriptor fields", identity);
        return FALSE;
    }

    GnoblinKeybindingAction* action = g_new0(GnoblinKeybindingAction, 1);
    action->key = g_strdup(key);
    if (!read_description(object, &action->description, error, identity)) {
        keybinding_action_free(action);
        return FALSE;
    }
    action->default_bindings = read_default_bindings(object, error, identity);
    if (!action->default_bindings) {
        keybinding_action_free(action);
        return FALSE;
    }

    GPtrArray* group_actions = g_hash_table_lookup(catalog->groups, group);
    g_ptr_array_add(group_actions, action);
    g_hash_table_insert(catalog->actions, g_steal_pointer(&identity), action);
    return TRUE;
}

static gboolean read_group(GnoblinKeybindingCatalog* catalog, JsonObject* groups,
                           const gchar* group, GError** error) {
    JsonNode* node = json_object_get_member(groups, group);
    if (!node || !JSON_NODE_HOLDS_OBJECT(node)) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "Mutter keybinding catalog group '%s' must be an object", group);
        return FALSE;
    }

    JsonObject* actions = json_node_get_object(node);
    GList* members = json_object_get_members(actions);
    for (GList* member = members; member; member = member->next) {
        const gchar* key = member->data;
        if (!append_action(catalog, group, key, json_object_get_member(actions, key), error)) {
            g_list_free(members);
            return FALSE;
        }
    }
    g_list_free(members);

    GPtrArray* group_actions = g_hash_table_lookup(catalog->groups, group);
    g_ptr_array_sort(group_actions, compare_actions);
    return TRUE;
}

GnoblinKeybindingCatalog* gnoblin_keybinding_catalog_load(GError** error) {
    g_autofree gchar* path = native_keybinding_catalog_path();
    if (!path) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_NOENT,
                            "Gnoblin keybinding catalog is missing; build or install the Gnoblin "
                            "Mutter component first");
        return NULL;
    }

    g_autoptr(JsonParser) parser = json_parser_new();
    if (!json_parser_load_from_file(parser, path, error))
        return NULL;

    JsonNode* root = json_parser_get_root(parser);
    if (!root || !JSON_NODE_HOLDS_OBJECT(root)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Mutter keybinding catalog root must be an object");
        return NULL;
    }
    JsonObject* root_object = json_node_get_object(root);
    JsonNode* format_node = json_object_get_member(root_object, "format");
    JsonNode* groups_node = json_object_get_member(root_object, "groups");
    GType format_type = format_node && JSON_NODE_HOLDS_VALUE(format_node)
                            ? json_node_get_value_type(format_node)
                            : G_TYPE_INVALID;
    if (json_object_get_size(root_object) != 2 || !format_node ||
        !JSON_NODE_HOLDS_VALUE(format_node) ||
        (format_type != G_TYPE_INT && format_type != G_TYPE_INT64) ||
        json_node_get_int(format_node) != 2 || !groups_node ||
        !JSON_NODE_HOLDS_OBJECT(groups_node)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Mutter keybinding catalog has an unsupported format");
        return NULL;
    }

    JsonObject* groups = json_node_get_object(groups_node);
    if (json_object_get_size(groups) != G_N_ELEMENTS(keybinding_groups)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "Mutter keybinding catalog must contain exactly three groups");
        return NULL;
    }

    GnoblinKeybindingCatalog* catalog = g_new0(GnoblinKeybindingCatalog, 1);
    catalog->groups =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, (GDestroyNotify)g_ptr_array_unref);
    catalog->actions = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    for (guint i = 0; i < G_N_ELEMENTS(keybinding_groups); i++) {
        const gchar* group = keybinding_groups[i];
        if (!json_object_has_member(groups, group)) {
            g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                        "Mutter keybinding catalog is missing group '%s'", group);
            gnoblin_keybinding_catalog_free(catalog);
            return NULL;
        }
        g_hash_table_insert(catalog->groups, g_strdup(group),
                            g_ptr_array_new_with_free_func(keybinding_action_free));
    }

    for (guint i = 0; i < G_N_ELEMENTS(keybinding_groups); i++) {
        if (!read_group(catalog, groups, keybinding_groups[i], error)) {
            gnoblin_keybinding_catalog_free(catalog);
            return NULL;
        }
    }
    return catalog;
}

const GPtrArray* gnoblin_keybinding_catalog_get_group(const GnoblinKeybindingCatalog* catalog,
                                                      const gchar* group) {
    if (!catalog || !valid_group(group))
        return NULL;
    return g_hash_table_lookup(catalog->groups, group);
}

const GnoblinKeybindingAction*
gnoblin_keybinding_catalog_lookup(const GnoblinKeybindingCatalog* catalog, const gchar* group,
                                  const gchar* key) {
    if (!catalog || !valid_group(group) || !key)
        return NULL;
    g_autofree gchar* identity = action_identity(group, key);
    return g_hash_table_lookup(catalog->actions, identity);
}
