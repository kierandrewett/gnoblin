/* The chord grammar (src/control/gnoblin-chord.h): what it accepts, the one written form it gives
 * back, and the message that points at the form when a chord is refused. */
#include <glib.h>

#include "gnoblin-chord.h"

static void expect_chord(const char* text, GnoblinChordKind kind, const char* written) {
    GnoblinChord chord;
    g_autofree char* error = NULL;
    g_assert_true(gnoblin_chord_parse(text, kind, &chord, &error));
    g_assert_null(error);
    g_autofree char* formatted = gnoblin_chord_format(&chord);
    g_assert_cmpstr(formatted, ==, written);
}

static void expect_refusal(const char* text, GnoblinChordKind kind, const char* hint) {
    g_autofree char* error = NULL;
    g_assert_false(gnoblin_chord_parse(text, kind, NULL, &error));
    g_assert_nonnull(error);
    if (!strstr(error, hint))
        g_error("chord \"%s\": message \"%s\" lacks \"%s\"", text, error, hint);
    /* Every message names the text and ends with a form to write. */
    g_assert_nonnull(strstr(error, "Write "));
}

static void test_accepts_the_one_form(void) {
    expect_chord("<Super>Return", GNOBLIN_CHORD_KEY, "<Super>Return");
    expect_chord("<Super><Alt>F9", GNOBLIN_CHORD_KEY, "<Alt><Super>F9");
    expect_chord("<Alt>F2", GNOBLIN_CHORD_KEY, "<Alt>F2");
    expect_chord("<Control><Alt>Down", GNOBLIN_CHORD_KEY, "<Control><Alt>Down");
    expect_chord("XF86AudioMute", GNOBLIN_CHORD_KEY, "XF86AudioMute");
    expect_chord("AudioMute", GNOBLIN_CHORD_KEY, "AudioMute");
    expect_chord("<Super>Above_Tab", GNOBLIN_CHORD_KEY, "<Super>Above_Tab");
    expect_chord("<Super>0x3F", GNOBLIN_CHORD_KEY, "<Super>0x3f");
    expect_chord("Super", GNOBLIN_CHORD_KEY, "Super");
    expect_chord("<Shift><Super>Page_Up", GNOBLIN_CHORD_KEY, "<Shift><Super>Page_Up");
}

static void test_reads_mutter_spellings_as_the_written_form(void) {
    expect_chord("<Primary><Alt>Down", GNOBLIN_CHORD_KEY, "<Control><Alt>Down");
    expect_chord("<Ctrl>a", GNOBLIN_CHORD_KEY, "<Control>a");
    expect_chord("<ctl>a", GNOBLIN_CHORD_KEY, "<Control>a");
    expect_chord("<Shft>a", GNOBLIN_CHORD_KEY, "<Shift>a");
    expect_chord("<Mod1>a", GNOBLIN_CHORD_KEY, "<Alt>a");
    expect_chord("<super>Q", GNOBLIN_CHORD_KEY, "<Super>Q");
}

static void test_pointer_chords(void) {
    expect_chord("<Super>Button1", GNOBLIN_CHORD_POINTER, "<Super>Button1");
    expect_chord("<Super><Alt>Button3", GNOBLIN_CHORD_POINTER, "<Alt><Super>Button3");
    expect_refusal("<Super>Button4", GNOBLIN_CHORD_POINTER, "Button1, Button2 or Button3");
    expect_refusal("<Super>Return", GNOBLIN_CHORD_POINTER, "Button1, Button2 or Button3");
    expect_refusal("Super", GNOBLIN_CHORD_POINTER, "Button1, Button2 or Button3");
}

static void test_other_grammars_get_the_form(void) {
    GnoblinChord chord;
    g_autofree char* error = NULL;

    /* Other ways to write a chord are refused and the message gives the one form. */
    g_assert_false(gnoblin_chord_parse("Super+Alt+F9", GNOBLIN_CHORD_KEY, &chord, &error));
    g_assert_nonnull(strstr(error, "\"Super+Alt+F9\" is not a chord"));
    g_assert_nonnull(strstr(error, "Write <Alt><Super>F9"));
    g_clear_pointer(&error, g_free);

    expect_refusal("ctrl-q", GNOBLIN_CHORD_KEY, "Write <Control>q");
    expect_refusal("Ctrl+Shift+Tab", GNOBLIN_CHORD_KEY, "Write <Shift><Control>Tab");
    expect_refusal("Win+Return", GNOBLIN_CHORD_KEY, "Write <Super>Return");
    expect_refusal("super alt F9", GNOBLIN_CHORD_KEY, "Write <Alt><Super>F9");
    expect_refusal("Super+Button1", GNOBLIN_CHORD_POINTER, "Write <Super>Button1");
    /* Without a recognisable chord the message gives the example. */
    expect_refusal("Super+Nonsense", GNOBLIN_CHORD_KEY, "for example <Super><Alt>F9");
}

static void test_refuses_what_mutter_would_skip(void) {
    /* Mutter ignores an unknown <Foo> and binds the bare key. */
    expect_refusal("<Windows>q", GNOBLIN_CHORD_KEY, "<Windows> is not a modifier");
    expect_refusal("<Mod4>q", GNOBLIN_CHORD_KEY, "write <Super>");
    expect_refusal("<Mod2>q", GNOBLIN_CHORD_KEY, "no portable form");
    expect_refusal("<Super><Super>q", GNOBLIN_CHORD_KEY, "appears twice");
    expect_refusal("<Super", GNOBLIN_CHORD_KEY, "no closing >");
    expect_refusal("<Super>", GNOBLIN_CHORD_KEY, "no key after the modifiers");
    expect_refusal("<Super>q<Alt>", GNOBLIN_CHORD_KEY, "more than one key");
    expect_refusal("<Super>NoSuchKey", GNOBLIN_CHORD_KEY, "is not a key name");
    expect_refusal("", GNOBLIN_CHORD_KEY, "it is empty");
    expect_refusal("<Super> q", GNOBLIN_CHORD_KEY, "space or control character");
    expect_refusal("disabled", GNOBLIN_CHORD_KEY, "is not a key name");
}

static void test_bare_super_is_one_word(void) {
    GnoblinChord chord;
    g_assert_true(gnoblin_chord_parse("Super", GNOBLIN_CHORD_KEY, &chord, NULL));
    g_assert_true(chord.bare_super);
    g_assert_true(gnoblin_chord_parse("<Super>q", GNOBLIN_CHORD_KEY, &chord, NULL));
    g_assert_false(chord.bare_super);
    g_assert_false(gnoblin_chord_parse("super", GNOBLIN_CHORD_KEY, &chord, NULL));
}

int main(int argc, char** argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/chord/accepts-the-one-form", test_accepts_the_one_form);
    g_test_add_func("/chord/reads-mutter-spellings",
                    test_reads_mutter_spellings_as_the_written_form);
    g_test_add_func("/chord/pointer", test_pointer_chords);
    g_test_add_func("/chord/other-grammars", test_other_grammars_get_the_form);
    g_test_add_func("/chord/refuses-what-mutter-skips", test_refuses_what_mutter_would_skip);
    g_test_add_func("/chord/bare-super", test_bare_super_is_one_word);
    return g_test_run();
}
