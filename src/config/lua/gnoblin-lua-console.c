#include "gnoblin-lua-internal.h"

static void console_append_escaped(GString* output, const char* text, gsize length) {
    for (gsize offset = 0; offset < length && output->len < MAX_CONSOLE_RESULT_BYTES;) {
        unsigned char character = (unsigned char)text[offset];
        if (character == '\n') {
            g_string_append(output, "\\n");
            offset++;
        } else if (character == '\r') {
            g_string_append(output, "\\r");
            offset++;
        } else if (character == '\t') {
            g_string_append(output, "\\t");
            offset++;
        } else if (character < 0x20 || character == 0x7f) {
            g_string_append_printf(output, "\\x%02x", character);
            offset++;
        } else {
            gunichar codepoint = g_utf8_get_char_validated(text + offset, length - offset);
            if (codepoint == (gunichar)-1 || codepoint == (gunichar)-2) {
                g_string_append_printf(output, "\\x%02x", character);
                offset++;
            } else {
                gsize width = g_unichar_to_utf8(codepoint, NULL);
                if (output->len + width > MAX_CONSOLE_RESULT_BYTES)
                    break;
                g_string_append_len(output, text + offset, width);
                offset += width;
            }
        }
    }
}

static void console_truncate_utf8(GString* output, gsize length) {
    if (length >= output->len)
        return;
    while (length > 0 && (((unsigned char)output->str[length] & 0xc0) == 0x80))
        length--;
    g_string_truncate(output, length);
}

/* Rendering deliberately avoids Lua conversion hooks. A console result must
 * not execute user __tostring/__pairs code after the submitted chunk has
 * returned, otherwise presentation could bypass the instruction budget. */
static void console_append_value(GString* output, lua_State* state, int index) {
    index = lua_absindex(state, index);
    switch (lua_type(state, index)) {
    case LUA_TNIL:
        g_string_append(output, "nil");
        break;
    case LUA_TBOOLEAN:
        g_string_append(output, lua_toboolean(state, index) ? "true" : "false");
        break;
    case LUA_TNUMBER:
        if (lua_isinteger(state, index))
            g_string_append_printf(output, "%" G_GINT64_FORMAT,
                                   (gint64)lua_tointeger(state, index));
        else
            g_string_append_printf(output, "%.17g", lua_tonumber(state, index));
        break;
    case LUA_TSTRING: {
        size_t length = 0;
        const char* text = lua_tolstring(state, index, &length);
        g_string_append_c(output, '"');
        console_append_escaped(output, text, length);
        if (length > 0 && output->len >= MAX_CONSOLE_RESULT_BYTES)
            g_string_append(output, "…");
        g_string_append_c(output, '"');
        break;
    }
    case LUA_TTABLE:
        g_string_append_printf(output, "<table: %" G_GINT64_FORMAT " array entries>",
                               (gint64)lua_rawlen(state, index));
        break;
    default:
        g_string_append_printf(output, "<%s>", lua_typename(state, lua_type(state, index)));
        break;
    }
}

static void console_append_inspection_value(GString* output, lua_State* state, int index,
                                            ConsoleInspection* inspection) {
    gsize before = output->len;
    console_append_value(output, state, index);
    gsize added = output->len - before;
    if (added > MAX_CONSOLE_INSPECTION_VALUE_BYTES) {
        console_truncate_utf8(output, before + MAX_CONSOLE_INSPECTION_VALUE_BYTES);
        g_string_append(output, "…");
        added = output->len - before;
    }
    if (inspection->text_bytes + added > MAX_CONSOLE_INSPECTION_TEXT_BYTES) {
        gsize available = inspection->text_bytes < MAX_CONSOLE_INSPECTION_TEXT_BYTES
                              ? MAX_CONSOLE_INSPECTION_TEXT_BYTES - inspection->text_bytes
                              : 0;
        console_truncate_utf8(output, before + available);
        g_string_append(output, "…");
        added = output->len - before;
    }
    inspection->text_bytes += added;
}

/* Snapshot userdata stores immutable fields in uservalue 1. Inspect that
 * backing table directly: `__index` also exposes operation methods and must
 * never run while displaying a console result. */
