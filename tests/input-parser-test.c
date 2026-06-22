/* Validates strict parsing for [input] numeric config fields. */
#include "gnoblin-input-spec.h"

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

int main(void) {
    double speed = 9.0;

    CHECK(gnoblin_input_parse_pointer_speed("0.5", &speed) && near(speed, 0.5),
          "pointer-speed parses a normal value");
    CHECK(gnoblin_input_parse_pointer_speed(" 2 ", &speed) && near(speed, 1.0),
          "pointer-speed clamps above the desktop range");
    CHECK(gnoblin_input_parse_pointer_speed("-2", &speed) && near(speed, -1.0),
          "pointer-speed clamps below the desktop range");

    speed = 0.25;
    CHECK(!gnoblin_input_parse_pointer_speed("0.5junk", &speed) && near(speed, 0.25),
          "pointer-speed rejects trailing garbage");
    CHECK(!gnoblin_input_parse_pointer_speed("nope", &speed) && near(speed, 0.25),
          "pointer-speed rejects text instead of becoming zero");
    CHECK(!gnoblin_input_parse_pointer_speed(NULL, &speed) && near(speed, 0.25),
          "pointer-speed rejects NULL text");
    CHECK(!gnoblin_input_parse_pointer_speed("0.5", NULL), "pointer-speed rejects NULL output");

    if (fails == 0) {
        printf("PASS: input config parser\n");
        return 0;
    }
    return 1;
}
