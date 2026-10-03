/*
 * Bounded Lua 5.4 string.find pattern matcher.
 *
 * The matching rules are adapted from Lua 5.4's lstrlib.c pattern matcher,
 * Copyright (C) 1994-2026 Lua.org, PUC-Rio. This implementation replaces
 * Lua's longjmp-based errors with GError and enforces explicit input/work
 * limits. It does not embed Lua or execute Lua code.
 *
 * Lua MIT license notice for the adapted matcher:
 * Permission is hereby granted, free of charge, to any person obtaining a
 * copy of this software and associated documentation files (the "Software"),
 * to deal in the Software without restriction, including without limitation
 * the rights to use, copy, modify, merge, publish, distribute, sublicense,
 * and/or sell copies of the Software, and to permit persons to whom the
 * Software is furnished to do so, subject to the following conditions: The
 * above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
 * FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
 * IN THE SOFTWARE.
 *
 * Copyright (C) 2026 Kieran Drewett
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "gnoblin-lua-pattern.h"

#include <ctype.h>
#include <stddef.h>
#include <string.h>

#define MAX_PATTERN_LENGTH 4096
#define MAX_SUBJECT_LENGTH 16384
#define MAX_PATTERN_CAPTURES 32
#define MAX_MATCH_DEPTH 128
#define MAX_MATCH_STEPS 1000000

#define CAP_UNFINISHED (-1)
#define CAP_POSITION (-2)
#define L_ESC '%'

typedef struct {
    const unsigned char* p_init;
    const unsigned char* src_init;
    const unsigned char* src_end;
    const unsigned char* p_end;
    guint64 steps;
    guint depth;
    guint level;
    gboolean failed;
    GError** error;
    struct {
        const unsigned char* init;
        ptrdiff_t len;
    } capture[MAX_PATTERN_CAPTURES];
} MatchState;

G_DEFINE_QUARK(gnoblin - lua - pattern - error - quark, gnoblin_lua_pattern_error)

static gboolean spend(MatchState* ms, guint64 amount) {
    if (ms->failed)
        return FALSE;
    if (G_UNLIKELY(amount > MAX_MATCH_STEPS - ms->steps)) {
        ms->failed = TRUE;
        g_set_error_literal(ms->error, GNOBLIN_LUA_PATTERN_ERROR, GNOBLIN_LUA_PATTERN_ERROR_LIMIT,
                            "Lua pattern matching exceeded its work limit");
        return FALSE;
    }
    ms->steps += amount;
    return TRUE;
}

static gboolean invalid(MatchState* ms, const char* message) {
    if (!ms->failed) {
        ms->failed = TRUE;
        g_set_error(ms->error, GNOBLIN_LUA_PATTERN_ERROR, GNOBLIN_LUA_PATTERN_ERROR_INVALID,
                    "invalid Lua pattern: %s", message);
    }
    return FALSE;
}

static const unsigned char* class_end(MatchState* ms, const unsigned char* p) {
    if (p >= ms->p_end)
        return p;
    if (*p++ == L_ESC) {
        if (p == ms->p_end) {
            invalid(ms, "malformed Lua pattern: trailing '%' escape");
            return ms->p_end;
        }
        return p + 1;
    }
    if (p[-1] == '[') {
        if (p < ms->p_end && *p == '^')
            p++;
        while (p < ms->p_end) {
            if (!spend(ms, 1))
                return ms->p_end;
            if (*p++ == L_ESC && p < ms->p_end) {
                p++;
            }
            if (p < ms->p_end && *p == ']')
                return p + 1;
        }
        invalid(ms, "malformed Lua pattern: missing ']' in character class");
    }
    return p;
}

static gboolean match_class(int c, int cl) {
    int result;
    switch (tolower((unsigned char)cl)) {
    case 'a':
        result = isalpha((unsigned char)c);
        break;
    case 'c':
        result = iscntrl((unsigned char)c);
        break;
    case 'd':
        result = isdigit((unsigned char)c);
        break;
    case 'g':
        result = isgraph((unsigned char)c);
        break;
    case 'l':
        result = islower((unsigned char)c);
        break;
    case 'p':
        result = ispunct((unsigned char)c);
        break;
    case 's':
        result = isspace((unsigned char)c);
        break;
    case 'u':
        result = isupper((unsigned char)c);
        break;
    case 'w':
        result = isalnum((unsigned char)c);
        break;
    case 'x':
        result = isxdigit((unsigned char)c);
        break;
    case 'z':
        result = (c == 0);
        break;
    default:
        return cl == c;
    }
    return islower((unsigned char)cl) ? result : !result;
}

static gboolean match_bracket_class(MatchState* ms, int c, const unsigned char* p,
                                    const unsigned char* ec) {
    gboolean positive = TRUE;
    if (p + 1 < ec && p[1] == '^') {
        positive = FALSE;
        p++;
    }
    while (++p < ec) {
        if (!spend(ms, 1))
            return FALSE;
        if (*p == L_ESC) {
            p++;
            if (p < ec && match_class(c, *p))
                return positive;
        } else if (p + 2 < ec && p[1] == '-') {
            int first = p[0];
            p += 2;
            if (first <= c && c <= *p)
                return positive;
        } else if (*p == c) {
            return positive;
        }
    }
    return !positive;
}

static gboolean single_match(MatchState* ms, const unsigned char* s, const unsigned char* p,
                             const unsigned char* ep) {
    if (!spend(ms, 1) || s >= ms->src_end)
        return FALSE;
    int c = *s;
    switch (*p) {
    case '.':
        return TRUE;
    case L_ESC:
        return p + 1 < ms->p_end && match_class(c, p[1]);
    case '[':
        return match_bracket_class(ms, c, p, ep - 1);
    default:
        return *p == c;
    }
}

static const unsigned char* match(MatchState*, const unsigned char*, const unsigned char*);

static const unsigned char* match_balance(MatchState* ms, const unsigned char* s,
                                          const unsigned char* p) {
    if (p + 1 >= ms->p_end) {
        invalid(ms, "malformed Lua pattern: missing arguments to '%b'");
        return NULL;
    }
    if (s >= ms->src_end || *s != *p)
        return NULL;
    unsigned char open = *p, close = p[1];
    guint nesting = 1;
    while (++s < ms->src_end) {
        if (!spend(ms, 1))
            return NULL;
        if (*s == close) {
            if (--nesting == 0)
                return s + 1;
        } else if (*s == open) {
            nesting++;
        }
    }
    return NULL;
}

static const unsigned char* max_expand(MatchState* ms, const unsigned char* s,
                                       const unsigned char* p, const unsigned char* ep) {
    ptrdiff_t count = 0;
    while (single_match(ms, s + count, p, ep))
        count++;
    while (count >= 0 && !ms->failed) {
        const unsigned char* result = match(ms, s + count, ep + 1);
        if (result)
            return result;
        count--;
    }
    return NULL;
}

static const unsigned char* min_expand(MatchState* ms, const unsigned char* s,
                                       const unsigned char* p, const unsigned char* ep) {
    for (;;) {
        const unsigned char* result = match(ms, s, ep + 1);
        if (result || ms->failed)
            return result;
        if (!single_match(ms, s, p, ep))
            return NULL;
        s++;
    }
}

static const unsigned char* start_capture(MatchState* ms, const unsigned char* s,
                                          const unsigned char* p, ptrdiff_t kind) {
    if (ms->level >= MAX_PATTERN_CAPTURES) {
        invalid(ms, "too many Lua pattern captures (maximum is 32)");
        return NULL;
    }
    guint level = ms->level++;
    ms->capture[level].init = s;
    ms->capture[level].len = kind;
    const unsigned char* result = match(ms, s, p);
    if (!result)
        ms->level--;
    return result;
}

static gboolean capture_to_close(MatchState* ms, guint* capture) {
    for (gint i = (gint)ms->level - 1; i >= 0; i--) {
        if (ms->capture[i].len == CAP_UNFINISHED) {
            *capture = (guint)i;
            return TRUE;
        }
    }
    return invalid(ms, "invalid Lua pattern capture close");
}

static const unsigned char* end_capture(MatchState* ms, const unsigned char* s,
                                        const unsigned char* p) {
    guint capture;
    if (!capture_to_close(ms, &capture))
        return NULL;
    ms->capture[capture].len = s - ms->capture[capture].init;
    const unsigned char* result = match(ms, s, p);
    if (!result)
        ms->capture[capture].len = CAP_UNFINISHED;
    return result;
}

static const unsigned char* match_capture(MatchState* ms, const unsigned char* s, int digit) {
    int index = digit - '1';
    if (index < 0 || (guint)index >= ms->level || ms->capture[index].len == CAP_UNFINISHED) {
        invalid(ms, "invalid Lua pattern capture reference");
        return NULL;
    }
    ptrdiff_t length = ms->capture[index].len;
    if (length < 0 || (size_t)(ms->src_end - s) < (size_t)length || !spend(ms, (guint64)length))
        return NULL;
    return memcmp(ms->capture[index].init, s, (size_t)length) == 0 ? s + length : NULL;
}

static const unsigned char* match(MatchState* ms, const unsigned char* s, const unsigned char* p) {
    if (!spend(ms, 1))
        return NULL;
    if (ms->depth >= MAX_MATCH_DEPTH) {
        ms->failed = TRUE;
        g_set_error_literal(ms->error, GNOBLIN_LUA_PATTERN_ERROR, GNOBLIN_LUA_PATTERN_ERROR_LIMIT,
                            "Lua pattern matching exceeded its recursion limit");
        return NULL;
    }
    ms->depth++;
    const unsigned char* result = NULL;
    while (p < ms->p_end && !ms->failed) {
        switch (*p) {
        case '(':
            if (p + 1 < ms->p_end && p[1] == ')')
                result = start_capture(ms, s, p + 2, CAP_POSITION);
            else
                result = start_capture(ms, s, p + 1, CAP_UNFINISHED);
            goto done;
        case ')':
            result = end_capture(ms, s, p + 1);
            goto done;
        case '$':
            if (p + 1 == ms->p_end) {
                result = (s == ms->src_end) ? s : NULL;
                goto done;
            }
            break;
        case L_ESC:
            if (p + 1 >= ms->p_end) {
                invalid(ms, "malformed Lua pattern: trailing '%' escape");
                goto done;
            }
            if (p[1] == 'b') {
                result = match_balance(ms, s, p + 2);
                if (result)
                    result = match(ms, result, p + 4);
                goto done;
            }
            if (p[1] == 'f') {
                const unsigned char* ep = p + 2;
                if (ep >= ms->p_end || *ep != '[') {
                    invalid(ms, "malformed Lua pattern: missing '[' after '%f'");
                    goto done;
                }
                ep = class_end(ms, ep);
                if (ms->failed)
                    goto done;
                unsigned char previous = s == ms->src_init ? 0 : s[-1];
                unsigned char current = s < ms->src_end ? *s : 0;
                if (!match_bracket_class(ms, previous, p + 2, ep - 1) &&
                    match_bracket_class(ms, current, p + 2, ep - 1))
                    p = ep;
                else
                    goto done;
                continue;
            }
            if (p[1] >= '0' && p[1] <= '9') {
                result = match_capture(ms, s, p[1]);
                if (result)
                    result = match(ms, result, p + 2);
                goto done;
            }
            break;
        default:
            break;
        }

        const unsigned char* ep = class_end(ms, p);
        if (ms->failed)
            goto done;
        gboolean first = single_match(ms, s, p, ep);
        if (ms->failed)
            goto done;
        if (!first) {
            if (ep < ms->p_end && (*ep == '*' || *ep == '?' || *ep == '-'))
                p = ep + 1;
            else
                goto done;
            continue;
        }
        if (ep == ms->p_end) {
            result = s + 1;
            goto done;
        }
        switch (*ep) {
        case '?':
            result = match(ms, s + 1, ep + 1);
            if (!result && !ms->failed) {
                p = ep + 1;
                continue;
            }
            goto done;
        case '+':
            s++;
            /* fall through */
        case '*':
            result = max_expand(ms, s, p, ep);
            goto done;
        case '-':
            result = min_expand(ms, s, p, ep);
            goto done;
        default:
            s++;
            p = ep;
            continue;
        }
    }
    if (!ms->failed && p == ms->p_end)
        result = s;
