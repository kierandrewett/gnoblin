/*
 * gnoblin-shell: strict parsing for #rrggbb/#rrggbbaa colours.
 */

#include "gnoblin-color-spec.h"

#include <string.h>

static gboolean parse_hex_pair(const char* text, guint8* out) {
    int hi;
    int lo;

    if (!text || !out || !g_ascii_isxdigit(text[0]) || !g_ascii_isxdigit(text[1]))
        return FALSE;

    hi = g_ascii_xdigit_value(text[0]);
    lo = g_ascii_xdigit_value(text[1]);
    if (hi < 0 || lo < 0)
        return FALSE;

    *out = (guint8)((hi << 4) | lo);
    return TRUE;
}

gboolean gnoblin_color_parse_hex(const char* text, guint8* r, guint8* g, guint8* b, guint8* a) {
    g_autofree char* copy = NULL;
    char* s;
    size_t len;
    guint8 pr, pg, pb, pa = 255;

    if (!text || !r || !g || !b || !a)
        return FALSE;

    copy = g_strdup(text);
    s = g_strstrip(copy);
    len = strlen(s);
    if (s[0] != '#' || (len != 7 && len != 9))
        return FALSE;

    if (!parse_hex_pair(s + 1, &pr) || !parse_hex_pair(s + 3, &pg) || !parse_hex_pair(s + 5, &pb))
        return FALSE;
    if (len == 9 && !parse_hex_pair(s + 7, &pa))
        return FALSE;

    *r = pr;
    *g = pg;
    *b = pb;
    *a = pa;
    return TRUE;
}
