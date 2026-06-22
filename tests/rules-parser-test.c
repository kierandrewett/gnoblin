/* Validates strict parsing for [window-rules] action arguments. */
#include "gnoblin-rules-spec.h"

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
    int a = 0, b = 0;

    CHECK(gnoblin_rules_parse_size("500x300", &a, &b) && a == 500 && b == 300, "size parses WxH");
    CHECK(gnoblin_rules_parse_size("500 X 300", &a, &b) && a == 500 && b == 300,
          "size accepts uppercase separator with whitespace");
    CHECK(!gnoblin_rules_parse_size("500x300junk", &a, &b), "size rejects trailing garbage");
    CHECK(!gnoblin_rules_parse_size("0x300", &a, &b), "size rejects zero dimensions");
    CHECK(!gnoblin_rules_parse_size("500x", &a, &b), "size rejects missing height");

    CHECK(gnoblin_rules_parse_position("10,20", &a, &b) && a == 10 && b == 20,
          "position parses comma form");
    CHECK(gnoblin_rules_parse_position("-10 20", &a, &b) && a == -10 && b == 20,
          "position parses signed space form");
    CHECK(gnoblin_rules_parse_position("10, 20", &a, &b) && a == 10 && b == 20,
          "position accepts whitespace after comma");
    CHECK(!gnoblin_rules_parse_position("10,20junk", &a, &b), "position rejects trailing garbage");
    CHECK(!gnoblin_rules_parse_position("10x20", &a, &b), "position rejects missing separator");

    CHECK(gnoblin_rules_parse_workspace_index("2", &a) && a == 1,
          "workspace index parses one-based config value");
    CHECK(!gnoblin_rules_parse_workspace_index("2junk", &a),
          "workspace index rejects trailing garbage");
    CHECK(!gnoblin_rules_parse_workspace_index("0", &a), "workspace index rejects zero");

    CHECK(gnoblin_rules_parse_monitor_index("0", &a) && a == 0, "monitor index accepts zero");
    CHECK(!gnoblin_rules_parse_monitor_index("-1", &a), "monitor index rejects negative values");
    CHECK(!gnoblin_rules_parse_monitor_index("1junk", &a),
          "monitor index rejects trailing garbage");

    CHECK(gnoblin_rules_parse_percent("60", &a) && a == 60, "percent parses normal value");
    CHECK(gnoblin_rules_parse_percent("120", &a) && a == 100, "percent clamps above range");
    CHECK(gnoblin_rules_parse_percent("-5", &a) && a == 0, "percent clamps below range");
    CHECK(!gnoblin_rules_parse_percent("60junk", &a), "percent rejects trailing garbage");

    if (fails == 0) {
        printf("PASS: window rules parser\n");
        return 0;
    }
    return 1;
}
