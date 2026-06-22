/*
 * gnoblin: ext-data-control-v1 support for mutter.
 *
 * Implements the standardized data-control protocol (the successor to
 * wlr-data-control) on top of mutter's MetaSelection. This lets clipboard
 * managers such as cliphist, clipman and wl-clipboard read and set the
 * clipboard and primary selection without keyboard focus, in a Gnoblin
 * session.
 *
 * A client data source is bridged into MetaSelection through a
 * MetaSelectionSource subclass (mirroring mutter's own
 * meta-selection-source-wayland.c): reads are serviced by piping the request
 * back to the owning client via the source's "send" event; writes (receive)
 * pull the current selection out of MetaSelection into the client's fd.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "config.h"

#include "wayland/meta-wayland-data-control.h"

#include <gio/gio.h>
#include <gio/gunixinputstream.h>
#include <gio/gunixoutputstream.h>
#include <glib-unix.h>

#include "meta/display.h"
#include "meta/meta-context.h"
#include "meta/meta-selection-source.h"
#include "meta/meta-selection.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/gnoblin-config.h"

#include "ext-data-control-v1-server-protocol.h"

#define META_EXT_DATA_CONTROL_VERSION 1

/* ------------------------------------------------------------------ */
/* MetaSelectionSource subclass bridging a client ext_data_control_source */

#define META_TYPE_DATA_CONTROL_SOURCE (meta_data_control_source_get_type ())
G_DECLARE_FINAL_TYPE (MetaDataControlSource, meta_data_control_source,
                      META, DATA_CONTROL_SOURCE, MetaSelectionSource)

struct _MetaDataControlSource
{
  MetaSelectionSource parent_instance;
  struct wl_resource *resource; /* NULL once the client source is gone */
  GList *mimetypes;
  gboolean used;                /* assigned to a selection already */
  MetaSelection *selection;     /* set while owning a selection, else NULL */
  MetaSelectionType sel_type;
};

G_DEFINE_TYPE (MetaDataControlSource, meta_data_control_source,
               META_TYPE_SELECTION_SOURCE)

static void
meta_data_control_source_finalize (GObject *object)
{
  MetaDataControlSource *source = META_DATA_CONTROL_SOURCE (object);

  g_list_free_full (source->mimetypes, g_free);

  G_OBJECT_CLASS (meta_data_control_source_parent_class)->finalize (object);
}

static void
meta_data_control_source_read_async (MetaSelectionSource *source_base,
                                     const gchar         *mimetype,
                                     GCancellable        *cancellable,
                                     GAsyncReadyCallback  callback,
                                     gpointer             user_data)
{
  MetaDataControlSource *source = META_DATA_CONTROL_SOURCE (source_base);
  GInputStream *stream;
  GTask *task;
  int pipe_fds[2];

  if (!source->resource)
    {
      g_task_report_new_error (source, callback, user_data,
                               meta_data_control_source_read_async,
                               G_IO_ERROR, G_IO_ERROR_CLOSED,
                               "data-control source is gone");
      return;
    }

  if (!g_unix_open_pipe (pipe_fds, FD_CLOEXEC, NULL))
    {
      g_task_report_new_error (source, callback, user_data,
                               meta_data_control_source_read_async,
                               G_IO_ERROR, G_IO_ERROR_FAILED,
                               "Could not open pipe to read data-control selection");
      return;
    }

  if (!g_unix_set_fd_nonblocking (pipe_fds[0], TRUE, NULL) ||
      !g_unix_set_fd_nonblocking (pipe_fds[1], TRUE, NULL))
    {
      g_task_report_new_error (source, callback, user_data,
                               meta_data_control_source_read_async,
                               G_IO_ERROR, G_IO_ERROR_FAILED,
                               "Could not make pipe nonblocking");
      close (pipe_fds[0]);
      close (pipe_fds[1]);
      return;
    }

  task = g_task_new (source, cancellable, callback, user_data);
  g_task_set_source_tag (task, meta_data_control_source_read_async);

  stream = g_unix_input_stream_new (pipe_fds[0], TRUE);
  ext_data_control_source_v1_send_send (source->resource, mimetype, pipe_fds[1]);
  close (pipe_fds[1]);

  g_task_return_pointer (task, stream, g_object_unref);
  g_object_unref (task);
}

static GInputStream *
meta_data_control_source_read_finish (MetaSelectionSource  *source,
                                      GAsyncResult         *result,
                                      GError              **error)
{
  g_return_val_if_fail (g_task_is_valid (result, source), NULL);

  return g_task_propagate_pointer (G_TASK (result), error);
}

