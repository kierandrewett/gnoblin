/* The one chord grammar for every shortcut mechanism.
 *
 * A chord is zero or more modifiers in angle brackets, then one key: `<Super><Alt>F9`,
 * `<Control>q`, `XF86AudioMute`. It is the accelerator syntax of GTK and Mutter, read strictly:
 * an unknown modifier or a second key is an error (Mutter skips an unknown `<Foo>` and binds the
 * bare key). The configuration validators, the Lua runtime (`gnoblin.shortcuts.bind`,
 * `gnoblin.input.on`, `gnoblin.configure.keybindings` and `shortcuts`) and the compositor all
 * call this one parser, so a chord is accepted or refused the same way everywhere.
 *
 * This header is shared like gnoblin-easing.h: the Lua runtime includes it from src/control and
 * the control manifest copies it into Mutter as core/gnoblin-chord.h. It needs xkbcommon only. */
#pragma once

#include <glib.h>
#include <string.h>
#include <xkbcommon/xkbcommon.h>

#define GNOBLIN_CHORD_MAX_LENGTH 128

/* The example every message ends with. */
#define GNOBLIN_CHORD_EXAMPLE "<Super><Alt>F9"

typedef enum {
    GNOBLIN_CHORD_KEY,     /* a keyboard chord: modifiers and a key */
    GNOBLIN_CHORD_POINTER, /* a pointer chord: modifiers and Button1, Button2 or Button3 */
} GnoblinChordKind;

enum {
    GNOBLIN_CHORD_SHIFT = 1 << 0,
    GNOBLIN_CHORD_CONTROL = 1 << 1,
    GNOBLIN_CHORD_ALT = 1 << 2,
    GNOBLIN_CHORD_META = 1 << 3,
    GNOBLIN_CHORD_SUPER = 1 << 4,
    GNOBLIN_CHORD_HYPER = 1 << 5,
};

typedef struct {
    guint modifiers;     /* GNOBLIN_CHORD_* */
    xkb_keysym_t keysym; /* 0 for a keycode, Above_Tab or a button */
    char key[64];        /* the key name as written, for a keysym */
    guint keycode;       /* the hardware key code of `0x3F`, else 0 */
    guint button;        /* 1 to 3 for a pointer chord, else 0 */
    gboolean above_tab;  /* the key above Tab, whatever the layout calls it */
    gboolean bare_super; /* the single word `Super`: Super pressed and released alone */
} GnoblinChord;

static const struct {
    const char* name; /* lower case */
    guint modifier;   /* 0: refused, with a hint */
    const char* hint;
} gnoblin_chord_modifiers[] = {
    {"shift", GNOBLIN_CHORD_SHIFT, NULL},
    {"shft", GNOBLIN_CHORD_SHIFT, NULL},
    {"control", GNOBLIN_CHORD_CONTROL, NULL},
    {"ctrl", GNOBLIN_CHORD_CONTROL, NULL},
    {"ctl", GNOBLIN_CHORD_CONTROL, NULL},
    {"primary", GNOBLIN_CHORD_CONTROL, NULL},
    {"alt", GNOBLIN_CHORD_ALT, NULL},
    {"mod1", GNOBLIN_CHORD_ALT, NULL},
    {"meta", GNOBLIN_CHORD_META, NULL},
    {"super", GNOBLIN_CHORD_SUPER, NULL},
    {"hyper", GNOBLIN_CHORD_HYPER, NULL},
    {"mod4", 0, "<Mod4> is an X11 mask name whose key depends on the keymap; write <Super>"},
    {"mod2", 0,
     "<Mod2> is an X11 mask name whose key depends on the keymap; there is no "
     "portable form"},
    {"mod3", 0,
     "<Mod3> is an X11 mask name whose key depends on the keymap; there is no "
     "portable form"},
    {"mod5", 0,
     "<Mod5> is an X11 mask name whose key depends on the keymap; there is no "
     "portable form"},
};

/* Words people write for a modifier outside angle brackets, to build the hint. */
static const struct {
    const char* word; /* lower case */
    guint modifier;
} gnoblin_chord_words[] = {
    {"shift", GNOBLIN_CHORD_SHIFT},     {"control", GNOBLIN_CHORD_CONTROL},
    {"ctrl", GNOBLIN_CHORD_CONTROL},    {"ctl", GNOBLIN_CHORD_CONTROL},
    {"primary", GNOBLIN_CHORD_CONTROL}, {"alt", GNOBLIN_CHORD_ALT},
    {"option", GNOBLIN_CHORD_ALT},      {"meta", GNOBLIN_CHORD_META},
    {"super", GNOBLIN_CHORD_SUPER},     {"win", GNOBLIN_CHORD_SUPER},
    {"windows", GNOBLIN_CHORD_SUPER},   {"logo", GNOBLIN_CHORD_SUPER},
    {"cmd", GNOBLIN_CHORD_SUPER},       {"command", GNOBLIN_CHORD_SUPER},
    {"hyper", GNOBLIN_CHORD_HYPER},
};

