#include "gnoblin-config.h"
#include <glib/gstdio.h>

int main (void)
{
    g_autoptr(GError) error = NULL;
    g_autofree char *dir = g_dir_make_tmp ("gnoblin-toml-test-XXXXXX", &error);
    g_assert_no_error (error);
    g_autofree char *path = g_build_filename (dir, "gnoblin.toml", NULL);
    g_autofree char *fragment = g_build_filename (dir, "bingux.toml", NULL);
    const char *fragment_text = "[protocols]\next-data-control = false\n"
                                "[shell]\nlayer-easing = \"linear\"\n";
    g_assert_true (g_file_set_contents (fragment, fragment_text, -1, &error));
    g_assert_no_error (error);
    g_autofree char *text = g_strdup_printf ("include = [\"%s\"]\n"
                       "[protocols]\nwlr-layer-shell = false\n"
                       "[layer-shell]\npreserve-active-window = false\n"
                       "[window-management]\nconstrain-drag-to-work-area = false\n"
                       "[shell]\nminimize-duration = 125\n"
                       "minimize-animation = \"zoom\"\n"
                       "[[autostart]]\nname = 'dock'\ncommand = ['qs', '-p', '/a path']\n",
                       fragment);
    g_setenv ("GNOBLIN_CONFIG", path, TRUE);
    g_setenv ("GNOME_SHELL_SESSION_MODE", "gnoblin", TRUE);
    g_assert_true (g_file_set_contents (path, text, -1, &error));
    g_assert_no_error (error);
    gnoblin_config_reload ();
    g_assert_false (gnoblin_config_protocol_enabled ("wlr-layer-shell"));
    g_assert_false (gnoblin_config_protocol_enabled ("ext-data-control"));
    g_assert_false (gnoblin_config_get_bool ("layer-shell", "preserve-active-window", TRUE));
    g_assert_false (gnoblin_config_get_bool ("window-management", "constrain-drag-to-work-area", TRUE));
    g_assert_cmpint (gnoblin_config_get_int ("shell", "minimize-duration", 0), ==, 125);
    g_autofree char *animation = gnoblin_config_get_string ("shell", "minimize-animation");
    g_assert_cmpstr (animation, ==, "zoom");
    g_autoptr(GVariant) parsed = gnoblin_config_parse_toml (text, &error);
    g_assert_no_error (error);
    g_autoptr(GVariant) entries = g_variant_lookup_value (parsed, "autostart", G_VARIANT_TYPE("av"));
    g_assert_cmpuint (g_variant_n_children (entries), ==, 1);
    const char *invalid[] = {
        "[shell]\nx=1\nx=2", "[protocols]\nwlr-layer-shell='false'",
        "[[protocols]]\nwlr-layer-shell=true", "[shell]\nx=not-toml",
        "[layer-shell]\npreserve-active-window='false'",
        "[layer-shell]\npreserve-active-window=1",
        "[layer-shell]\nunknown=true", "layer-shell=true", NULL
    };
    for (int i = 0; invalid[i]; i++) {
        g_autoptr(GVariant) bad = gnoblin_config_parse_toml (invalid[i], &error);
        g_assert_null (bad);
        g_assert_nonnull (error);
        g_clear_error (&error);
    }
    g_assert_true (g_file_set_contents (path, "invalid TOML", -1, NULL));
    gnoblin_config_reload ();
    g_assert_false (gnoblin_config_protocol_enabled ("wlr-layer-shell"));
    g_unlink (path);
    g_unlink (fragment);
    g_rmdir (dir);
    g_print ("PASS: native TOML types, protocol gating, arrays, invalid-file retention\n");
    return 0;
}