static GList *
meta_data_control_source_get_mimetypes (MetaSelectionSource *source_base)
{
  MetaDataControlSource *source = META_DATA_CONTROL_SOURCE (source_base);

  return g_list_copy_deep (source->mimetypes, (GCopyFunc) g_strdup, NULL);
}

static void
meta_data_control_source_deactivated (MetaSelectionSource *source_base)
{
  MetaDataControlSource *source = META_DATA_CONTROL_SOURCE (source_base);

  /* No longer the selection owner. */
  source->selection = NULL;
  if (source->resource)
    ext_data_control_source_v1_send_cancelled (source->resource);

  META_SELECTION_SOURCE_CLASS (meta_data_control_source_parent_class)->deactivated (source_base);
}

static void
meta_data_control_source_class_init (MetaDataControlSourceClass *klass)
{
  GObjectClass *object_class = G_OBJECT_CLASS (klass);
  MetaSelectionSourceClass *source_class = META_SELECTION_SOURCE_CLASS (klass);

  object_class->finalize = meta_data_control_source_finalize;

  source_class->read_async = meta_data_control_source_read_async;
  source_class->read_finish = meta_data_control_source_read_finish;
  source_class->get_mimetypes = meta_data_control_source_get_mimetypes;
  source_class->deactivated = meta_data_control_source_deactivated;
}

static void
meta_data_control_source_init (MetaDataControlSource *source)
{
}

/* ------------------------------------------------------------------ */

typedef struct _MetaWaylandDataControl
{
  MetaWaylandCompositor *compositor;
  GList *devices; /* MetaWaylandDataControlDevice* */
} MetaWaylandDataControl;

/* The MetaDisplay (and its MetaSelection) does not exist yet at shell-init
 * time, so resolve it lazily — every caller runs after a client has bound,
 * by which point the display is up. */
static MetaSelection *
data_control_selection (MetaWaylandDataControl *data_control)
{
  MetaContext *context =
    meta_wayland_compositor_get_context (data_control->compositor);

  return meta_display_get_selection (meta_context_get_display (context));
}

typedef struct _MetaWaylandDataControlDevice
{
  struct wl_resource *resource;
  MetaWaylandDataControl *data_control;
  gulong owner_changed_id;
  /* The source this device last assigned to each selection, used to honour
   * set_selection(null) without clobbering other clients' selections. */
  MetaDataControlSource *clipboard_source;
  MetaDataControlSource *primary_source;
} MetaWaylandDataControlDevice;

typedef struct _MetaWaylandDataControlOffer
{
  struct wl_resource *resource;
  MetaWaylandDataControl *data_control;
  MetaSelectionType selection_type;
} MetaWaylandDataControlOffer;

/* ---- source ---- */

static void
data_control_source_destroy (struct wl_resource *resource)
{
  MetaDataControlSource *source = wl_resource_get_user_data (resource);

  source->resource = NULL;

  /* If this source is still the active selection, clear it so clients are not
   * advertised a selection that can no longer be read. */
  if (source->selection)
    {
      meta_selection_unset_owner (source->selection, source->sel_type,
                                  META_SELECTION_SOURCE (source));
    }

  g_object_unref (source);
}

static void
data_control_source_offer (struct wl_client   *client,
                           struct wl_resource *resource,
                           const char         *mime_type)
{
  MetaDataControlSource *source = wl_resource_get_user_data (resource);

  if (source->used)
    {
      wl_resource_post_error (resource,
                              EXT_DATA_CONTROL_SOURCE_V1_ERROR_INVALID_OFFER,
                              "offer after the source was used in a selection");
      return;
    }

  source->mimetypes = g_list_append (source->mimetypes, g_strdup (mime_type));
}

