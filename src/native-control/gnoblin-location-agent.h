/* GeoClue2 authorization agent for the native Gnoblin runtime. */
#pragma once

#include <gio/gio.h>

G_BEGIN_DECLS

typedef struct _GnoblinLocationAgent GnoblinLocationAgent;
typedef struct _GnoblinLocationRequest GnoblinLocationRequest;

typedef enum {
    GNOBLIN_LOCATION_ACCURACY_INHERIT = 0,
    GNOBLIN_LOCATION_ACCURACY_COUNTRY = 1,
    GNOBLIN_LOCATION_ACCURACY_CITY = 4,
    GNOBLIN_LOCATION_ACCURACY_NEIGHBORHOOD = 5,
    GNOBLIN_LOCATION_ACCURACY_STREET = 6,
    GNOBLIN_LOCATION_ACCURACY_EXACT = 8,
} GnoblinLocationAccuracy;

typedef void (*GnoblinLocationAuthorizeFunc)(GnoblinLocationAgent* agent, const char* app_id,
                                             guint requested_accuracy,
                                             GnoblinLocationRequest* request, gpointer user_data);
typedef void (*GnoblinLocationStateFunc)(GnoblinLocationAgent* agent, gboolean available,
                                         gboolean in_use, gpointer user_data);

GnoblinLocationAgent* gnoblin_location_agent_new(GMainContext* context,
                                                 GnoblinLocationAuthorizeFunc authorize,
                                                 GnoblinLocationStateFunc state_changed,
                                                 gpointer user_data,
                                                 GDestroyNotify user_data_destroy);

/* Start/stop are safe from any thread. Authorization callbacks run on context. */
void gnoblin_location_agent_start(GnoblinLocationAgent* agent);
void gnoblin_location_agent_stop(GnoblinLocationAgent* agent);
void gnoblin_location_agent_free(GnoblinLocationAgent* agent);

/* Lua overrides take precedence field-by-field. Unset fields follow the
 * corresponding org.gnome.system.location compatibility setting. */
void gnoblin_location_agent_set_policy(GnoblinLocationAgent* agent, gboolean enabled_set,
                                       gboolean enabled, gboolean accuracy_set,
                                       GnoblinLocationAccuracy accuracy);

/* Requests are borrowed during authorize(); ref them to retain asynchronously. */
GnoblinLocationRequest* gnoblin_location_request_ref(GnoblinLocationRequest* request);
void gnoblin_location_request_unref(GnoblinLocationRequest* request);
/* Safe from any thread. Late or duplicate completions are ignored. */
void gnoblin_location_request_complete(GnoblinLocationRequest* request, gboolean allowed,
                                       guint accuracy_level);

G_DEFINE_AUTOPTR_CLEANUP_FUNC(GnoblinLocationAgent, gnoblin_location_agent_free)
G_DEFINE_AUTOPTR_CLEANUP_FUNC(GnoblinLocationRequest, gnoblin_location_request_unref)

G_END_DECLS