static gboolean console_push_inspection_table(lua_State* state, int index) {
    index = lua_absindex(state, index);
    if (lua_istable(state, index)) {
        lua_pushvalue(state, index);
        return TRUE;
    }
    if (!lua_isuserdata(state, index) ||
        !luaL_testudata(state, index, LUA_READONLY_SNAPSHOT_METATABLE))
        return FALSE;
    lua_getiuservalue(state, index, 1);
    if (lua_istable(state, -1))
        return TRUE;
    lua_pop(state, 1);
    return FALSE;
}

/* Number of entries in a table the inspector does not walk, so the console can say how many it
 * left out instead of a bare ellipsis. */
static gint32 console_count_entries(lua_State* state, int table) {
    gint32 count = 0;
    lua_pushnil(state);
    while (count < G_MAXINT32 && lua_next(state, table)) {
        count++;
        lua_pop(state, 1);
    }
    if (lua_gettop(state) > table)
        lua_pop(state, 1);
    return count;
}

static GVariant* console_inspection_node(lua_State* state, const char* label, int index,
                                         guint depth, ConsoleInspection* inspection) {
    index = lua_absindex(state, index);
    gboolean has_more = FALSE;
    gint32 more = 0;
    g_autoptr(GString) value = g_string_new(NULL);
    console_append_inspection_value(value, state, index, inspection);

    GVariantBuilder children;
    g_variant_builder_init(&children, G_VARIANT_TYPE("aa{sv}"));
    if (inspection->nodes >= MAX_CONSOLE_INSPECTION_NODES ||
        depth >= MAX_CONSOLE_INSPECTION_DEPTH) {
        has_more = lua_istable(state, index) || lua_isuserdata(state, index);
        if (has_more && console_push_inspection_table(state, index)) {
            more = console_count_entries(state, lua_absindex(state, -1));
            lua_pop(state, 1);
        }
    } else if (console_push_inspection_table(state, index)) {
        int table = lua_absindex(state, -1);
        const void* identity = lua_topointer(state, table);
        if (!identity || g_hash_table_contains(inspection->seen, identity)) {
            /* Already shown above, so listing it again would loop. */
            g_string_append(value, " (circular)");
        } else {
            g_hash_table_add(inspection->seen, (gpointer)identity);
            guint child_count = 0;
            lua_pushnil(state);
            while (lua_next(state, table)) {
                if (child_count >= MAX_CONSOLE_INSPECTION_CHILDREN ||
                    inspection->nodes >= MAX_CONSOLE_INSPECTION_NODES) {
                    has_more = TRUE;
                    /* Count what is left so the console can say how much it did not list. */
                    more = 1;
                    lua_pop(state, 1); /* value; the key stays for lua_next */
                    while (more < G_MAXINT32 && lua_next(state, table)) {
                        more++;
                        lua_pop(state, 1);
                    }
                    break;
                }
                g_autoptr(GString) child_label = g_string_new(NULL);
                console_append_inspection_value(child_label, state, -2, inspection);
                inspection->nodes++;
                g_autoptr(GVariant) child =
                    console_inspection_node(state, child_label->str, -1, depth + 1, inspection);
                g_variant_builder_add_value(&children, child);
                child_count++;
                lua_pop(state, 1); /* value; lua_next retains key */
            }
            /* A normal exhausted lua_next removes its key. A capped traversal
             * still has its key; discard it before restoring the table stack. */
            if (lua_gettop(state) > table)
                lua_pop(state, 1);
        }
        lua_pop(state, 1); /* backing table */
    } else if (lua_istable(state, index) || lua_isuserdata(state, index)) {
        has_more = TRUE;
    }

    GVariantBuilder node;
    g_variant_builder_init(&node, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&node, "{sv}", "label", g_variant_new_string(label));
    g_variant_builder_add(&node, "{sv}", "value", g_variant_new_string(value->str));
    g_variant_builder_add(&node, "{sv}", "children", g_variant_builder_end(&children));
    g_variant_builder_add(&node, "{sv}", "has_more", g_variant_new_boolean(has_more));
    g_variant_builder_add(&node, "{sv}", "more", g_variant_new_int32(more));
    return g_variant_ref_sink(g_variant_builder_end(&node));
}