done:
    ms->depth--;
    return result;
}

static gboolean validate_pattern(MatchState* ms) {
    gboolean capture_closed[MAX_PATTERN_CAPTURES] = {FALSE};
    guint level = 0;
    const unsigned char* p = ms->p_init;

    if (p < ms->p_end && *p == '^')
        p++;
    while (p < ms->p_end) {
        if (!spend(ms, 1))
            return FALSE;
        if (*p == '(') {
            if (level == MAX_PATTERN_CAPTURES)
                return invalid(ms, "too many Lua pattern captures (maximum is 32)");
            capture_closed[level++] = FALSE;
            if (p + 1 < ms->p_end && p[1] == ')') {
                capture_closed[level - 1] = TRUE;
                p += 2;
            } else {
                p++;
            }
        } else if (*p == ')') {
            if (level == 0)
                return invalid(ms, "invalid Lua pattern capture close");
            guint capture = level - 1;
            while (capture > 0 && capture_closed[capture])
                capture--;
            if (capture_closed[capture])
                return invalid(ms, "invalid Lua pattern capture close");
            capture_closed[capture] = TRUE;
            p++;
        } else if (*p == L_ESC) {
            if (p + 1 >= ms->p_end)
                return invalid(ms, "malformed Lua pattern: trailing '%' escape");
            if (p[1] == 'b') {
                if (p + 3 >= ms->p_end)
                    return invalid(ms, "malformed Lua pattern: missing arguments to '%b'");
                p += 4;
            } else if (p[1] == 'f') {
                if (p + 2 >= ms->p_end || p[2] != '[')
                    return invalid(ms, "malformed Lua pattern: missing '[' after '%f'");
                p = class_end(ms, p + 2);
                if (ms->failed)
                    return FALSE;
            } else if (p[1] >= '0' && p[1] <= '9') {
                int index = p[1] - '1';
                if (index < 0 || (guint)index >= level || !capture_closed[index])
                    return invalid(ms, "invalid Lua pattern capture reference");
                p += 2;
            } else {
                p += 2;
            }
        } else if (*p == '[') {
            p = class_end(ms, p);
            if (ms->failed)
                return FALSE;
        } else {
            p++;
        }
    }
    if (level != 0) {
        for (guint i = 0; i < level; i++)
            if (!capture_closed[i])
                return invalid(ms, "unfinished Lua pattern capture");
    }
    return TRUE;
}

