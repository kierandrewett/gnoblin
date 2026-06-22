/* Validates strict #rrggbb/#rrggbbaa colour parsing. */
#include "gnoblin-color-spec.h"

#include <stdio.h>

static int fails;

#define CHECK(cond, msg)                                                                           \
    do {                                                                                           \
        if (!(cond)) {                                                                             \
            printf("FAIL: %s\n", msg);                                                             \
            fails++;                                                                               \
        }                                                                                          \
    } while (0)

int main(void) {
    guint8 r = 0, g = 0, b = 0, a = 0;

    CHECK(gnoblin_color_parse_hex("#102030", &r, &g, &b, &a), "rgb hex parses");
    CHECK(r == 0x10 && g == 0x20 && b == 0x30 && a == 0xff, "rgb defaults alpha to opaque");

    CHECK(gnoblin_color_parse_hex(" #10203080 ", &r, &g, &b, &a),
          "rgba hex parses with surrounding whitespace");
    CHECK(r == 0x10 && g == 0x20 && b == 0x30 && a == 0x80, "rgba returns alpha");

    CHECK(!gnoblin_color_parse_hex("#10203", &r, &g, &b, &a), "short hex is rejected");
    CHECK(!gnoblin_color_parse_hex("#1020304", &r, &g, &b, &a), "odd-length hex is rejected");
    CHECK(!gnoblin_color_parse_hex("#102030zz", &r, &g, &b, &a), "non-hex alpha is rejected");
    CHECK(!gnoblin_color_parse_hex("#102030ff00", &r, &g, &b, &a), "trailing bytes are rejected");
    CHECK(!gnoblin_color_parse_hex("102030", &r, &g, &b, &a), "missing hash is rejected");

    if (fails == 0) {
        printf("PASS: hex colour parser\n");
        return 0;
    }
    return 1;
}
