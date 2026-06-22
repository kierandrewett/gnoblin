/* Validates strict action argument parsing. */
#include "gnoblin-actions-spec.h"

#include <math.h>
#include <stdio.h>

static int fails;

#define CHECK(cond, msg)                                                                           \
    do {                                                                                           \
        if (!(cond)) {                                                                             \
            printf("FAIL: %s\n", msg);                                                             \
            fails++;                                                                               \
        }                                                                                          \
    } while (0)

static int neard(double a, double b) {
    return fabs(a - b) < 0.0001;
}

int main(void) {
    double x = 0, y = 0, w = 0, h = 0;
    int idx = -1;
    int percent = -1;
    guint u = 0;

    CHECK(gnoblin_actions_parse_snap_region(" 0.25 0 0.5 1 ", &x, &y, &w, &h),
          "snap region accepts four fractions");
    CHECK(neard(x, 0.25) && neard(y, 0) && neard(w, 0.5) && neard(h, 1),
          "snap region values are returned");
    CHECK(!gnoblin_actions_parse_snap_region("0 0 0.5 1junk", &x, &y, &w, &h),
          "snap region rejects trailing garbage");
    CHECK(!gnoblin_actions_parse_snap_region("0 0 0 1", &x, &y, &w, &h),
          "snap region rejects zero width");
    CHECK(!gnoblin_actions_parse_snap_region("0 0 nan 1", &x, &y, &w, &h),
          "snap region rejects non-finite values");
    CHECK(!gnoblin_actions_parse_snap_region("0 0 1", &x, &y, &w, &h),
          "snap region rejects missing values");

    CHECK(gnoblin_actions_parse_workspace_index("2", &idx) && idx == 1,
          "workspace action parses 1-based index");
    CHECK(!gnoblin_actions_parse_workspace_index("2junk", &idx),
          "workspace action rejects trailing garbage");
    CHECK(!gnoblin_actions_parse_workspace_index("0", &idx), "workspace action rejects zero");

    CHECK(gnoblin_actions_parse_monitor_index("0", &idx) && idx == 0,
          "monitor action parses 0-based index");
    CHECK(!gnoblin_actions_parse_monitor_index("-1", &idx), "monitor action rejects negative");
    CHECK(!gnoblin_actions_parse_monitor_index("1x", &idx),
          "monitor action rejects trailing garbage");

    CHECK(gnoblin_actions_parse_percent("150", &percent) && percent == 100,
          "percent clamps high values");
    CHECK(gnoblin_actions_parse_percent("-5", &percent) && percent == 0,
          "percent clamps low values");
    CHECK(!gnoblin_actions_parse_percent("50%", &percent), "percent rejects suffixes");

    CHECK(gnoblin_actions_parse_uint("2500", &u) && u == 2500, "uint parses positive values");
    CHECK(gnoblin_actions_parse_uint(" 0 ", &u) && u == 0, "uint parses zero with spaces");
    CHECK(!gnoblin_actions_parse_uint("-1", &u), "uint rejects negatives");
    CHECK(!gnoblin_actions_parse_uint("5s", &u), "uint rejects trailing garbage");

    if (fails == 0) {
        printf("PASS: action argument parser\n");
        return 0;
    }
    return 1;
}
