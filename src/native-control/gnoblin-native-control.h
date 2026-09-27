/* Native control endpoint for the standalone Gnoblin compositor preview. */
#pragma once

#include <glib.h>

#include "meta/meta-context.h"

typedef struct _GnoblinNativeControl GnoblinNativeControl;

GnoblinNativeControl* gnoblin_native_control_start(MetaContext* context, GVariant* document,
                                                   GError** error);
void gnoblin_native_control_stop(GnoblinNativeControl* control);