static inline gboolean gnoblin_chord_key_symbol(const char* name, xkb_keysym_t* keysym) {
    /* Mutter's own lookup, so a chord means the same key to both. */
    xkb_keysym_t found = xkb_keysym_from_name(name, XKB_KEYSYM_CASE_INSENSITIVE);
    if (found == XKB_KEY_NoSymbol) {
        g_autofree char* prefixed = g_strconcat("XF86", name, NULL);
        found = xkb_keysym_from_name(prefixed, XKB_KEYSYM_CASE_INSENSITIVE);
    }
    if (found == XKB_KEY_NoSymbol)
        return FALSE;
    if (keysym)
        *keysym = found;
    return TRUE;
}

/* The chord in its one written form: modifiers as <Shift><Control><Alt><Meta><Super><Hyper> in
 * that order, then the key. The caller frees it. */
static inline char* gnoblin_chord_format(const GnoblinChord* chord) {
    if (chord->bare_super)
        return g_strdup("Super");
    GString* text = g_string_new(NULL);
    static const struct {
        guint modifier;
        const char* name;
    } order[] = {
        {GNOBLIN_CHORD_SHIFT, "<Shift>"}, {GNOBLIN_CHORD_CONTROL, "<Control>"},
        {GNOBLIN_CHORD_ALT, "<Alt>"},     {GNOBLIN_CHORD_META, "<Meta>"},
        {GNOBLIN_CHORD_SUPER, "<Super>"}, {GNOBLIN_CHORD_HYPER, "<Hyper>"},
    };
    for (guint i = 0; i < G_N_ELEMENTS(order); i++)
        if (chord->modifiers & order[i].modifier)
            g_string_append(text, order[i].name);
    if (chord->button) {
        g_string_append_printf(text, "Button%u", chord->button);
    } else if (chord->above_tab) {
        g_string_append(text, "Above_Tab");
    } else if (chord->keycode) {
        g_string_append_printf(text, "0x%02x", chord->keycode);
    } else {
        g_string_append(text, chord->key);
    }
    return g_string_free(text, FALSE);
}

/* A chord written with plus signs, dashes or spaces ("Super+Alt+F9", "ctrl-q"): the chord it
 * means in the one form, or NULL. */
static inline char* gnoblin_chord_suggest(const char* text, GnoblinChordKind kind) {
    g_auto(GStrv) parts = NULL;
    if (strchr(text, '+'))
        parts = g_strsplit(text, "+", -1);
    else if (strchr(text, ' '))
        parts = g_strsplit(text, " ", -1);
    else if (strchr(text, '-') && strlen(text) > 1)
        parts = g_strsplit(text, "-", -1);
    guint count = parts ? g_strv_length(parts) : 0;
    if (count < 2)
        return NULL;
    GnoblinChord chord = {0};
    for (guint i = 0; i + 1 < count; i++) {
        g_autofree char* word = g_ascii_strdown(g_strstrip(parts[i]), -1);
        gboolean known = FALSE;
        for (guint w = 0; w < G_N_ELEMENTS(gnoblin_chord_words); w++) {
            if (g_str_equal(word, gnoblin_chord_words[w].word)) {
                chord.modifiers |= gnoblin_chord_words[w].modifier;
                known = TRUE;
            }
        }
        if (!known)
            return NULL;
    }
    const char* key = g_strstrip(parts[count - 1]);
    if (kind == GNOBLIN_CHORD_POINTER) {
        if (!g_str_has_prefix(key, "Button") || strlen(key) != 7 || key[6] < '1' || key[6] > '3')
            return NULL;
        chord.button = (guint)(key[6] - '0');
    } else if (strlen(key) >= sizeof chord.key || !gnoblin_chord_key_symbol(key, &chord.keysym)) {
        return NULL;
    } else {
        g_strlcpy(chord.key, key, sizeof chord.key);
    }
    return gnoblin_chord_format(&chord);
}

/* Parse `text`. On success fill `chord` (when given). On failure return FALSE and set `error`
 * (when given) to a message that names the form to write, which the caller frees. `kind` is
 * the keyboard form unless a pointer chord is wanted. */
