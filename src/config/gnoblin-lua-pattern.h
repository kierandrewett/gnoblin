/*
 * Bounded Lua 5.4 string.find pattern matching shared by Gnoblin and Mutter.
 *
 * Copyright (C) 2026 Kieran Drewett
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <glib.h>

G_BEGIN_DECLS

typedef enum {
    GNOBLIN_LUA_PATTERN_ERROR_INVALID,
    GNOBLIN_LUA_PATTERN_ERROR_LIMIT,
} GnoblinLuaPatternError;

#define GNOBLIN_LUA_PATTERN_ERROR (gnoblin_lua_pattern_error_quark())
GQuark gnoblin_lua_pattern_error_quark(void);

/*
 * Match with Lua 5.4 string.find pattern semantics (byte-oriented; C strings
 * therefore cannot contain embedded NUL bytes). Returns TRUE when evaluation
 * completed; *matched distinguishes a match from an ordinary non-match.
 * Invalid patterns and bounded-work/input failures return FALSE with error.
 */
gboolean gnoblin_lua_pattern_match(const char* pattern, const char* value, gboolean* matched,
                                   GError** error);

G_END_DECLS
