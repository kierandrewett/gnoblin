#!/usr/bin/env bash
# Headless functional tests for security- and geometry-critical logic that the
# visual/lock features rely on (PAM rejection, the shadow/corner SDF). No display.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cc "$ROOT/tests/lock-pam-test.c" -lpam -o /tmp/gnoblin-lock-pam-test
cc "$ROOT/tests/lock-pam-accept-test.c" -lpam -o /tmp/gnoblin-lock-accept-test
cc "$ROOT/tests/sdf-test.c" -lm -o /tmp/gnoblin-sdf-test
cc "$ROOT/tests/anim-parser-test.c" "$ROOT/src/compositor/gnoblin-anim-spec.c" \
    "$ROOT/src/compositor/gnoblin-spec-util.c" \
    -I "$ROOT/src/compositor" $(pkg-config --cflags --libs glib-2.0) -lm \
    -o /tmp/gnoblin-anim-parser-test
cc "$ROOT/tests/actions-parser-test.c" "$ROOT/src/compositor/gnoblin-actions-spec.c" \
    "$ROOT/src/compositor/gnoblin-spec-util.c" \
    -I "$ROOT/src/compositor" $(pkg-config --cflags --libs glib-2.0) -lm \
    -o /tmp/gnoblin-actions-parser-test
cc "$ROOT/tests/color-parser-test.c" "$ROOT/src/compositor/gnoblin-color-spec.c" \
    -I "$ROOT/src/compositor" $(pkg-config --cflags --libs glib-2.0) -lm \
    -o /tmp/gnoblin-color-parser-test
cc "$ROOT/tests/input-parser-test.c" "$ROOT/src/compositor/gnoblin-input-spec.c" \
    "$ROOT/src/compositor/gnoblin-spec-util.c" \
    -I "$ROOT/src/compositor" $(pkg-config --cflags --libs glib-2.0) -lm \
    -o /tmp/gnoblin-input-parser-test
cc "$ROOT/tests/output-parser-test.c" "$ROOT/src/compositor/gnoblin-output-spec.c" \
    "$ROOT/src/compositor/gnoblin-spec-util.c" \
    -I "$ROOT/src/compositor" $(pkg-config --cflags --libs glib-2.0) -lm \
    -o /tmp/gnoblin-output-parser-test
cc "$ROOT/tests/rules-parser-test.c" "$ROOT/src/compositor/gnoblin-rules-spec.c" \
    "$ROOT/src/compositor/gnoblin-spec-util.c" \
    -I "$ROOT/src/compositor" $(pkg-config --cflags --libs glib-2.0) -lm \
    -o /tmp/gnoblin-rules-parser-test
cc "$ROOT/tests/shadow-parser-test.c" "$ROOT/src/compositor/gnoblin-shadow-spec.c" \
    "$ROOT/src/compositor/gnoblin-color-spec.c" \
    -I "$ROOT/src/compositor" $(pkg-config --cflags --libs glib-2.0) -lm \
    -o /tmp/gnoblin-shadow-parser-test
cc "$ROOT/tests/config-test.c" "$ROOT/src/config/gnoblin-config.c" \
    -I "$ROOT/src/config" $(pkg-config --cflags --libs glib-2.0) -o /tmp/gnoblin-config-test
cc "$ROOT/tests/config-example-test.c" "$ROOT/src/config/gnoblin-config.c" \
    -I "$ROOT/src/config" $(pkg-config --cflags --libs glib-2.0) -o /tmp/gnoblin-config-example-test
echo "== lock screen PAM auth (reject wrong) =="; /tmp/gnoblin-lock-pam-test
echo "== lock screen PAM auth (unlock on success / stay locked on fail) =="; /tmp/gnoblin-lock-accept-test
echo "== shadow/rounded-corner SDF geometry =="; /tmp/gnoblin-sdf-test
echo "== animation numeric parser =="; /tmp/gnoblin-anim-parser-test
echo "== action argument parser =="; /tmp/gnoblin-actions-parser-test
echo "== hex colour parser =="; /tmp/gnoblin-color-parser-test
echo "== input config parser =="; /tmp/gnoblin-input-parser-test
echo "== output config parser =="; /tmp/gnoblin-output-parser-test
echo "== window rules parser =="; /tmp/gnoblin-rules-parser-test
echo "== CSS box-shadow parser =="; /tmp/gnoblin-shadow-parser-test
echo "== gnoblin.conf parser =="; /tmp/gnoblin-config-test
echo "== gnoblin.conf.example (shipped default) =="; /tmp/gnoblin-config-example-test "$ROOT/src/data/gnoblin.conf.example"
echo ">> logic tests passed"
