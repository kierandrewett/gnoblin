/* Loads the *actual shipped* src/data/gnoblin.conf.example and asserts the
 * tricky real-world values parse correctly — this is the file users start from,
 * and it leans on every parser edge: inline comments after quoted and unquoted
 * values, `exec` lines with trailing comments, `spawn` binds whose value embeds
 * a quoted `sh -c "...; ..."` (a '#'/';' that must NOT be mistaken for a comment),
 * and commented-out keys that must stay absent. Path comes from argv[1] so the
 * test runner can point at the in-tree copy. */
#include "gnoblin-config.h"

#include <glib.h>
#include <stdio.h>
#include <string.h>

static int fails;
#define CHECK(cond, msg)                                                       \
    do {                                                                       \
        if (!(cond)) {                                                         \
            printf("FAIL: %s\n", msg);                                         \
            fails++;                                                           \
        }                                                                      \
    } while (0)

int main(int argc, char** argv) {
    const char* path = argc > 1 ? argv[1] : "src/data/gnoblin.conf.example";

    g_setenv("GNOBLIN_CONFIG", path, TRUE);
    gnoblin_config_reload();

    /* Quoted colour with a trailing comment -> just the hex, no quotes/comment. */
    {
        g_autofree char* bg = gnoblin_config_get_string("appearance", "background");
        g_autofree char* shadow = gnoblin_config_get_string("appearance", "shadow");
        CHECK(g_strcmp0(bg, "#1d1f21") == 0, "appearance.background = bare hex");
        CHECK(g_strcmp0(shadow,
                        "0 20px 48px -20px rgba(0,0,0,.22), 0 4px 12px -6px rgba(0,0,0,.14)") == 0,
              "appearance.shadow default parses as bare CSS shadow");
    }

    /* exec / exec_per_output lists: panels/backgrounds run once-per-monitor,
     * daemons run once for the session. Every active line carries a trailing
     * comment that must be stripped. */
    {
        g_auto(GStrv) execs = gnoblin_config_get_list("startup", "exec");
        CHECK(execs && g_strv_length(execs) == 2,
              "startup.exec has 2 single-instance entries");
        if (execs && g_strv_length(execs) == 2) {
            CHECK(g_strcmp0(execs[0], "gnoblin-notifyd") == 0,
                  "exec[0] comment stripped");
            CHECK(g_strcmp0(execs[1], "gnoblin-night-light") == 0,
                  "exec[1] comment stripped");
        }
        g_auto(GStrv) per = gnoblin_config_get_list("startup", "exec_per_output");
        CHECK(per && g_strv_length(per) == 3,
              "startup.exec_per_output has 3 per-monitor entries");
        if (per && g_strv_length(per) == 3) {
            CHECK(g_strcmp0(per[0], "gnoblin-topbar") == 0,
                  "exec_per_output[0] comment stripped");
            CHECK(g_strcmp0(per[1], "gnoblin-dock") == 0,
                  "exec_per_output[1] comment stripped");
            CHECK(g_strcmp0(per[2], "gnoblin-wallpaper") == 0,
                  "exec_per_output[2] comment stripped");
        }
    }

    /* animations: a value with a trailing comment ("250, ease-out-quint   # ...").
     * `enabled` is commented out — animations follow the standard desktop setting
     * (org.gnome.desktop.interface enable-animations) by default — so it must fall
     * back to the supplied default. A genuine bool-on key is checked via
     * [features] to keep `= on` parsing covered. */
    {
        g_autofree char* ws = gnoblin_config_get_string("animations", "workspace");
        g_autofree char* dialog =
            gnoblin_config_get_string("animations", "open.modal-dialog");
        g_autofree char* menu = gnoblin_config_get_string("animations", "open.menu");
        g_autofree char* popup =
            gnoblin_config_get_string("animations", "open.popup-menu");
        g_autofree char* maximize = gnoblin_config_get_string("animations", "maximize");
        g_autofree char* unmaximize = gnoblin_config_get_string("animations", "unmaximize");
        g_autofree char* overview = gnoblin_config_get_string("animations", "overview");
        CHECK(g_strcmp0(ws, "250, ease-out-quint") == 0,
              "animations.workspace keeps value, drops comment");
        CHECK(g_strcmp0(dialog, "105, ease-out-cubic, 0.985") == 0,
              "animations.open.modal-dialog present");
        CHECK(g_strcmp0(menu, "80, ease-out-quad, 0.995") == 0,
              "animations.open.menu present");
        CHECK(g_strcmp0(popup, "80, ease-out-quad, 0.995") == 0,
              "animations.open.popup-menu present");
        CHECK(g_strcmp0(maximize, "210, ease-out-quint") == 0,
              "animations.maximize present");
        CHECK(g_strcmp0(unmaximize, "190, ease-out-quint") == 0,
              "animations.unmaximize present");
        CHECK(g_strcmp0(overview, "250, ease-out-quint") == 0,
              "animations.overview present");
        CHECK(gnoblin_config_get_bool("animations", "enabled", FALSE) == FALSE,
              "commented-out animations.enabled falls back to default");
        CHECK(gnoblin_config_get_bool("features", "appmenu", FALSE) == TRUE,
              "features.appmenu = on parses true");
    }

    /* Topbar placement arrays and menu backend are user-customisable. */
    {
        g_autofree char* left = gnoblin_config_get_string("topbar", "left");
        g_autofree char* center = gnoblin_config_get_string("topbar", "center");
        g_autofree char* right = gnoblin_config_get_string("topbar", "right");
        g_autofree char* backend = gnoblin_config_get_string("topbar", "appmenu-backend");
        CHECK(g_strcmp0(left, "workspaces, focused-app, appmenu, spring") == 0,
              "topbar.left widget placement parses");
        CHECK(g_strcmp0(center, "clock") == 0,
              "topbar.center widget placement parses");
        CHECK(g_strcmp0(right, "launcher, tray, status") == 0,
              "topbar.right widget placement parses");
        CHECK(g_strcmp0(backend, "auto") == 0,
              "topbar.appmenu-backend parses");
    }

    /* The critical case: a spawn bind whose value is `spawn sh -c "...; ..."`.
     * The ';' has no leading space and the '#'-free command sits inside quotes,
     * so the whole value — quotes included — must survive verbatim. */
    {
        g_autofree char* raise =
            gnoblin_config_get_string("bind", "XF86AudioRaiseVolume");
        CHECK(raise != NULL, "bind XF86AudioRaiseVolume present");
        if (raise) {
            CHECK(strstr(raise, "gnoblin-osd volume") != NULL,
                  "spawn command survives intact past ';'");
            CHECK(strstr(raise, "5%+; gnoblin-osd") != NULL,
                  "';' inside spawn not treated as a comment");
        }
    }

    /* Plain unquoted binds parse to the bare action. */
    {
        g_autofree char* q = gnoblin_config_get_string("bind", "Super+Q");
        CHECK(g_strcmp0(q, "close") == 0, "bind Super+Q = close");
    }

    /* Commented-out keys are absent (return NULL / fall back). */
    {
        g_autofree char* wp = gnoblin_config_get_string("appearance", "wallpaper");
        CHECK(wp == NULL, "commented-out appearance.wallpaper is absent");
    }

    /* Protocol toggles read as bools. */
    CHECK(gnoblin_config_get_bool("protocols", "ext-data-control", FALSE) == TRUE,
          "protocols.ext-data-control = on");
    CHECK(gnoblin_config_get_bool("protocols", "kde-appmenu", FALSE) == TRUE,
          "protocols.kde-appmenu = on");

    if (fails == 0) {
        printf("PASS: gnoblin.conf.example parses correctly\n");
        return 0;
    }
    return 1;
}
