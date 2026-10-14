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

/* Signatures for the public C functions come from the LuaCATS annotations in src/config/lua/api/,
 * turned into the tables below by scripts/gen-lua-api.py. */
typedef struct {
    const char* name;
    const char* type;
    gboolean optional;
    const char* description;
} ApiMember;

typedef struct {
    const char* name;
    const char* summary;
    const ApiMember* fields;
    gsize n_fields;
} ApiClass;

typedef struct {
    const char* path;
    const char* summary;
    const ApiMember* params;
    gsize n_params;
    const char* returns;
} ApiFunction;

#include "gnoblin-lua-signatures.inc"

#define CONSOLE_SIGNATURES_KEY "gnoblin.signatures"

/* Remember each annotated C function's signature, keyed by the function itself, so a result shows
 * it wherever it was reached from, such as a local alias of gnoblin.windows.list. Called once the
 * API is installed and before any user code can replace a function. */
void console_register_signatures(lua_State* state) {
    lua_newtable(state);
    for (const ApiFunction* function = api_functions; function->path; function++) {
        g_auto(GStrv) parts = g_strsplit(function->path, ".", -1);
        lua_getglobal(state, "gnoblin");
        for (guint part = 0; parts[part] && lua_istable(state, -1); part++) {
            lua_pushstring(state, parts[part]);
            lua_rawget(state, -2); /* raw: the gnoblin table's __index must not run here */
            lua_remove(state, -2);
        }
        if (lua_iscfunction(state, -1)) {
            lua_pushlightuserdata(state, (void*)function);
            lua_rawset(state, -3);
        } else {
            lua_pop(state, 1);
        }
    }
    lua_setfield(state, LUA_REGISTRYINDEX, CONSOLE_SIGNATURES_KEY);
}

static const ApiFunction* console_api_function(lua_State* state, int index) {
    index = lua_absindex(state, index);
    const ApiFunction* function = NULL;
    if (lua_getfield(state, LUA_REGISTRYINDEX, CONSOLE_SIGNATURES_KEY) == LUA_TTABLE) {
        lua_pushvalue(state, index);
        if (lua_rawget(state, -2) == LUA_TLIGHTUSERDATA)
            function = lua_touserdata(state, -1);
        lua_pop(state, 1);
    }
    lua_pop(state, 1);
    return function;
}

static void console_append_parameters(GString* out, const ApiFunction* function) {
    for (gsize i = 0; i < function->n_params; i++) {
        const ApiMember* param = &function->params[i];
        g_string_append_printf(out, "%s%s%s: %s", i ? ", " : "", param->name,
                               param->optional ? "?" : "", param->type);
    }
}

/* The class named by a type such as "WorkspaceArgs", "WorkspaceArgs?" or "Rule|nil"; a union of
 * several types, an array or a builtin names none. */
static const ApiClass* console_find_class(const char* type) {
    g_autofree char* name = g_strdup(type);
    g_strstrip(name);
    gsize length = strlen(name);
    if (length > 4 && g_str_has_suffix(name, "|nil"))
        name[length - 4] = '\0';
    length = strlen(name);
    if (length && name[length - 1] == '?')
        name[length - 1] = '\0';
    char* generic = strchr(name, '<');
    if (generic)
        *generic = '\0';
    for (const ApiClass* class = api_classes; class->name; class ++)
        if (g_str_equal(class->name, name))
            return class;
    return NULL;
}

/* The argument list of the function at the given index as "(a: T, b?: U): R", or NULL when it
 * is not known. Annotated C functions carry their types; Lua functions name their own
 * parameters. */
