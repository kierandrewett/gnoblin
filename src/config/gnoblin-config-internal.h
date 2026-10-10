#ifndef GNOBLIN_CONFIG_INTERNAL_H
#define GNOBLIN_CONFIG_INTERNAL_H

/* Lua configuration evaluation and supervisor runtime state. */
#include "gnoblin-config.h"
#include "gnoblin-portal-policy.h"
#include "gnoblin-input-config.h"
#include "gnoblin-keybinding-catalog.h"
#include <errno.h>
#include <gio/gio.h>
#include <glib/gstdio.h>
#include <math.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
#include <xkbcommon/xkbcommon.h>
typedef enum {
    KEYBOARD_ACCESSIBILITY_BOOLEAN,
    KEYBOARD_ACCESSIBILITY_INTEGER,
} KeyboardAccessibilityValueType;
typedef struct {
    const char* section;
    const char* name;
    KeyboardAccessibilityValueType type;
    gint64 minimum;
    gint64 maximum;
} KeyboardAccessibilityField;
typedef struct {
    char* kind;
    gint64 fingers;
    char* direction;
    GVariant* path;
    char* when;
} TouchpadGestureInput;
#define LAST_GOOD_DOCUMENT_MAX_BYTES (16u * 1024u * 1024u)
/* Partial recovery. A document that fails validation is not thrown away. Each round finds the
 * smallest key or list entry whose removal changes the validation error, drops it and tries again.
 * Validators report only the first failure, so removing the culprit is what moves the error on. The
 * caller gets every valid setting plus a list of what was ignored. */
#define SALVAGE_MAX_DROPS 32
#define SALVAGE_MAX_VALIDATIONS 4000

/* Declarations shared between the split files. */
gboolean input_number(GVariant* value, double* number);
gboolean validate_xwayland(GVariant* xwayland, GError** error);
gboolean input_string_is(GVariant* value, const char* const* choices);
gboolean validate_location(GVariant* location, GError** error);
gboolean validate_auth(GVariant* auth, GError** error);
gboolean validate_prompts(GVariant* prompts, GError** error);
gboolean validate_input(GVariant* input, GError** error);
gboolean window_modifier_valid(const char* modifier);
gboolean compositor_locate_pointer_key_valid(const char* name);
gboolean validate_touchpad_gestures(GVariant* gestures, GError** error);
gboolean validate_window_rule_patterns(GVariant* document, GError** error);
gboolean validate_portal_selection(GVariant* portals, GError** error);
gboolean titlebar_button_layout_valid(GVariant* layout);
gboolean validate_monitors(GVariant* monitors, GError** error);
gboolean validate_input_handlers(GVariant* document, GError** error);
gboolean validate_keybindings(GVariant* document, GError** error);

#endif /* GNOBLIN_CONFIG_INTERNAL_H */