static GVariant* console_inspection(lua_State* state, int first, int values) {
    GVariantBuilder nodes;
    g_variant_builder_init(&nodes, G_VARIANT_TYPE("aa{sv}"));
    ConsoleInspection inspection = {
        .nodes = 0,
        .text_bytes = 0,
        .seen = g_hash_table_new(g_direct_hash, g_direct_equal),
    };
    for (int i = 0; i < values && i < 8; i++) {
        inspection.nodes++;
        g_autofree char* label = g_strdup_printf("%d", i + 1);
        g_autoptr(GVariant) node = console_inspection_node(state, label, first + i, 0, &inspection);
        g_variant_builder_add_value(&nodes, node);
    }
    if (values > 8) {
        GVariantBuilder omitted, no_children;
        g_variant_builder_init(&omitted, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_init(&no_children, G_VARIANT_TYPE("aa{sv}"));
        g_variant_builder_add(&omitted, "{sv}", "label", g_variant_new_string("…"));
        g_variant_builder_add(&omitted, "{sv}", "value", g_variant_new_string("more values"));
        g_variant_builder_add(&omitted, "{sv}", "children", g_variant_builder_end(&no_children));
        g_variant_builder_add(&omitted, "{sv}", "has_more", g_variant_new_boolean(TRUE));
        g_variant_builder_add_value(&nodes, g_variant_builder_end(&omitted));
    }
    g_hash_table_unref(inspection.seen);
    return g_variant_ref_sink(g_variant_builder_end(&nodes));
}

static void console_completion_free(gpointer data) {
    ConsoleCompletion* completion = data;
    if (!completion)
        return;
    g_free(completion->label);
    g_free(completion->detail);
    g_free(completion->documentation);
    g_free(completion);
}

static gint console_completion_compare(gconstpointer first, gconstpointer second) {
    const ConsoleCompletion* const* left = first;
    const ConsoleCompletion* const* right = second;
    return g_strcmp0((*left)->label, (*right)->label);
}

static gboolean console_completion_identifier(const char* text, gsize length) {
    if (length == 0 || !(g_ascii_isalpha(text[0]) || text[0] == '_'))
        return FALSE;
    for (gsize i = 1; i < length; i++) {
        if (!(g_ascii_isalnum(text[i]) || text[i] == '_'))
            return FALSE;
    }
    return TRUE;
}

/* The panel replaces precisely the text after these separators. Keep parsing
 * coupled to that contract so completion never rewrites a preceding chunk. */
static const char* console_completion_token_start(const char* source) {
    const char* start = source;
    for (const char* cursor = source; *cursor; cursor++) {
        if (strchr(" \t\n;(){}[]", *cursor))
            start = cursor + 1;
    }
    return start;
}

/* Parse only an identifier chain. In particular, no brackets, calls, string
 * indexing, or operators reach the Lua VM. `segments` resolves the receiver
 * and `prefix` selects fields from it. */
static gboolean console_completion_parse(const char* source, GPtrArray* segments, char** prefix,
                                         gboolean* method) {
    const char* token = console_completion_token_start(source);
    gsize length = strlen(token);
    if (length > MAX_CONSOLE_COMPLETION_TOKEN_BYTES)
        return FALSE;
    const char* colon = strchr(token, ':');
    if (colon && strchr(colon + 1, ':'))
        return FALSE;
    *method = colon != NULL;
    g_autofree char* receiver = colon ? g_strndup(token, colon - token) : g_strdup(token);
    const char* final = colon ? colon + 1 : receiver;
    if (colon && strchr(final, '.'))
        return FALSE;
    if (colon && !*receiver)
        return FALSE;
    if (!colon && length == 0) {
        *prefix = g_strdup("");
        return TRUE;
    }
    const char* part = receiver;
    while (TRUE) {
        const char* dot = strchr(part, '.');
        gsize part_length = dot ? (gsize)(dot - part) : strlen(part);
        if (dot) {
            if (segments->len >= MAX_CONSOLE_COMPLETION_SEGMENTS ||
                !console_completion_identifier(part, part_length))
                return FALSE;
            g_ptr_array_add(segments, g_strndup(part, part_length));
            part = dot + 1;
            continue;
        }
        if (colon) {
            if (!console_completion_identifier(part, part_length) ||
                (!console_completion_identifier(final, strlen(final)) && *final))
                return FALSE;
            g_ptr_array_add(segments, g_strndup(part, part_length));
            *prefix = g_strdup(final);
        } else {
            if (!console_completion_identifier(part, part_length) && part_length != 0)
                return FALSE;
            *prefix = g_strndup(part, part_length);
        }
        return TRUE;
    }
}

/* Raw field reads deliberately bypass __index. Readonly snapshot userdata has
 * two explicitly owned uservalues: data first, then Gnoblin method members. */
static gboolean console_completion_push_field(lua_State* state, int object, const char* field) {
    object = lua_absindex(state, object);
    if (lua_istable(state, object)) {
        lua_pushstring(state, field);
        lua_rawget(state, object);
        if (!lua_isnil(state, -1))
            return TRUE;
        lua_pop(state, 1);
        return FALSE;
    }
    if (lua_type(state, object) != LUA_TUSERDATA ||
        !luaL_testudata(state, object, LUA_READONLY_SNAPSHOT_METATABLE))
        return FALSE;
    for (int slot = 1; slot <= 2; slot++) {
        if (lua_getiuservalue(state, object, slot) != LUA_TTABLE) {
            lua_pop(state, 1);
            continue;
        }
        lua_pushstring(state, field);
        lua_rawget(state, -2);
        lua_remove(state, -2);
        if (!lua_isnil(state, -1))
            return TRUE;
        lua_pop(state, 1);
    }
    return FALSE;
}

static void console_completion_add(GPtrArray* completions, GHashTable* seen, const char* receiver,
                                   const char* key, gsize key_length, lua_State* state, int value,
                                   gboolean readonly, gboolean method) {
    if (!g_utf8_validate(key, key_length, NULL) || (method && !lua_isfunction(state, value)) ||
        g_hash_table_contains(seen, key) || completions->len >= MAX_CONSOLE_COMPLETIONS)
        return;
    g_hash_table_add(seen, g_strdup(key));
    ConsoleCompletion* completion = g_new0(ConsoleCompletion, 1);
    completion->label = receiver && *receiver
                            ? g_strdup_printf(method ? "%s:%s" : "%s.%s", receiver, key)
                            : g_strdup(key);
    completion->detail = g_strdup_printf("Lua %s", lua_typename(state, lua_type(state, value)));
    completion->documentation =
        g_strdup(readonly ? "Read-only Gnoblin snapshot member" : "Live Lua environment member");
    g_ptr_array_add(completions, completion);
}

static gboolean console_completion_key_matches(const char* key, gsize length, const char* prefix) {
    gsize prefix_length = strlen(prefix);
    return key && length <= MAX_CONSOLE_COMPLETION_TOKEN_BYTES && !memchr(key, '\0', length) &&
           prefix_length <= length && memcmp(key, prefix, prefix_length) == 0 &&
           console_completion_identifier(key, length);
}

static void console_completion_collect_table(lua_State* state, int table, const char* receiver,
                                             const char* prefix, gboolean readonly, gboolean method,
                                             GPtrArray* completions, GHashTable* seen,
                                             guint* visits) {
    int base = lua_gettop(state);
    table = lua_absindex(state, table);
    lua_pushnil(state);
    while (*visits < MAX_CONSOLE_COMPLETION_TABLE_VISITS && lua_next(state, table)) {
        (*visits)++;
        if (lua_type(state, -2) == LUA_TSTRING) {
            size_t length = 0;
            const char* key = lua_tolstring(state, -2, &length);
            if (console_completion_key_matches(key, length, prefix))
                console_completion_add(completions, seen, receiver, key, length, state, -1,
                                       readonly, method);
        }
        lua_pop(state, 1);
    }
    lua_settop(state, base);
}

static char* console_completion_receiver(GPtrArray* segments) {
    g_autoptr(GString) receiver = g_string_new(NULL);
    for (guint i = 0; i < segments->len; i++) {
        if (i)
            g_string_append_c(receiver, '.');
        g_string_append(receiver, g_ptr_array_index(segments, i));
    }
    return g_string_free(g_steal_pointer(&receiver), FALSE);
}

GVariant* gnoblin_config_complete_console(const char* source, GError** error) {
    if (!source || strlen(source) > MAX_CONSOLE_SOURCE_BYTES ||
        !g_utf8_validate(source, -1, NULL)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "console completion source must be UTF-8 and no more than 16 KiB");
        return NULL;
    }
    if (pending_runtime || deferred_runtime || !active_runtime) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "Gnoblin Lua completion is unavailable while configuration reloads");
        return NULL;
    }
    LuaRuntime* runtime = active_runtime;
    LuaConfig* config = &runtime->config;
    if (config->dispatching || config->api_calling) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "Gnoblin Lua completion is busy processing another request");
        return NULL;
    }

    g_autoptr(GPtrArray) segments = g_ptr_array_new_with_free_func(g_free);
    g_autofree char* prefix = NULL;
    gboolean method = FALSE;
    if (!console_completion_parse(source, segments, &prefix, &method)) {
        GVariantBuilder empty, result;
        g_variant_builder_init(&empty, G_VARIANT_TYPE("aa{sv}"));
        g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&result, "{sv}", "completions", g_variant_builder_end(&empty));
        return g_variant_ref_sink(g_variant_builder_end(&result));
    }

    lua_State* state = runtime->state;
    int base = lua_gettop(state);
    guint actions = config->runtime_actions ? config->runtime_actions->len : 0;
    guint operations = config->operations ? g_hash_table_size(config->operations) : 0;
    config->api_calling = TRUE;
    lua_rawgeti(state, LUA_REGISTRYINDEX, LUA_RIDX_GLOBALS);
    gboolean resolved = lua_istable(state, -1);
    for (guint i = 0; resolved && i < segments->len; i++) {
        const char* segment = g_ptr_array_index(segments, i);
        if (!console_completion_push_field(state, -1, segment))
            resolved = FALSE;
        else
            lua_remove(state, -2);
    }

    g_autoptr(GPtrArray) completions = g_ptr_array_new_with_free_func(console_completion_free);
    g_autoptr(GHashTable) seen = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    guint visits = 0;
    g_autofree char* receiver = NULL;
    if (segments->len)
        receiver = console_completion_receiver(segments);
    if (resolved && lua_istable(state, -1)) {
        console_completion_collect_table(state, -1, receiver, prefix, FALSE, method, completions,
                                         seen, &visits);
    } else if (resolved && lua_type(state, -1) == LUA_TUSERDATA &&
               luaL_testudata(state, -1, LUA_READONLY_SNAPSHOT_METATABLE)) {
        /* Snapshot fields use dot notation; its callable method table uses colon
         * notation so completion preserves the implicit receiver contract. */
        int slot = method ? 2 : 1;
        if (lua_getiuservalue(state, -1, slot) == LUA_TTABLE)
            console_completion_collect_table(state, -1, receiver, prefix, TRUE, method, completions,
                                             seen, &visits);
        lua_pop(state, 1);
    }
    g_ptr_array_sort(completions, console_completion_compare);

    lua_settop(state, base);
    config->api_calling = FALSE;
    if ((config->runtime_actions && config->runtime_actions->len != actions) ||
        (config->operations && g_hash_table_size(config->operations) != operations)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                            "Lua completion attempted to queue a runtime operation");
        return NULL;
    }

    GVariantBuilder values, result;
    g_variant_builder_init(&values, G_VARIANT_TYPE("aa{sv}"));
    for (guint i = 0; i < completions->len; i++) {
        ConsoleCompletion* completion = g_ptr_array_index(completions, i);
        GVariantBuilder item;
        g_variant_builder_init(&item, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&item, "{sv}", "label", g_variant_new_string(completion->label));
        g_variant_builder_add(&item, "{sv}", "detail", g_variant_new_string(completion->detail));
        g_variant_builder_add(&item, "{sv}", "documentation",
                              g_variant_new_string(completion->documentation));
        g_variant_builder_add_value(&values, g_variant_builder_end(&item));
    }
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "completions", g_variant_builder_end(&values));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static void console_reject_actions(LuaRuntime* runtime, guint action_start, const char* reason) {
    LuaConfig* config = &runtime->config;
    if (action_start >= config->runtime_actions->len)
        return;
    g_autoptr(GPtrArray) dropped = g_ptr_array_new_with_free_func((GDestroyNotify)g_variant_unref);
    for (guint i = action_start; i < config->runtime_actions->len; i++)
        g_ptr_array_add(dropped, g_variant_ref(g_ptr_array_index(config->runtime_actions, i)));
    g_ptr_array_set_size(config->runtime_actions, action_start);
    config->rejecting_operations = TRUE;
    for (guint i = 0; i < dropped->len; i++)
        fail_dropped_operation(runtime, g_ptr_array_index(dropped, i), reason);
    config->rejecting_operations = FALSE;
}