static char* console_function_arguments(lua_State* state, int index) {
    index = lua_absindex(state, index);
    g_autoptr(GString) text = g_string_new("(");
    const ApiFunction* function = console_api_function(state, index);
    if (function) {
        console_append_parameters(text, function);
        g_string_append_c(text, ')');
        if (function->returns && *function->returns)
            g_string_append_printf(text, ": %s", function->returns);
        return g_string_free(g_steal_pointer(&text), FALSE);
    }
    if (lua_iscfunction(state, index))
        return NULL;

    lua_Debug info;
    lua_pushvalue(state, index);
    lua_getinfo(state, ">u", &info); /* pops the function */
    lua_pushvalue(state, index);
    for (int parameter = 1; parameter <= info.nparams; parameter++) {
        const char* name = lua_getlocal(state, NULL, parameter);
        if (parameter > 1)
            g_string_append(text, ", ");
        g_string_append(text, name && name[0] != '(' ? name : "?");
    }
    lua_pop(state, 1);
    if (info.isvararg)
        g_string_append(text, info.nparams > 0 ? ", ..." : "...");
    g_string_append_c(text, ')');
    return g_string_free(g_steal_pointer(&text), FALSE);
}

static void console_append_member(GString* out, const ApiMember* member, const char* indent) {
    g_string_append_printf(out, "%s%s%s: %s", indent, member->name, member->optional ? "?" : "",
                           member->type);
    if (member->description && *member->description)
        g_string_append_printf(out, "  %s", member->description);
    g_string_append_c(out, '\n');
}

/* The summary, then every parameter with its description, with the keys of a table parameter
 * listed under it. NULL for functions without an annotation. */
