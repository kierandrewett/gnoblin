/*
 * Copyright (C) 2026 Kieran Drewett
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "gnoblin-lua-pattern.h"

#include <string.h>

typedef struct {
    const char* pattern;
    const char* value;
    gboolean expected;
} MatchCase;

static void test_valid_patterns(void) {
    const MatchCase cases[] = {
        {"^gnoblin$", "gnoblin", TRUE},
        {"%d+", "window-2048", TRUE},
        {"^[%a_][%w_]*$", "gnoblin_2", TRUE},
        {"(ab)%1", "abab", TRUE},
        {"%b()", "prefix (nested (pair)) suffix", TRUE},
        {"%f[%a]gnoblin%f[%A]", "use gnoblin now", TRUE},
        {"^a-b$", "ab", TRUE},
        {"^a-b$", "aaab", TRUE},
        {"^a-b$", "acb", FALSE},
        {"^%d%d%d%d$", "2026", TRUE},
        {"^%d%d%d%d$", "26", FALSE},
        {"plain", "prefix plain suffix", TRUE},
        {"plain", "absent", FALSE},
    };

    for (gsize i = 0; i < G_N_ELEMENTS(cases); i++) {
        gboolean matched = !cases[i].expected;
        GError* error = NULL;
        gboolean ok = gnoblin_lua_pattern_match(cases[i].pattern, cases[i].value, &matched, &error);
        g_assert_no_error(error);
        g_assert_true(ok);
        g_assert_cmpint(matched, ==, cases[i].expected);
    }
}

static void test_invalid_patterns(void) {
    const char* patterns[] = {"%", "[abc", "(", ")", "%1", "%fabc", "%b("};
    for (gsize i = 0; i < G_N_ELEMENTS(patterns); i++) {
        gboolean matched = TRUE;
        GError* error = NULL;
        gboolean ok = gnoblin_lua_pattern_match(patterns[i], "value", &matched, &error);
        g_assert_false(ok);
        g_assert_false(matched);
        g_assert_error(error, GNOBLIN_LUA_PATTERN_ERROR, GNOBLIN_LUA_PATTERN_ERROR_INVALID);
        g_clear_error(&error);
    }
}

static void test_input_limits(void) {
    char* long_subject = g_malloc(16386);
    memset(long_subject, 'a', 16385);
    long_subject[16385] = '\0';
    gboolean matched = TRUE;
    GError* error = NULL;

    g_assert_false(gnoblin_lua_pattern_match(".*", long_subject, &matched, &error));
    g_assert_false(matched);
    g_assert_error(error, GNOBLIN_LUA_PATTERN_ERROR, GNOBLIN_LUA_PATTERN_ERROR_LIMIT);

    g_clear_error(&error);
    g_free(long_subject);
}

int main(int argc, char** argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/lua-pattern/valid", test_valid_patterns);
    g_test_add_func("/lua-pattern/invalid", test_invalid_patterns);
    g_test_add_func("/lua-pattern/limits", test_input_limits);
    return g_test_run();
}