GVariant* gnoblin_config_eval_console(const char* source, GError** error) {
    if (!source || !*source || strlen(source) > MAX_CONSOLE_SOURCE_BYTES ||
        !g_utf8_validate(source, -1, NULL)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                            "console source must be non-empty UTF-8 and no more than 16 KiB");
        return NULL;
    }
    if (pending_runtime || !active_runtime) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "Gnoblin Lua console is unavailable while configuration reloads");
        return NULL;
    }
    LuaRuntime* runtime = active_runtime;
    LuaConfig* config = &runtime->config;
    if (config->dispatching || config->api_calling) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "Gnoblin Lua console is busy processing another request");
        return NULL;
    }
    if (config->runtime_actions->len >= 256) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_NOSPC,
                            "too many pending Gnoblin session actions");
        return NULL;
    }

    lua_State* state = runtime->state;
    int base = lua_gettop(state);
    guint action_start = config->runtime_actions->len;
    config->actions_in_dispatch = 0;
    config->api_calling = TRUE;
    int steps = MAX_CONSOLE_STEPS / 1000;
    memcpy(lua_getextraspace(state), &steps, sizeof steps);

    /* Prefer normal Lua chunks, then make a bare expression convenient for a
     * prompt. The second parse has no side effects because the first only
     * compiles source. */
    int status = luaL_loadbufferx(state, source, strlen(source), "=(gnoblin console)", "t");
    if (status != LUA_OK) {
        lua_pop(state, 1);
        g_autofree char* expression = g_strdup_printf("return %s", source);
        status = luaL_loadbufferx(state, expression, strlen(expression), "=(gnoblin console)", "t");
    }
    if (status == LUA_OK)
        status = lua_pcall(state, 0, LUA_MULTRET, 0);
    if (status != LUA_OK) {
        const char* message = lua_error_text(state);
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s", message);
        lua_settop(state, base);
        config->api_calling = FALSE;
        config->actions_in_dispatch = 0;
        console_reject_actions(runtime, action_start, "console evaluation failed");
        return NULL;
    }

    g_autoptr(GError) document_error = NULL;
    lua_getglobal(state, "gnoblin");
    lua_getfield(state, -1, "config");
    g_autoptr(GVariant) document = variant_from_lua(state, -1, 0, FALSE, 0, &document_error);
    lua_pop(state, 2);
    if (!document || !g_variant_is_of_type(document, G_VARIANT_TYPE_VARDICT) ||
        !gnoblin_config_validate_document(document, &document_error) ||
        !g_variant_equal(document, runtime->document)) {
        const char* message =
            document_error ? document_error->message : "console code may not modify gnoblin.config";
        lua_getglobal(state, "gnoblin");
        push_variant(state, runtime->document);
        lua_setfield(state, -2, "config");
        lua_pop(state, 1);
        console_reject_actions(runtime, action_start, message);
        lua_settop(state, base);
        config->api_calling = FALSE;
        config->actions_in_dispatch = 0;
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "%s", message);
        return NULL;
    }

    int values = lua_gettop(state) - base;
    g_autoptr(GString) rendered = g_string_new(NULL);
    for (int i = 0; i < values && i < 8; i++) {
        if (i)
            g_string_append(rendered, "\n");
        console_append_value(rendered, state, base + i + 1);
    }
    if (values > 8)
        g_string_append(rendered, "\n…");
    const char* type = values == 1   ? lua_typename(state, lua_type(state, base + 1))
                       : values == 0 ? "nil"
                                     : "values";
    guint operations = config->runtime_actions->len - action_start;
    g_autoptr(GVariant) inspection = console_inspection(state, base + 1, values);
    lua_settop(state, base);
    config->api_calling = FALSE;
    config->actions_in_dispatch = 0;

    GVariantBuilder result;
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "value", g_variant_new_string(rendered->str));
    g_variant_builder_add(&result, "{sv}", "type", g_variant_new_string(type));
    g_variant_builder_add(&result, "{sv}", "inspection", inspection);
    g_variant_builder_add(&result, "{sv}", "operations", g_variant_new_int64(operations));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

