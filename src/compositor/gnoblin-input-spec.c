/*
 * gnoblin-shell: strict parsing for [input] config fields.
 */

#include "gnoblin-input-spec.h"

#include <errno.h>
#include <math.h>

gboolean gnoblin_input_parse_pointer_speed(const char* text, double* out) {
    g_autofree char* copy = NULL;
    char* s;
    char* end = NULL;
    double value;

    if (!text || !out)
        return FALSE;

    copy = g_strdup(text);
    s = g_strstrip(copy);
    if (*s == '\0')
        return FALSE;

    errno = 0;
    value = g_ascii_strtod(s, &end);
    if (errno != 0 || end == s || !isfinite(value) || *g_strstrip(end) != '\0')
        return FALSE;

    *out = CLAMP(value, -1.0, 1.0);
    return TRUE;
}
