/* Validates the gnoblin.conf parser (src/config/gnoblin-config.c) — the core
 * sectioned-INI reader the compositor and every protocol overlay use: quoted
 * values, inline comments, ints, on/off bools, repeated-key lists, fallbacks. */
#include "gnoblin-config.h"

#include <glib.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

static int fails;
#define CHECK(cond, msg)                                                                           \
  do                                                                                               \
    {                                                                                              \
      if (!(cond))                                                                                 \
        {                                                                                          \
          printf ("FAIL: %s\n", msg);                                                              \
          fails++;                                                                                 \
        }                                                                                          \
    }                                                                                              \
  while (0)

int
main (void)
{
  const char *conf = "  # leading comment whitespace\n"
                     "root-key = root-value\n"
                     "[missing-close\n"
                     "[ appearance ] trailing text ignored\n"
                     "  background = \"#1d1f21\"   # trailing comment\n"
                     "\taccent = 'purple # kept' # dropped\n"
                     "rounding = 14\n"
                     "empty-key-ignored = before\n"
                     " = ignored\n"
                     "empty-key-ignored = after\n"
                     "too-large = 999999999999999999999999\n"
                     "too-small = -999999999999999999999999\n"
                     "with-suffix = 12px\n"
                     "trimmed = value \t\r\n"
                     "semicolon-inline = alpha ; beta\n"
                     "; semicolon-comment = ignored\n"
                     "[startup]\n"
                     "exec = alpha\n"
                     "exec = beta\n"
                     "[bind]\n"
                     "Super+Hash = spawn printf 'value # kept' > /tmp/marker # trailing\n"
                     "[empty]\n"
                     "[protocols]\n"
                     "wlr-gamma-control = off\n"
                     "ext-data-control = on\n";
  char path[] = "/tmp/gnoblin-conf-test.XXXXXX";
  int fd = mkstemp (path);

  if (fd < 0 || write (fd, conf, strlen (conf)) < 0)
    {
      printf ("FAIL: could not write temp config\n");
      return 1;
    }
  close (fd);

  g_setenv ("GNOBLIN_CONFIG", path, TRUE);
  gnoblin_config_reload ();

  {
    g_autofree char *bg = gnoblin_config_get_string ("appearance", "background");
    CHECK (g_strcmp0 (bg, "#1d1f21") == 0, "quoted value + inline comment stripped");
  }

  {
    g_autofree char *accent = gnoblin_config_get_string ("appearance", "accent");
    CHECK (g_strcmp0 (accent, "purple # kept") == 0,
           "single-quoted value + trailing text stripped");
  }

  {
    g_autofree char *root = gnoblin_config_get_string (NULL, "root-key");
    CHECK (g_strcmp0 (root, "root-value") == 0, "top-level key before first section");
  }

  CHECK (gnoblin_config_get_string (NULL, "missing-close") == NULL,
         "section line with missing close is dropped, not parsed as a key");
  CHECK (g_strcmp0 (gnoblin_config_get_string ("appearance", "empty-key-ignored"), "after") == 0,
         "line with empty key is ignored without dropping later keys");

  {
    g_autofree char *trimmed = gnoblin_config_get_string ("appearance", "trimmed");
    CHECK (g_strcmp0 (trimmed, "value") == 0, "trailing space/tab/CR is stripped");
  }

  {
    g_autofree char *semicolon = gnoblin_config_get_string ("appearance", "semicolon-inline");
    CHECK (g_strcmp0 (semicolon, "alpha ; beta") == 0, "semicolon is data inline");
  }

  CHECK (gnoblin_config_get_string ("appearance", "semicolon-comment") == NULL,
         "semicolon starts a whole-line comment after trim");

  CHECK (gnoblin_config_get_int ("appearance", "rounding", 0) == 14, "int parse");
  CHECK (gnoblin_config_get_int ("appearance", "missing", 7) == 7, "int fallback");
  CHECK (gnoblin_config_get_int ("appearance", "too-large", 7) == 7,
         "int overflow falls back");
  CHECK (gnoblin_config_get_int ("appearance", "too-small", 7) == 7,
         "int underflow falls back");
  CHECK (gnoblin_config_get_int ("appearance", "with-suffix", 7) == 7,
         "malformed int falls back");

  CHECK (gnoblin_config_get_bool ("protocols", "ext-data-control", FALSE) == TRUE, "bool on");
  CHECK (gnoblin_config_get_bool ("protocols", "wlr-gamma-control", TRUE) == FALSE, "bool off");
  CHECK (gnoblin_config_get_bool ("protocols", "missing", TRUE) == TRUE, "bool fallback");

  {
    g_auto (GStrv) execs = gnoblin_config_get_list ("startup", "exec");
    CHECK (execs && g_strv_length (execs) == 2, "repeated-key list length");
    CHECK (execs && g_strcmp0 (execs[0], "alpha") == 0 && g_strcmp0 (execs[1], "beta") == 0,
           "repeated-key list values in order");
  }

  {
    g_auto (GStrv) keys = gnoblin_config_get_keys ("startup");
    CHECK (keys && g_strv_length (keys) == 2, "get_keys includes repeated keys, not just unique ones");
    CHECK (keys && g_strcmp0 (keys[0], "exec") == 0 && g_strcmp0 (keys[1], "exec") == 0,
           "get_keys preserves file order");
  }

  CHECK (gnoblin_config_get_keys ("missing-section") == NULL, "get_keys on absent section is NULL");
  CHECK (gnoblin_config_get_keys ("empty") == NULL, "get_keys on empty section is NULL");

  {
    g_autofree char *bind = gnoblin_config_get_string ("bind", "Super+Hash");
    CHECK (g_strcmp0 (bind, "spawn printf 'value # kept' > /tmp/marker") == 0,
           "hash inside quoted span is kept, trailing comment stripped");
  }

  unlink (path);

  if (fails == 0)
    {
      printf ("PASS: gnoblin-config parser\n");
      return 0;
    }
  return 1;
}
