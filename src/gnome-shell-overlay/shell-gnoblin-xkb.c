/* Gnoblin's XKB lookup uses the same registry that Mutter uses for keymaps. */

#include "config.h"

#include <libintl.h>
#include <string.h>
#include <xkbcommon/xkbregistry.h>

#include "shell-util.h"

static struct rxkb_context* xkb_registry(void) {
    static struct rxkb_context* registry;

    if (!registry) {
        registry = rxkb_context_new(RXKB_CONTEXT_LOAD_EXOTIC_RULES);
        if (registry && !rxkb_context_parse_default_ruleset(registry))
            registry = rxkb_context_unref(registry);
    }
    return registry;
}

static struct rxkb_layout* find_layout(const char* id) {
    struct rxkb_context* registry = xkb_registry();
    struct rxkb_layout* item;

    if (!registry || !id)
        return NULL;

    for (item = rxkb_layout_first(registry); item; item = rxkb_layout_next(item)) {
        const char* name = rxkb_layout_get_name(item);
        const char* variant = rxkb_layout_get_variant(item);
        g_autofree char* key = variant ? g_strconcat(name, "+", variant, NULL) : g_strdup(name);

        if (g_str_equal(key, id))
            return item;
    }
    return NULL;
}

/**
 * shell_util_xkb_layout_info:
 * @id: XKB input source ID, such as us or us+intl
 * @display_name: (out) (transfer full): localized display name
 * @short_name: (out) (transfer full): short indicator label
 * @layout: (out) (transfer full): XKB layout
 * @variant: (out) (transfer full): XKB variant, or an empty string
 *
 * Returns: whether @id is in the installed XKB registry
 */
gboolean shell_util_xkb_layout_info(const char* id, char** display_name, char** short_name,
                                    char** layout, char** variant) {
    struct rxkb_layout* item = find_layout(id);
    const char* name;
    const char* brief;
    const char* description;

    *display_name = NULL;
    *short_name = NULL;
    *layout = NULL;
    *variant = NULL;
    if (!item)
        return FALSE;

    name = rxkb_layout_get_name(item);
    brief = rxkb_layout_get_brief(item);
    description = rxkb_layout_get_description(item);
    *display_name = g_strdup(description ? dgettext("xkeyboard-config", description) : id);
    *short_name = g_strdup(brief ? brief : name);
    *layout = g_strdup(name);
    *variant = g_strdup(rxkb_layout_get_variant(item) ?: "");
    return TRUE;
}

/**
 * shell_util_xkb_languages_for_layout:
 * @id: XKB input source ID
 *
 * Returns: (transfer full): ISO 639-3 language codes for @id
 */
GStrv shell_util_xkb_languages_for_layout(const char* id) {
    struct rxkb_layout* item = find_layout(id);
    GPtrArray* languages = g_ptr_array_new_with_free_func(g_free);
    struct rxkb_iso639_code* code;

    if (item)
        for (code = rxkb_layout_get_iso639_first(item); code; code = rxkb_iso639_code_next(code))
            g_ptr_array_add(languages, g_strdup(rxkb_iso639_code_get_code(code)));
    g_ptr_array_add(languages, NULL);
    return (GStrv)g_ptr_array_free(languages, FALSE);
}

/**
 * shell_util_xkb_layout_for_locale:
 * @locale: locale such as en_US.UTF-8
 *
 * Returns: (transfer full): installed base layout for the locale territory
 */
char* shell_util_xkb_layout_for_locale(const char* locale) {
    struct rxkb_context* registry = xkb_registry();
    struct rxkb_layout* item;
    const char* territory;
    char country[3];

    if (!registry || !locale || !(territory = strchr(locale, '_')) ||
        !g_ascii_isalpha(territory[1]) || !g_ascii_isalpha(territory[2]))
        return g_strdup("us");

    country[0] = g_ascii_toupper(territory[1]);
    country[1] = g_ascii_toupper(territory[2]);
    country[2] = '\0';

    /* Prefer the territory's named base layout when several layouts share it. */
    for (item = rxkb_layout_first(registry); item; item = rxkb_layout_next(item))
        if (!rxkb_layout_get_variant(item) &&
            g_ascii_strcasecmp(rxkb_layout_get_name(item), country) == 0)
            return g_strdup(rxkb_layout_get_name(item));

    for (item = rxkb_layout_first(registry); item; item = rxkb_layout_next(item)) {
        struct rxkb_iso3166_code* code;

        if (rxkb_layout_get_variant(item))
            continue;
        for (code = rxkb_layout_get_iso3166_first(item); code; code = rxkb_iso3166_code_next(code))
            if (g_str_equal(rxkb_iso3166_code_get_code(code), country))
                return g_strdup(rxkb_layout_get_name(item));
    }
    return g_strdup("us");
}
