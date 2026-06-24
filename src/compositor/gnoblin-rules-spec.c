/*
 * gnoblin-shell: strict parsing for [window-rules] action arguments.
 */

#include "gnoblin-rules-spec.h"
#include "gnoblin-spec-util.h"

#include <string.h>

gboolean gnoblin_rules_parse_size(const char* text, int* width, int* height) {
    g_autofree char* copy = NULL;
    char* p;
    int w = 0, h = 0;

    if (!text || !width || !height)
        return FALSE;

    copy = g_strdup(text);
    p = copy;
    if (!gnoblin_spec_parse_int(&p, &w))
        return FALSE;
    gnoblin_spec_skip_spaces(&p);
    if (*p != 'x' && *p != 'X')
        return FALSE;
    p++;
    if (!gnoblin_spec_parse_int(&p, &h) || !gnoblin_spec_at_end(p) || w <= 0 || h <= 0)
        return FALSE;

    *width = w;
    *height = h;
    return TRUE;
}

gboolean gnoblin_rules_parse_position(const char* text, int* x, int* y) {
    g_autofree char* copy = NULL;
    char* p;
    int px = 0, py = 0;

    if (!text || !x || !y)
        return FALSE;

    copy = g_strdup(text);
    p = copy;
    if (!gnoblin_spec_parse_int(&p, &px))
        return FALSE;
    if (*p == ',') {
        p++;
    } else if (g_ascii_isspace(*p)) {
        gnoblin_spec_skip_spaces(&p);
    } else {
        return FALSE;
    }
    if (!gnoblin_spec_parse_int(&p, &py) || !gnoblin_spec_at_end(p))
        return FALSE;

    *x = px;
    *y = py;
    return TRUE;
}

gboolean gnoblin_rules_parse_workspace_index(const char* text, int* zero_based_index) {
    int value;

    if (!gnoblin_spec_parse_whole_int(text, &value) || value <= 0 || !zero_based_index)
        return FALSE;

    *zero_based_index = value - 1;
    return TRUE;
}

gboolean gnoblin_rules_parse_monitor_index(const char* text, int* index) {
    int value;

    if (!gnoblin_spec_parse_whole_int(text, &value) || value < 0 || !index)
        return FALSE;

    *index = value;
    return TRUE;
}

gboolean gnoblin_rules_parse_percent(const char* text, int* percent) {
    int value;

    if (!gnoblin_spec_parse_whole_int(text, &value) || !percent)
        return FALSE;

    *percent = CLAMP(value, 0, 100);
    return TRUE;
}
