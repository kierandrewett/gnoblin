/*
 * gnoblin-shell: shared cursor tokenizer — see gnoblin-spec-util.h.
 */

#include "gnoblin-spec-util.h"

#include <errno.h>

void gnoblin_spec_skip_spaces(char** p) {
    while (**p && g_ascii_isspace(**p))
        (*p)++;
}

gboolean gnoblin_spec_at_end(char* p) {
    gnoblin_spec_skip_spaces(&p);
    return *p == '\0';
}

gboolean gnoblin_spec_parse_int(char** p, int* out) {
    char* end = NULL;
    gint64 value;

    gnoblin_spec_skip_spaces(p);
    errno = 0;
    value = g_ascii_strtoll(*p, &end, 10);
    if (errno != 0 || end == *p || value < G_MININT || value > G_MAXINT)
        return FALSE;

    *out = (int)value;
    *p = end;
    return TRUE;
}

gboolean gnoblin_spec_parse_whole_int(const char* text, int* out) {
    g_autofree char* copy = NULL;
    char* p;

    if (!text || !out)
        return FALSE;

    copy = g_strdup(text);
    p = copy;
    return gnoblin_spec_parse_int(&p, out) && gnoblin_spec_at_end(p);
}