static size_t bounded_length(const char* text, size_t limit) {
    size_t length = 0;
    while (length < limit && text[length] != '\0')
        length++;
    return length;
}

gboolean gnoblin_lua_pattern_match(const char* pattern, const char* value, gboolean* matched,
                                   GError** error) {
    g_return_val_if_fail(matched != NULL, FALSE);
    *matched = FALSE;
    if (!pattern || !value) {
        g_set_error_literal(error, GNOBLIN_LUA_PATTERN_ERROR, GNOBLIN_LUA_PATTERN_ERROR_INVALID,
                            "invalid Lua pattern: pattern and subject must be strings");
        return FALSE;
    }
    size_t pattern_length = bounded_length(pattern, MAX_PATTERN_LENGTH + 1);
    size_t subject_length = bounded_length(value, MAX_SUBJECT_LENGTH + 1);
    if (pattern_length > MAX_PATTERN_LENGTH || subject_length > MAX_SUBJECT_LENGTH) {
        g_set_error(error, GNOBLIN_LUA_PATTERN_ERROR, GNOBLIN_LUA_PATTERN_ERROR_LIMIT,
                    "Lua pattern input exceeds limits (pattern %u bytes, subject %u bytes)",
                    MAX_PATTERN_LENGTH, MAX_SUBJECT_LENGTH);
        return FALSE;
    }

    MatchState ms = {
        .p_init = (const unsigned char*)pattern,
        .src_init = (const unsigned char*)value,
        .src_end = (const unsigned char*)value + subject_length,
        .p_end = (const unsigned char*)pattern + pattern_length,
        .error = error,
    };
    if (!validate_pattern(&ms))
        return FALSE;

    const unsigned char* p = (const unsigned char*)pattern;
    gboolean anchored = pattern_length > 0 && *p == '^';
    if (anchored)
        p++;
    const unsigned char* s = ms.src_init;
    do {
        ms.level = 0;
        ms.depth = 0;
        const unsigned char* result = match(&ms, s, p);
        if (result) {
            *matched = TRUE;
            return TRUE;
        }
        if (ms.failed || anchored || s == ms.src_end)
            break;
        s++;
    } while (TRUE);
    return !ms.failed;
}