static void
data_control_source_handle_destroy (struct wl_client   *client,
                                    struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct ext_data_control_source_v1_interface source_interface = {
  data_control_source_offer,
  data_control_source_handle_destroy,
};

static void
manager_create_data_source (struct wl_client   *client,
                            struct wl_resource *manager_resource,
                            uint32_t            id)
{
  MetaDataControlSource *source;
  struct wl_resource *resource;

  resource = wl_resource_create (client, &ext_data_control_source_v1_interface,
                                 wl_resource_get_version (manager_resource), id);
  if (!resource)
    {
      wl_client_post_no_memory (client);
      return;
    }

  source = g_object_new (META_TYPE_DATA_CONTROL_SOURCE, NULL);
  source->resource = resource;

  wl_resource_set_implementation (resource, &source_interface, source,
                                  data_control_source_destroy);
}

/* ---- offer ---- */

static void
data_control_offer_transfer_cb (GObject      *object,
                                GAsyncResult *result,
                                gpointer      user_data)
{
  MetaSelection *selection = META_SELECTION (object);
  g_autoptr (GError) error = NULL;

  if (!meta_selection_transfer_finish (selection, result, &error))
    g_debug ("data-control transfer failed: %s", error ? error->message : "?");
}

static void
data_control_offer_receive (struct wl_client   *client,
                            struct wl_resource *resource,
                            const char         *mime_type,
                            int32_t             fd)
{
  MetaWaylandDataControlOffer *offer = wl_resource_get_user_data (resource);
  GOutputStream *stream;

  stream = g_unix_output_stream_new (fd, TRUE);
  meta_selection_transfer_async (data_control_selection (offer->data_control),
                                 offer->selection_type,
                                 mime_type,
                                 -1,
                                 stream,
                                 NULL,
                                 data_control_offer_transfer_cb,
                                 NULL);
  g_object_unref (stream);
}

static void
data_control_offer_handle_destroy (struct wl_client   *client,
                                   struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct ext_data_control_offer_v1_interface offer_interface = {
  data_control_offer_receive,
  data_control_offer_handle_destroy,
};

static void
data_control_offer_destroy (struct wl_resource *resource)
{
  MetaWaylandDataControlOffer *offer = wl_resource_get_user_data (resource);

  g_free (offer);
}

/* ---- device ---- */

static void
device_advertise_selection (MetaWaylandDataControlDevice *device,
                            MetaSelectionType             type)
{
  struct wl_client *client = wl_resource_get_client (device->resource);
  MetaSelection *selection = data_control_selection (device->data_control);
  GList *mimetypes;
  MetaWaylandDataControlOffer *offer;
  struct wl_resource *offer_resource;
  GList *l;

  mimetypes = meta_selection_get_mimetypes (selection, type);

  if (!mimetypes)
    {
      if (type == META_SELECTION_CLIPBOARD)
        ext_data_control_device_v1_send_selection (device->resource, NULL);
      else if (type == META_SELECTION_PRIMARY)
        ext_data_control_device_v1_send_primary_selection (device->resource, NULL);
      return;
    }

  offer_resource = wl_resource_create (client,
                                       &ext_data_control_offer_v1_interface,
                                       wl_resource_get_version (device->resource),
                                       0);
  if (!offer_resource)
    {
      wl_client_post_no_memory (client);
      return;
    }

  offer = g_new0 (MetaWaylandDataControlOffer, 1);
  offer->resource = offer_resource;
  offer->data_control = device->data_control;
  offer->selection_type = type;
  wl_resource_set_implementation (offer_resource, &offer_interface, offer,
                                  data_control_offer_destroy);

  ext_data_control_device_v1_send_data_offer (device->resource, offer_resource);

  for (l = mimetypes; l; l = l->next)
    ext_data_control_offer_v1_send_offer (offer_resource, l->data);

  if (type == META_SELECTION_CLIPBOARD)
    ext_data_control_device_v1_send_selection (device->resource, offer_resource);
  else if (type == META_SELECTION_PRIMARY)
    ext_data_control_device_v1_send_primary_selection (device->resource, offer_resource);

  g_list_free_full (mimetypes, g_free);
}

static void
on_owner_changed (MetaSelection       *selection,
                  guint                selection_type,
                  MetaSelectionSource *new_owner,
                  gpointer             user_data)
{
  MetaWaylandDataControlDevice *device = user_data;

  if (selection_type != META_SELECTION_CLIPBOARD &&
      selection_type != META_SELECTION_PRIMARY)
    return;

  /* Drop our tracked source pointer once we are no longer the owner, so a
   * later set_selection(null) cannot dereference a freed source. */
  if (selection_type == META_SELECTION_CLIPBOARD &&
      new_owner != META_SELECTION_SOURCE (device->clipboard_source))
    device->clipboard_source = NULL;
  if (selection_type == META_SELECTION_PRIMARY &&
      new_owner != META_SELECTION_SOURCE (device->primary_source))
    device->primary_source = NULL;

  device_advertise_selection (device, selection_type);
}

static void
device_set_selection (MetaWaylandDataControlDevice *device,
                      MetaSelectionType             type,
                      struct wl_resource           *source_resource,
                      MetaDataControlSource       **tracked)
{
  MetaSelection *selection = data_control_selection (device->data_control);

  if (source_resource)
    {
      MetaDataControlSource *source =
        wl_resource_get_user_data (source_resource);

      if (source->used)
        {
          wl_resource_post_error (device->resource,
                                  EXT_DATA_CONTROL_DEVICE_V1_ERROR_USED_SOURCE,
                                  "data source already used");
          return;
        }

      source->used = TRUE;
      source->selection = selection;
      source->sel_type = type;
      *tracked = source;
      meta_selection_set_owner (selection, type, META_SELECTION_SOURCE (source));
    }
  else
    {
      /* Clear, but only if we still own the selection we set. */
      if (*tracked)
        {
          meta_selection_unset_owner (selection, type,
                                      META_SELECTION_SOURCE (*tracked));
          *tracked = NULL;
        }
    }
}

static void
device_set_selection_request (struct wl_client   *client,
                              struct wl_resource *resource,
                              struct wl_resource *source_resource)
{
  MetaWaylandDataControlDevice *device = wl_resource_get_user_data (resource);

  device_set_selection (device, META_SELECTION_CLIPBOARD, source_resource,
                        &device->clipboard_source);
}

static void
device_set_primary_selection_request (struct wl_client   *client,
                                      struct wl_resource *resource,
                                      struct wl_resource *source_resource)
{
  MetaWaylandDataControlDevice *device = wl_resource_get_user_data (resource);

  device_set_selection (device, META_SELECTION_PRIMARY, source_resource,
                        &device->primary_source);
}

static void
device_handle_destroy (struct wl_client   *client,
                       struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct ext_data_control_device_v1_interface device_interface = {
  device_set_selection_request,
  device_handle_destroy,
  device_set_primary_selection_request,
};

static void
device_destroy (struct wl_resource *resource)
{
  MetaWaylandDataControlDevice *device = wl_resource_get_user_data (resource);

  g_clear_signal_handler (&device->owner_changed_id,
                          data_control_selection (device->data_control));
  device->data_control->devices =
    g_list_remove (device->data_control->devices, device);

  g_free (device);
}

static void
manager_get_data_device (struct wl_client   *client,
                         struct wl_resource *manager_resource,
                         uint32_t            id,
                         struct wl_resource *seat_resource)
{
  MetaWaylandDataControl *data_control =
    wl_resource_get_user_data (manager_resource);
  MetaWaylandDataControlDevice *device;
  struct wl_resource *resource;

  resource = wl_resource_create (client, &ext_data_control_device_v1_interface,
                                 wl_resource_get_version (manager_resource), id);
  if (!resource)
    {
      wl_client_post_no_memory (client);
      return;
    }

  device = g_new0 (MetaWaylandDataControlDevice, 1);
  device->resource = resource;
  device->data_control = data_control;
  wl_resource_set_implementation (resource, &device_interface, device,
                                  device_destroy);

  data_control->devices = g_list_prepend (data_control->devices, device);

  device->owner_changed_id =
    g_signal_connect (data_control_selection (data_control), "owner-changed",
                      G_CALLBACK (on_owner_changed), device);

  /* Advertise the current selections immediately. */
  device_advertise_selection (device, META_SELECTION_CLIPBOARD);
  device_advertise_selection (device, META_SELECTION_PRIMARY);
}

static void
manager_handle_destroy (struct wl_client   *client,
                        struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct ext_data_control_manager_v1_interface manager_interface = {
  manager_create_data_source,
  manager_get_data_device,
  manager_handle_destroy,
};

static void
bind_data_control_manager (struct wl_client *client,
                           void             *data,
                           uint32_t          version,
                           uint32_t          id)
{
  MetaWaylandDataControl *data_control = data;
  struct wl_resource *resource;

  resource = wl_resource_create (client,
                                 &ext_data_control_manager_v1_interface,
                                 version, id);
  if (!resource)
    {
      wl_client_post_no_memory (client);
      return;
    }

  wl_resource_set_implementation (resource, &manager_interface, data_control,
                                  NULL);
}

void
meta_wayland_init_data_control (MetaWaylandCompositor *compositor)
{
  MetaWaylandDataControl *data_control;

  if (!gnoblin_config_get_bool ("protocols", "ext-data-control", TRUE))
    {
      g_message ("Gnoblin ext-data-control protocol disabled by settings");
      return;
    }

  data_control = g_new0 (MetaWaylandDataControl, 1);
  /* The display/selection is resolved lazily (see data_control_selection);
   * it does not exist yet at shell-init time. */
  data_control->compositor = compositor;

  if (!wl_global_create (compositor->wayland_display,
                         &ext_data_control_manager_v1_interface,
                         META_EXT_DATA_CONTROL_VERSION,
                         data_control,
                         bind_data_control_manager))
    g_error ("Failed to register ext-data-control global");
}
