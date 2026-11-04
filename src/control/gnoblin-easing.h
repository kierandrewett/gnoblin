/* Easing curves shared by the Lua runtime (gnoblin.ease) and the compositor's tween player, so a
 * curve sampled in Lua matches the one the compositor plays. Pure C with no dependencies; the
 * control manifest copies it into Mutter as core/gnoblin-easing.h.
 *
 * Names follow the usual families. `in` starts slowly, `out` ends slowly and `in_out` does
 * both. linear ignores the direction. */
#ifndef GNOBLIN_EASING_H
#define GNOBLIN_EASING_H

#include <math.h>
#include <string.h>

typedef enum {
    GNOBLIN_EASE_LINEAR,
    GNOBLIN_EASE_SINE,
    GNOBLIN_EASE_QUAD,
    GNOBLIN_EASE_CUBIC,
    GNOBLIN_EASE_QUART,
    GNOBLIN_EASE_QUINT,
    GNOBLIN_EASE_EXPO,
    GNOBLIN_EASE_CIRC,
    GNOBLIN_EASE_BACK,
    GNOBLIN_EASE_ELASTIC,
    GNOBLIN_EASE_BOUNCE,
} GnoblinEase;

typedef enum {
    GNOBLIN_EASE_IN,
    GNOBLIN_EASE_OUT,
    GNOBLIN_EASE_IN_OUT,
} GnoblinEaseDirection;

/* Look up a curve or direction by name; returns -1 for an unknown name. */
static inline int gnoblin_ease_from_name(const char* name) {
    static const char* const names[] = {"linear", "sine", "quad", "cubic",   "quart", "quint",
                                        "expo",   "circ", "back", "elastic", "bounce"};
    for (int i = 0; name && i < (int)(sizeof names / sizeof *names); i++)
        if (strcmp(name, names[i]) == 0)
            return i;
    return -1;
}

static inline int gnoblin_ease_direction_from_name(const char* name) {
    static const char* const names[] = {"in", "out", "in_out"};
    for (int i = 0; name && i < 3; i++)
        if (strcmp(name, names[i]) == 0)
            return i;
    return -1;
}

/* The `in` form of each curve, for t from 0 to 1. */
static inline double gnoblin_ease_in(GnoblinEase ease, double t) {
    const double pi = 3.14159265358979323846;
    switch (ease) {
    case GNOBLIN_EASE_LINEAR:
        return t;
    case GNOBLIN_EASE_SINE:
        return 1.0 - cos(t * pi / 2.0);
    case GNOBLIN_EASE_QUAD:
        return t * t;
    case GNOBLIN_EASE_CUBIC:
        return t * t * t;
    case GNOBLIN_EASE_QUART:
        return t * t * t * t;
    case GNOBLIN_EASE_QUINT:
        return t * t * t * t * t;
    case GNOBLIN_EASE_EXPO:
        return t <= 0.0 ? 0.0 : pow(2.0, 10.0 * t - 10.0);
    case GNOBLIN_EASE_CIRC:
        return 1.0 - sqrt(1.0 - t * t);
    case GNOBLIN_EASE_BACK:
        return 2.70158 * t * t * t - 1.70158 * t * t;
    case GNOBLIN_EASE_ELASTIC:
        if (t <= 0.0 || t >= 1.0)
            return t;
        return -pow(2.0, 10.0 * t - 10.0) * sin((t * 10.0 - 10.75) * (2.0 * pi / 3.0));
    case GNOBLIN_EASE_BOUNCE: {
        /* bounce `in` mirrors the `out` curve below. */
        double u = 1.0 - t;
        double out;
        if (u < 1.0 / 2.75) {
            out = 7.5625 * u * u;
        } else if (u < 2.0 / 2.75) {
            u -= 1.5 / 2.75;
            out = 7.5625 * u * u + 0.75;
        } else if (u < 2.5 / 2.75) {
            u -= 2.25 / 2.75;
            out = 7.5625 * u * u + 0.9375;
        } else {
            u -= 2.625 / 2.75;
            out = 7.5625 * u * u + 0.984375;
        }
        return 1.0 - out;
    }
    }
    return t;
}

/* Eased progress for t, clamped to 0..1. */
static inline double gnoblin_ease(GnoblinEase ease, GnoblinEaseDirection direction, double t) {
    t = t < 0.0 ? 0.0 : t > 1.0 ? 1.0 : t;
    switch (direction) {
    case GNOBLIN_EASE_IN:
        return gnoblin_ease_in(ease, t);
    case GNOBLIN_EASE_OUT:
        return 1.0 - gnoblin_ease_in(ease, 1.0 - t);
    case GNOBLIN_EASE_IN_OUT:
        return t < 0.5 ? gnoblin_ease_in(ease, 2.0 * t) / 2.0
                       : 1.0 - gnoblin_ease_in(ease, 2.0 - 2.0 * t) / 2.0;
    }
    return t;
}

#endif
