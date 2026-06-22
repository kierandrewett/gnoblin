/* Validates strict parsing for [output] monitor layout specs. */
#include "gnoblin-output-spec.h"

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

static int near(double a, double b) {
    return fabs(a - b) < 0.000001;
}

static GnoblinOutputSpec parse(const char* text) {
    GnoblinOutputSpec spec;
    CHECK(gnoblin_output_parse_spec(text, &spec), "parse call succeeds for non-null spec");
    return spec;
}

int main(void) {
    GnoblinOutputSpec spec =
        parse("mode 2560x1600@59.94, scale 1.5, position -1280 0, transform flipped-90, primary");

    CHECK(spec.has_mode && spec.mode_w == 2560 && spec.mode_h == 1600 &&
              near(spec.mode_refresh, 59.94),
          "mode parses WxH@refresh");
    CHECK(spec.has_scale && near(spec.scale, 1.5), "scale parses full floating point value");
    CHECK(spec.has_position && spec.pos_x == -1280 && spec.pos_y == 0,
          "position parses signed integer pair");
    CHECK(spec.has_transform && spec.transform == 5, "transform parses named flipped rotation");
    CHECK(spec.primary, "primary flag parses");

    spec = parse("mode 1920x1080, transform normal, off");
    CHECK(spec.has_mode && spec.mode_w == 1920 && spec.mode_h == 1080 &&
              near(spec.mode_refresh, 0.0),
          "mode parses WxH without refresh");
    CHECK(spec.has_transform && spec.transform == 0, "normal transform is explicit");
    CHECK(spec.disable, "off flag parses as disable");

    spec = parse("scale 2junk, scale nope, scale -1");
    CHECK(!spec.has_scale, "scale rejects trailing garbage, text, and negative values");

    spec = parse("mode 1920x1080junk, mode 0x1080, mode 1920x");
    CHECK(!spec.has_mode, "mode rejects trailing garbage, zero dimensions, and missing height");

    spec = parse("position 10 20junk, position 10x20");
    CHECK(!spec.has_position, "position rejects trailing garbage and missing separator");

    spec = parse("transform sideways");
    CHECK(!spec.has_transform, "unknown transform is ignored instead of becoming normal");

    CHECK(!gnoblin_output_parse_spec(NULL, &spec), "NULL output spec is rejected");
    CHECK(!gnoblin_output_parse_spec("scale 1", NULL), "NULL output target is rejected");

    if (fails == 0) {
        printf("PASS: output config parser\n");
        return 0;
    }
    return 1;
}
