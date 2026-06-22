/* Validates the CSS box-shadow parser backing [appearance] shadow. */
#include "gnoblin-shadow-spec.h"

#include <math.h>
#include <stdio.h>
#include <string.h>

static int fails;

#define CHECK(cond, msg)                                                                           \
    do {                                                                                           \
        if (!(cond)) {                                                                             \
            printf("FAIL: %s\n", msg);                                                            \
            fails++;                                                                               \
        }                                                                                          \
    } while (0)

static int nearf(float a, float b) {
    return fabsf(a - b) < 0.001f;
}

int main(void) {
    GnoblinShadowLayer layers[GNOBLIN_SHADOW_MAX_LAYERS];
    float color[4];
    int n;

    memset(layers, 0, sizeof layers);
    n = gnoblin_shadow_parse_box_shadow(
        "0 18px 36px 0 rgba(0,0,0,.16), 0 2px 8px 0 rgba(0,0,0,.10)", layers,
        GNOBLIN_SHADOW_MAX_LAYERS);
    CHECK(n == 2, "default soft shadow parses as two layers");
    CHECK(nearf(layers[0].offset_x, 0) && nearf(layers[0].offset_y, 18) &&
              nearf(layers[0].blur, 36) && nearf(layers[0].spread, 0),
          "default outer shadow geometry and spread");
    CHECK(nearf(layers[0].color[3], 0.16f), "default outer shadow alpha");
    CHECK(nearf(layers[1].offset_y, 2) && nearf(layers[1].blur, 8) &&
              nearf(layers[1].color[3], 0.10f),
          "default inner shadow geometry and alpha");
    CHECK(nearf(gnoblin_shadow_pad_for_layers(layers, n), 58.0f),
          "default shadow pad covers offset plus blur");

    memset(layers, 0, sizeof layers);
    n = gnoblin_shadow_parse_box_shadow(
        "1px -2px 3px #10203080, 4 5 6 rgba(7, 8, 9, 0.25)", layers,
        GNOBLIN_SHADOW_MAX_LAYERS);
    CHECK(n == 2, "hex-alpha and rgba commas parse together");
    CHECK(nearf(layers[0].offset_x, 1) && nearf(layers[0].offset_y, -2) &&
              nearf(layers[0].blur, 3) && nearf(layers[0].spread, 0),
          "px suffixes and negative offsets parse");
    CHECK(nearf(layers[0].color[0], 16.0f / 255.0f) &&
              nearf(layers[0].color[3], 128.0f / 255.0f),
          "hex alpha colour parses");
    CHECK(nearf(layers[1].color[0], 7.0f / 255.0f) &&
              nearf(layers[1].color[1], 8.0f / 255.0f) &&
              nearf(layers[1].color[2], 9.0f / 255.0f) &&
              nearf(layers[1].color[3], 0.25f),
          "rgba colour with spaces parses");

    memset(layers, 0, sizeof layers);
    n = gnoblin_shadow_parse_box_shadow("0 8px 24px -12px rgba(0,0,0,.18)",
                                        layers, GNOBLIN_SHADOW_MAX_LAYERS);
    CHECK(n == 1, "negative spread parses");
    CHECK(nearf(layers[0].spread, -12.0f), "negative spread is retained");
    CHECK(nearf(gnoblin_shadow_pad_for_layers(layers, n), 36.0f),
          "negative spread does not expand shadow pad");

    memset(layers, 0, sizeof layers);
    n = gnoblin_shadow_parse_box_shadow("0 8px 24px 6px rgba(0,0,0,.18)",
                                        layers, GNOBLIN_SHADOW_MAX_LAYERS);
    CHECK(n == 1, "positive spread parses");
    CHECK(nearf(layers[0].spread, 6.0f), "positive spread is retained");
    CHECK(nearf(gnoblin_shadow_pad_for_layers(layers, n), 42.0f),
          "positive spread expands shadow pad");

    CHECK(gnoblin_shadow_parse_css_color("rgb(1, 2, 3)", color) &&
              nearf(color[0], 1.0f / 255.0f) && nearf(color[3], 1.0f),
          "rgb colour defaults alpha to one");
    CHECK(!gnoblin_shadow_parse_css_color("rgb(1, 2, 3, 4)", color),
          "rgb rejects alpha channel");
    CHECK(!gnoblin_shadow_parse_css_color("rgba(1, 2, 3)", color),
          "rgba requires alpha channel");
    CHECK(!gnoblin_shadow_parse_css_color("#000000ff00", color),
          "hex colour rejects trailing bytes");
    CHECK(!gnoblin_shadow_parse_css_color("rgb(nan, 2, 3)", color),
          "rgb rejects non-finite channels");
    CHECK(!gnoblin_shadow_parse_css_color("rgba(1, 2, 3, inf)", color),
          "rgba rejects non-finite alpha");

    memset(layers, 0, sizeof layers);
    n = gnoblin_shadow_parse_box_shadow(
        "0 1 2 #000, 0 1 2 #111111, 0 1 2 #222222, 0 1 2 #333333, 0 1 2 #444444",
        layers, GNOBLIN_SHADOW_MAX_LAYERS);
    CHECK(n == 4, "box-shadow parser caps at four layers");

    memset(layers, 0, sizeof layers);
    CHECK(gnoblin_shadow_parse_box_shadow("", layers, GNOBLIN_SHADOW_MAX_LAYERS) == 0,
          "empty shadow disables shadow");
    CHECK(gnoblin_shadow_parse_box_shadow(NULL, layers, GNOBLIN_SHADOW_MAX_LAYERS) == 0,
          "NULL shadow disables shadow");
    CHECK(gnoblin_shadow_parse_box_shadow("0 0 -4 rgba(0,0,0,.2)", layers,
                                          GNOBLIN_SHADOW_MAX_LAYERS) == 0,
          "negative blur is rejected");
    CHECK(gnoblin_shadow_parse_box_shadow("0 0 4 rgba(0,0,0,2)", layers,
                                          GNOBLIN_SHADOW_MAX_LAYERS) == 0,
          "invalid alpha is rejected");
    CHECK(gnoblin_shadow_parse_box_shadow("0 0 4pt rgba(0,0,0,.2)", layers,
                                          GNOBLIN_SHADOW_MAX_LAYERS) == 0,
          "unsupported units are rejected");
    CHECK(gnoblin_shadow_parse_box_shadow("0 0 nan rgba(0,0,0,.2)", layers,
                                          GNOBLIN_SHADOW_MAX_LAYERS) == 0,
          "non-finite blur is rejected");
    CHECK(gnoblin_shadow_parse_box_shadow("0 1e309 4 rgba(0,0,0,.2)", layers,
                                          GNOBLIN_SHADOW_MAX_LAYERS) == 0,
          "non-finite offsets are rejected");

    if (fails == 0) {
        printf("PASS: CSS box-shadow parser\n");
        return 0;
    }
    return 1;
}