static char* console_function_documentation(lua_State* state, int index) {
    const ApiFunction* function = console_api_function(state, index);
    if (!function)
        return NULL;
    g_autoptr(GString) text = g_string_new(function->summary);
    if (function->n_params)
        g_string_append(text, "\n\n");
    for (gsize i = 0; i < function->n_params; i++) {
        console_append_member(text, &function->params[i], "");
        const ApiClass* class = console_find_class(function->params[i].type);
        for (gsize field = 0; class && field < class->n_fields; field++)
            console_append_member(text, &class->fields[field], "    ");
    }
    if (function->returns && *function->returns)
        g_string_append_printf(text, "\nreturns %s", function->returns);
    g_strchomp(text->str);
    return g_string_free(g_steal_pointer(&text), FALSE);
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
    case LUA_TFUNCTION: {
        g_autofree char* arguments = console_function_arguments(state, index);
        if (arguments)
            g_string_append_printf(output, "<function%s>", arguments);
        else
            g_string_append(output, "<function>");
        break;
    }
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
        if (strchr(" \t\n;(){}[],=", *cursor))
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
    g_autofree char* signature =
        lua_isfunction(state, value) ? console_function_arguments(state, value) : NULL;
    g_autofree char* documentation =
        lua_isfunction(state, value) ? console_function_documentation(state, value) : NULL;
    completion->detail =
        signature ? g_steal_pointer(&signature)
                  : g_strdup_printf("Lua %s", lua_typename(state, lua_type(state, value)));
    completion->documentation = documentation
                                    ? g_steal_pointer(&documentation)
                                    : g_strdup(readonly ? "Read-only Gnoblin snapshot member"
                                                        : "Live Lua environment member");
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

/* Where the cursor sits in a function call being typed: the innermost call that is still open. */
typedef struct {
    char open;
    gsize position;
    guint commas;
} ConsoleFrame;

typedef struct {
    gboolean found;
    char* callee;      /* the dotted name before the opening parenthesis */
    guint active;      /* zero-based argument being typed */
    gboolean in_table; /* inside a table constructor within that argument */
} ConsoleCall;

static ConsoleCall console_call_context(const char* source) {
    ConsoleCall call = {FALSE, NULL, 0, FALSE};
    g_autoptr(GArray) frames = g_array_new(FALSE, FALSE, sizeof(ConsoleFrame));
    char quote = 0;
    for (gsize i = 0; source[i]; i++) {
        char c = source[i];
        if (quote) {
            if (c == '\\' && source[i + 1])
                i++;
            else if (c == quote)
                quote = 0;
            continue;
        }
        if (c == '"' || c == '\'') {
            quote = c;
        } else if (c == '-' && source[i + 1] == '-') {
            while (source[i] && source[i] != '\n')
                i++;
            if (!source[i])
                break;
        } else if (c == '(' || c == '{' || c == '[') {
            ConsoleFrame frame = {c, i, 0};
            g_array_append_val(frames, frame);
        } else if ((c == ')' || c == '}' || c == ']') && frames->len) {
            g_array_set_size(frames, frames->len - 1);
        } else if (c == ',' && frames->len) {
            g_array_index(frames, ConsoleFrame, frames->len - 1).commas++;
        }
    }
    for (gint i = (gint)frames->len - 1; i >= 0; i--) {
        ConsoleFrame* frame = &g_array_index(frames, ConsoleFrame, i);
        if (frame->open != '(')
            continue;
        gsize end = frame->position;
        gsize start = end;
        while (start > 0 && (g_ascii_isalnum(source[start - 1]) || source[start - 1] == '_' ||
                             source[start - 1] == '.'))
            start--;
        if (start == end || (start > 0 && source[start - 1] == ':'))
            return call;
        call.found = TRUE;
        call.callee = g_strndup(source + start, end - start);
        call.active = frame->commas;
        call.in_table = (guint)i + 1 < frames->len &&
                        g_array_index(frames, ConsoleFrame, frames->len - 1).open == '{';
        return call;
    }
    return call;
}

/* Push the value reached by a dotted name from the globals. Raw reads only: the lookup must not
 * run any __index code. */
static gboolean console_push_path(lua_State* state, const char* path) {
    g_auto(GStrv) parts = g_strsplit(path, ".", -1);
    lua_rawgeti(state, LUA_REGISTRYINDEX, LUA_RIDX_GLOBALS);
    for (guint i = 0; parts[i]; i++) {
        if (!console_completion_identifier(parts[i], strlen(parts[i])) ||
            !console_completion_push_field(state, -1, parts[i])) {
            lua_pop(state, 1);
            return FALSE;
        }
        lua_remove(state, -2);
    }
    return TRUE;
}

static void console_add_help_row(GVariantBuilder* rows, const ApiMember* member, gint32 indent,
                                 gboolean active) {
    GVariantBuilder row;
    g_variant_builder_init(&row, G_VARIANT_TYPE_VARDICT);
    g_autofree char* label = g_strconcat(member->name, member->optional ? "?" : "", NULL);
    g_variant_builder_add(&row, "{sv}", "label", g_variant_new_string(label));
    g_variant_builder_add(&row, "{sv}", "type", g_variant_new_string(member->type));
    g_variant_builder_add(&row, "{sv}", "description",
                          g_variant_new_string(member->description ? member->description : ""));
    g_variant_builder_add(&row, "{sv}", "indent", g_variant_new_int32(indent));
    g_variant_builder_add(&row, "{sv}", "active", g_variant_new_boolean(active));
    g_variant_builder_add_value(rows, g_variant_builder_end(&row));
}

/* Parameter help for the call being typed, or NULL when the callee is not a known function. When
 * the cursor is inside a table argument, table_class is the class describing its keys. */
static GVariant* console_signature_help(lua_State* state, const ConsoleCall* call,
                                        const ApiClass** table_class) {
    int base = lua_gettop(state);
    GVariant* help = NULL;
    if (console_push_path(state, call->callee) && lua_isfunction(state, -1)) {
        const ApiFunction* function = console_api_function(state, -1);
        g_autofree char* arguments = console_function_arguments(state, -1);
        if (arguments) {
            GVariantBuilder rows, result;
            g_variant_builder_init(&rows, G_VARIANT_TYPE("aa{sv}"));
            for (gsize i = 0; function && i < function->n_params; i++) {
                gboolean active = i == call->active;
                console_add_help_row(&rows, &function->params[i], 0, active);
                const ApiClass* class = console_find_class(function->params[i].type);
                if (active && call->in_table && class)
                    *table_class = class;
                if (active && class)
                    for (gsize field = 0; field < class->n_fields; field++)
                        console_add_help_row(&rows, &class->fields[field], 1, FALSE);
            }
            g_autofree char* title = g_strconcat(call->callee, arguments, NULL);
            g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(&result, "{sv}", "title", g_variant_new_string(title));
            g_variant_builder_add(&result, "{sv}", "summary",
                                  g_variant_new_string(function ? function->summary : ""));
            g_variant_builder_add(&result, "{sv}", "rows", g_variant_builder_end(&rows));
            help = g_variant_ref_sink(g_variant_builder_end(&result));
        }
    }
    lua_settop(state, base);
    return help;
}

/* True when the text ends in a table key being typed: "{na" or "{a = 1, na". */
static gboolean console_typing_key(const char* source, const char** key) {
    gsize end = strlen(source);
    gsize start = end;
    while (start > 0 && (g_ascii_isalnum(source[start - 1]) || source[start - 1] == '_'))
        start--;
    gsize before = start;
    while (before > 0 && g_ascii_isspace(source[before - 1]))
        before--;
    *key = source + start;
    return before > 0 && (source[before - 1] == '{' || source[before - 1] == ',');
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
    gboolean parsed = console_completion_parse(source, segments, &prefix, &method);
    ConsoleCall call = console_call_context(source);
    g_autofree char* callee = call.callee;

    lua_State* state = runtime->state;
    int base = lua_gettop(state);
    guint actions = config->runtime_actions ? config->runtime_actions->len : 0;
    guint operations = config->operations ? g_hash_table_size(config->operations) : 0;
    config->api_calling = TRUE;
    lua_rawgeti(state, LUA_REGISTRYINDEX, LUA_RIDX_GLOBALS);
    const char* typed_key = NULL;
    gboolean key_position = call.found && call.in_table && console_typing_key(source, &typed_key);
    /* With no name started, as after a closing or opening parenthesis, a comma or a space, there is
     * nothing to complete: the signature help is the hint, so list nothing rather than every
     * global. A table key position lists the table's keys instead of globals. */
    gboolean resolved = parsed && !key_position && lua_istable(state, -1) &&
                        *console_completion_token_start(source);
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
    /* Help for the call being typed, and the keys of a table argument as completions. */
    const ApiClass* table_class = NULL;
    g_autoptr(GVariant) help =
        call.found ? console_signature_help(state, &call, &table_class) : NULL;
    for (gsize i = 0; key_position && table_class && i < table_class->n_fields; i++) {
        const ApiMember* field = &table_class->fields[i];
        if (!g_str_has_prefix(field->name, typed_key) ||
            completions->len >= MAX_CONSOLE_COMPLETIONS)
            continue;
        ConsoleCompletion* completion = g_new0(ConsoleCompletion, 1);
        completion->label = g_strdup(field->name);
        completion->detail = g_strdup(field->type);
        completion->documentation = g_strdup(field->description ? field->description : "");
        g_ptr_array_add(completions, completion);
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
    if (help)
        g_variant_builder_add(&result, "{sv}", "signature", help);
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

    /* Read the input as an expression first, like the Lua REPL, so a call such as
     * gnoblin.session.status() shows what it returns. A call is also a valid statement, so trying
     * the statement form first would run it and discard the result. Input that is not an
     * expression, such as an assignment or several statements, loads as a normal chunk instead.
     * Loading only compiles, so the failed attempt has no side effects. */
    g_autofree char* expression = g_strdup_printf("return %s", source);
    int status = luaL_loadbufferx(state, expression, strlen(expression), "=(gnoblin console)", "t");
    if (status != LUA_OK) {
        lua_pop(state, 1);
        status = luaL_loadbufferx(state, source, strlen(source), "=(gnoblin console)", "t");
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
    /* Extension namespaces such as gnoblin.spatial register their own Lua functions; the lookup
     * below reports one that is missing. */
    known |= g_str_has_prefix(method, "spatial.");
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
