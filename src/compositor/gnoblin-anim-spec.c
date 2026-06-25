/*
 * gnoblin-shell: strict parsing for animation numeric config fields.
 */

#include "gnoblin-anim-spec.h"
#include "gnoblin-spec-util.h"

#include <errno.h>

gboolean gnoblin_anim_parse_duration_ms(const char* text, guint* out) {
    g_autofree char* copy = NULL;
    char* s;
    char* end = NULL;
    guint64 value;

    if (!text || !out)
        return FALSE;

    copy = g_strdup(text);
    s = g_strstrip(copy);
    if (*s == '\0' || *s == '-')
        return FALSE;

    errno = 0;
    value = g_ascii_strtoull(s, &end, 10);
    if (errno != 0 || end == s || *g_strstrip(end) != '\0' || value > G_MAXUINT)
        return FALSE;

    *out = (guint)value;
    return TRUE;
}

gboolean gnoblin_anim_parse_scale(const char* text, double* out) {
    double value;

    if (!out || !gnoblin_spec_parse_double(text, &value) || value < 0.0 || value > 2.0)
        return FALSE;

    *out = value;
    return TRUE;
}