static inline gboolean gnoblin_chord_parse(const char* text, GnoblinChordKind kind,
                                           GnoblinChord* chord, char** error) {
    GnoblinChord parsed = {0};
    g_autofree char* message = NULL;
    const char* cursor = text;
    if (!text || !*text) {
        message = g_strdup("it is empty");
        goto fail;
    }
    if (strlen(text) > GNOBLIN_CHORD_MAX_LENGTH || !g_utf8_validate(text, -1, NULL)) {
        message =
            g_strdup_printf("it is not UTF-8 text of at most %d bytes", GNOBLIN_CHORD_MAX_LENGTH);
        goto fail;
    }
    for (const char* c = text; *c; c++) {
        if (g_ascii_isspace(*c) || g_ascii_iscntrl(*c)) {
            message = g_strdup("it has a space or control character");
            goto fail;
        }
    }
    if (kind == GNOBLIN_CHORD_KEY && g_str_equal(text, "Super")) {
        parsed.bare_super = TRUE;
        parsed.modifiers = GNOBLIN_CHORD_SUPER;
        goto done;
    }
    while (*cursor == '<') {
        const char* end = strchr(cursor, '>');
        if (!end) {
            message = g_strdup("a modifier has no closing >");
            goto fail;
        }
        g_autofree char* raw_name = g_strndup(cursor + 1, end - cursor - 1);
        g_autofree char* name = g_ascii_strdown(raw_name, -1);
        guint modifier = 0;
        gboolean known = FALSE;
        for (guint i = 0; i < G_N_ELEMENTS(gnoblin_chord_modifiers); i++) {
            if (!g_str_equal(name, gnoblin_chord_modifiers[i].name))
                continue;
            known = TRUE;
            modifier = gnoblin_chord_modifiers[i].modifier;
            if (!modifier)
                message = g_strdup(gnoblin_chord_modifiers[i].hint);
        }
        if (!known) {
            message = g_strdup_printf(
                "<%s> is not a modifier; the modifiers are <Shift>, <Control>, <Alt>, <Super>, "
                "<Hyper> and <Meta>",
                raw_name);
            goto fail;
        }
        if (!modifier)
            goto fail;
        if (parsed.modifiers & modifier) {
            message = g_strdup_printf("<%s> appears twice", raw_name);
            goto fail;
        }
        parsed.modifiers |= modifier;
        cursor = end + 1;
    }
    if (!*cursor) {
        message = g_strdup("it has no key after the modifiers");
        goto fail;
    }
    if (strchr(cursor, '<') || strchr(cursor, '>')) {
        message = g_strdup("it has more than one key, or a modifier after the key");
        goto fail;
    }
    if (kind == GNOBLIN_CHORD_POINTER) {
        if (!g_str_has_prefix(cursor, "Button") || strlen(cursor) != 7 || cursor[6] < '1' ||
            cursor[6] > '3') {
            message = g_strdup("a pointer chord ends with Button1, Button2 or Button3");
            goto fail;
        }
        parsed.button = (guint)(cursor[6] - '0');
        goto done;
    }
    if (g_str_equal(cursor, "Above_Tab")) {
        parsed.above_tab = TRUE;
    } else if (strlen(cursor) == 4 && cursor[0] == '0' && cursor[1] == 'x' &&
               g_ascii_isxdigit(cursor[2]) && g_ascii_isxdigit(cursor[3])) {
        parsed.keycode = (guint)strtoul(cursor, NULL, 16);
    } else if (strlen(cursor) >= sizeof parsed.key ||
               !gnoblin_chord_key_symbol(cursor, &parsed.keysym)) {
        message = g_strdup_printf("\"%s\" is not a key name; use an XKB keysym name such as "
                                  "Return, F9, q or XF86AudioMute",
                                  cursor);
        goto fail;
    } else {
        g_strlcpy(parsed.key, cursor, sizeof parsed.key);
    }
done:
    if (chord)
        *chord = parsed;
    return TRUE;
fail:
    if (error) {
        g_autofree char* suggestion = gnoblin_chord_suggest(text, kind);
        if (suggestion)
            *error = g_strdup_printf("\"%s\" is not a chord: put each modifier in angle brackets "
                                     "before the key. Write %s",
                                     text, suggestion);
        else
            *error = g_strdup_printf("\"%s\" is not a chord: %s. Write modifiers in angle "
                                     "brackets before one key, for example %s",
                                     text, message ? message : "it does not parse",
                                     kind == GNOBLIN_CHORD_POINTER ? "<Super>Button1"
                                                                   : GNOBLIN_CHORD_EXAMPLE);
    }
    return FALSE;
}