GVariant* gnoblin_config_call_api(const char* method, GVariant* arguments, GError** error) {
    if (pending_runtime) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "Gnoblin Lua API is temporarily unavailable while config reloads");
        return NULL;
    }
    if (!active_runtime || !method || !arguments ||
        !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(
            error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
            "Gnoblin API call requires an active Lua runtime, method, and argument table");
        return NULL;
    }
    gboolean known = FALSE;
    for (guint i = 0; api_methods[i]; i++)
        known |= g_str_equal(api_methods[i], method);
    for (guint i = 0; i < api_aliases_count; i++)
        known |= g_str_equal(api_aliases[i].public_name, method);
    if (!known) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "unknown Gnoblin API method '%s'",
                    method);
        return NULL;
    }
    LuaConfig* config = &active_runtime->config;
    if (config->dispatching || config->api_calling) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                            "Gnoblin Lua API calls cannot be dispatched reentrantly");
        return NULL;
    }
    if (config->runtime_actions->len >= 256) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_NOSPC,
                            "too many pending Gnoblin session actions");
        return NULL;
    }
    /* Wire methods use operation names; Lua exposes them under public collection names such
     * as gnoblin.animations.preview. Look the function up by its public name. */
    const char* lookup = method;
    for (guint i = 0; i < api_aliases_count; i++)
        if (g_str_equal(api_aliases[i].operation, method)) {
            lookup = api_aliases[i].public_name;
            break;
        }
    const char* separator = strchr(lookup, '.');
    g_autofree char* domain = NULL;
    const char* operation = NULL;
    if (g_str_equal(method, "shortcut.capture")) {
        domain = g_strdup("shortcuts");
        operation = "capture";
    } else if (g_str_equal(method, "shortcut.bind") || g_str_equal(method, "shortcut.unbind")) {
        domain = g_strdup("shortcuts");
        operation = g_str_equal(method, "shortcut.bind") ? "bind" : "unbind";
    } else if (g_str_equal(method, "shortcut.session.end")) {
        domain = g_strdup("shortcuts");
        operation = "end_session";
    } else {
        domain = g_strndup(lookup, separator - lookup);
        operation = separator + 1;
    }
    lua_State* state = active_runtime->state;
    guint action_start = config->runtime_actions->len;
    config->actions_in_dispatch = 0;
    config->api_calling = TRUE;
    gboolean native_operation =
        g_str_has_prefix(method, "workspace.") || g_str_has_prefix(method, "window.") ||
        g_str_equal(method, "input.select") || g_str_equal(method, "command.run") ||
        g_str_equal(method, "command.capture");
    int stack_base = lua_gettop(state);
    if (native_operation) {
        lua_pushlightuserdata(state, config);
        lua_pushstring(state, method);
        lua_pushcclosure(state,
                         g_str_has_prefix(method, "workspace.") ? lua_workspace_action
                                                                : lua_generic_api_action,
                         2);
    } else {
        /* An unregistered domain is a nil, and indexing it outside a protected call would abort
         * the whole Lua worker. Walk the path only through tables. */
        lua_getglobal(state, "gnoblin");
        if (lua_istable(state, -1))
            lua_getfield(state, -1, domain);
        if (lua_istable(state, -1))
            lua_getfield(state, -1, operation);
        else
            lua_pushnil(state);
    }
    if (!lua_isfunction(state, -1)) {
        lua_settop(state, stack_base);
        config->api_calling = FALSE;
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                    "Lua API method '%s' is not registered", method);
        return NULL;
    }
    int argument_count = g_variant_n_children(arguments) ? 1 : 0;
    if (argument_count)
        push_variant(state, arguments);
    if (lua_pcall(state, argument_count, 1, 0) != LUA_OK) {
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "Lua API method '%s' failed: %s",
                    method, lua_error_text(state));
        lua_settop(state, stack_base);
        config->api_calling = FALSE;
        g_ptr_array_set_size(config->runtime_actions, action_start);
        config->actions_in_dispatch = 0;
        return NULL;
    }
    gboolean valid_ticket = lua_isinteger(state, -1) && lua_tointeger(state, -1) > 0 &&
                            config->runtime_actions->len == action_start + 1;
    lua_settop(state, stack_base);
    config->api_calling = FALSE;
    config->actions_in_dispatch = 0;
    if (!valid_ticket) {
        g_ptr_array_set_size(config->runtime_actions, action_start);
        g_set_error(error, G_FILE_ERROR, G_FILE_ERROR_FAILED,
                    "Lua API method '%s' did not produce exactly one operation", method);
        return NULL;
    }
    GVariant* operation_call =
        g_variant_ref(g_ptr_array_index(config->runtime_actions, action_start));
    return operation_call;
}
