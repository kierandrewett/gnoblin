/* Validates strict parsing for [animations] duration and scale fields. */
#include "gnoblin-anim-spec.h"

#include <math.h>
#include <stdio.h>

static int fails;

#define CHECK(cond, msg)                                                                           \
    do {                                                                                           \
        if (!(cond)) {                                                                             \
            printf("FAIL: %s\n", msg);                                                            \
            fails++;                                                                               \
        }                                                                                          \
    } while (0)

static int near(double a, double b) {
    return fabs(a - b) < 0.000001;
}

int main(void) {
    guint duration = 999;
    double scale = 9.0;

    CHECK(gnoblin_anim_parse_duration_ms("150", &duration) && duration == 150,
          "duration parses bare milliseconds");
    CHECK(gnoblin_anim_parse_duration_ms(" 0 ", &duration) && duration == 0,
          "duration zero remains valid for disabling an effect");
    duration = 321;
    CHECK(!gnoblin_anim_parse_duration_ms("150ms", &duration) && duration == 321,
          "duration rejects unit suffixes instead of accepting partial numbers");
    CHECK(!gnoblin_anim_parse_duration_ms("nope", &duration) && duration == 321,
          "duration rejects text");
    CHECK(!gnoblin_anim_parse_duration_ms("-1", &duration) && duration == 321,
          "duration rejects negative values");

    CHECK(gnoblin_anim_parse_scale("0.985", &scale) && near(scale, 0.985),
          "scale parses subtle defaults");
    CHECK(gnoblin_anim_parse_scale(" 2.0 ", &scale) && near(scale, 2.0),
          "scale upper bound is valid");
    scale = 0.75;
    CHECK(!gnoblin_anim_parse_scale("nope", &scale) && near(scale, 0.75),
          "scale rejects text instead of becoming zero");
    CHECK(!gnoblin_anim_parse_scale("0.98junk", &scale) && near(scale, 0.75),
          "scale rejects trailing garbage");
    CHECK(!gnoblin_anim_parse_scale("nan", &scale) && near(scale, 0.75),
          "scale rejects NaN");
    CHECK(!gnoblin_anim_parse_scale("-nan", &scale) && near(scale, 0.75),
          "scale rejects negative NaN");
    CHECK(!gnoblin_anim_parse_scale("-0.1", &scale) && near(scale, 0.75),
          "scale rejects negative values");
    CHECK(!gnoblin_anim_parse_scale("2.1", &scale) && near(scale, 0.75),
          "scale rejects values above compositor bound");

    if (fails == 0) {
        printf("PASS: animation numeric parser\n");
        return 0;
    }
    return 1;
}
