#pragma once

#include <gio/gio.h>

G_BEGIN_DECLS

typedef struct _GnoblinKeybindingAction GnoblinKeybindingAction;
typedef struct _GnoblinKeybindingCatalog GnoblinKeybindingCatalog;

struct _GnoblinKeybindingAction {
    gchar* key;
    gchar* description;
    GStrv default_bindings;
};

GnoblinKeybindingCatalog* gnoblin_keybinding_catalog_load(GError** error);
void gnoblin_keybinding_catalog_free(GnoblinKeybindingCatalog* catalog);
const GPtrArray* gnoblin_keybinding_catalog_get_group(const GnoblinKeybindingCatalog* catalog,
                                                      const gchar* group);
const GnoblinKeybindingAction*
gnoblin_keybinding_catalog_lookup(const GnoblinKeybindingCatalog* catalog, const gchar* group,
                                  const gchar* key);

G_DEFINE_AUTOPTR_CLEANUP_FUNC(GnoblinKeybindingCatalog, gnoblin_keybinding_catalog_free)

G_END_DECLS
