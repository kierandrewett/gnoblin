#pragma once

#include <glib.h>

/* Greeter sessions are started by a login manager (greetd) with `gnoblin --greeter`.
 * User sessions are everything else. The guardian publishes the class in the environment so
 * the compositor, autostart children, and Lua can read it without threading a flag through. */
typedef enum {
    GNOBLIN_SESSION_CLASS_USER = 0,
    GNOBLIN_SESSION_CLASS_GREETER = 1,
} GnoblinSessionClass;

#define GNOBLIN_SESSION_CLASS_ENV "GNOBLIN_SESSION_CLASS"

static inline const char* gnoblin_session_class_name(GnoblinSessionClass session_class) {
    return session_class == GNOBLIN_SESSION_CLASS_GREETER ? "greeter" : "user";
}

static inline GnoblinSessionClass gnoblin_session_class_from_environment(void) {
    const char* value = g_getenv(GNOBLIN_SESSION_CLASS_ENV);
    return value && g_str_equal(value, "greeter") ? GNOBLIN_SESSION_CLASS_GREETER
                                                  : GNOBLIN_SESSION_CLASS_USER;
}
