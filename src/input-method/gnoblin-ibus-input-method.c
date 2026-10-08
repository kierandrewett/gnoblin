/* -*- mode: C; c-file-style: "gnu"; indent-tabs-mode: nil; -*- */

/*
 * Copyright (C) 2026 Gnoblin contributors
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the
 * Free Software Foundation; either version 2 of the License, or (at your
 * option) any later version.
 */

/*
 * GNOME Shell implements this bridge in JavaScript (js/misc/inputMethod.js). Gnoblin has no
 * shell, so the compositor does it: one IBus input context mirrors the focused text field
 * (cursor, surrounding text, content hints and purpose), and IBus signals become commit,
 * preedit, delete-surrounding and forwarded-key calls on the Clutter input method.
 */

#include "config.h"

#include "backends/gnoblin-ibus-input-method.h"

#include <ibus.h>

#include "backends/meta-backend-private.h"

#define GNOBLIN_TYPE_IBUS_INPUT_METHOD (gnoblin_ibus_input_method_get_type ())
G_DECLARE_FINAL_TYPE (GnoblinIbusInputMethod, gnoblin_ibus_input_method,
                      GNOBLIN, IBUS_INPUT_METHOD, ClutterInputMethod)

struct _GnoblinIbusInputMethod
{
  ClutterInputMethod parent;

  IBusBus *bus;
  IBusInputContext *context;
  GCancellable *cancellable;

  guint ibus_hints;
  guint ibus_purpose;

  char *preedit;
  guint preedit_position;
  guint preedit_anchor;
  gboolean preedit_visible;
  ClutterPreeditResetMode preedit_mode;
  ClutterPreeditAttribute *preedit_attributes;
  guint n_preedit_attributes;

  char *surrounding_text;
  guint surrounding_cursor;
  guint surrounding_anchor;

  graphene_rect_t cursor_rect;
  gboolean has_cursor_rect;
};

G_DEFINE_FINAL_TYPE (GnoblinIbusInputMethod, gnoblin_ibus_input_method,
                     CLUTTER_TYPE_INPUT_METHOD)

static void create_context (GnoblinIbusInputMethod *self);

static void
clear_preedit (GnoblinIbusInputMethod *self)
{
  g_clear_pointer (&self->preedit, g_free);
  g_clear_pointer (&self->preedit_attributes, g_free);
  self->n_preedit_attributes = 0;
  self->preedit_position = 0;
  self->preedit_anchor = 0;
  self->preedit_visible = FALSE;
}

static void
clear_context (GnoblinIbusInputMethod *self)
{
  if (self->cancellable)
    {
      g_cancellable_cancel (self->cancellable);
      g_clear_object (&self->cancellable);
    }

  g_clear_object (&self->context);
  self->ibus_hints = 0;
  self->ibus_purpose = 0;
  clear_preedit (self);
}

static void
update_capabilities (GnoblinIbusInputMethod *self)
{
  guint caps = IBUS_CAP_PREEDIT_TEXT | IBUS_CAP_FOCUS;

  if (!self->context)
    return;

  if (self->surrounding_text)
    caps |= IBUS_CAP_SURROUNDING_TEXT;

  ibus_input_context_set_capabilities (self->context, caps);
}

static void
request_surrounding_if_needed (GnoblinIbusInputMethod *self)
{
  if (self->context && ibus_input_context_needs_surrounding_text (self->context))
    clutter_input_method_request_surrounding (CLUTTER_INPUT_METHOD (self));
}

static void
update_context (GnoblinIbusInputMethod *self)
{
  if (!self->context)
    return;

  update_capabilities (self);
  ibus_input_context_set_content_type (self->context, self->ibus_purpose, self->ibus_hints);
  if (self->has_cursor_rect)
    {
      ibus_input_context_set_cursor_location (self->context,
                                              (int) self->cursor_rect.origin.x,
                                              (int) self->cursor_rect.origin.y,
                                              (int) self->cursor_rect.size.width,
                                              (int) self->cursor_rect.size.height);
    }
  request_surrounding_if_needed (self);
}

static void
on_commit_text (IBusInputContext       *context,
                IBusText               *text,
                GnoblinIbusInputMethod *self)
{
  clutter_input_method_commit (CLUTTER_INPUT_METHOD (self), ibus_text_get_text (text));
}

static void
on_require_surrounding_text (IBusInputContext       *context,
                             GnoblinIbusInputMethod *self)
{
  clutter_input_method_request_surrounding (CLUTTER_INPUT_METHOD (self));
}

static void
on_delete_surrounding_text (IBusInputContext       *context,
                            int                     offset,
                            guint                   n_chars,
                            GnoblinIbusInputMethod *self)
{
  /* Engines may only delete text when the client gave surrounding text. */
  if (!self->surrounding_text)
    {
      g_debug ("IBus engine asked to delete surrounding text, but the client has none");
      return;
    }

  clutter_input_method_delete_surrounding (CLUTTER_INPUT_METHOD (self), offset, n_chars);
}

static ClutterPreeditStyleHint
preedit_hint_from_ibus (guint value,
                        gboolean *known)
{
  *known = TRUE;
  switch (value)
    {
    case IBUS_ATTR_PREEDIT_DEFAULT:
      return CLUTTER_PREEDIT_STYLE_NONE;
    case IBUS_ATTR_PREEDIT_WHOLE:
      return CLUTTER_PREEDIT_STYLE_WHOLE;
    case IBUS_ATTR_PREEDIT_SELECTION:
      return CLUTTER_PREEDIT_STYLE_SELECTION;
    case IBUS_ATTR_PREEDIT_PREDICTION:
      return CLUTTER_PREEDIT_STYLE_PREDICTION;
    case IBUS_ATTR_PREEDIT_PREFIX:
      return CLUTTER_PREEDIT_STYLE_PREFIX;
    case IBUS_ATTR_PREEDIT_SUFFIX:
      return CLUTTER_PREEDIT_STYLE_SUFFIX;
    case IBUS_ATTR_PREEDIT_ERROR_SPELLING:
      return CLUTTER_PREEDIT_STYLE_SPELLING_ERROR;
    case IBUS_ATTR_PREEDIT_ERROR_COMPOSE:
      return CLUTTER_PREEDIT_STYLE_COMPOSE_ERROR;
    default:
      *known = FALSE;
      return CLUTTER_PREEDIT_STYLE_NONE;
    }
}

static void
store_preedit_attributes (GnoblinIbusInputMethod *self,
                          IBusText               *text)
{
  IBusAttrList *list = ibus_text_get_attributes (text);
  GArray *attributes = g_array_new (FALSE, FALSE, sizeof (ClutterPreeditAttribute));

  g_clear_pointer (&self->preedit_attributes, g_free);
  self->n_preedit_attributes = 0;

  for (guint i = 0; list; i++)
    {
      IBusAttribute *attribute = ibus_attr_list_get (list, i);
      ClutterPreeditAttribute converted;
      gboolean known;

      if (!attribute)
        break;

      /* Ignore explicit style attributes. Only the preedit hints are mapped. */
      if (ibus_attribute_get_attr_type (attribute) != IBUS_ATTR_TYPE_HINT)
        continue;

      converted.hint = preedit_hint_from_ibus (ibus_attribute_get_value (attribute), &known);
      if (!known)
        continue;

      converted.start = ibus_attribute_get_start_index (attribute);
      converted.end = ibus_attribute_get_end_index (attribute);
      g_array_append_val (attributes, converted);
    }

  self->n_preedit_attributes = attributes->len;
  self->preedit_attributes = (ClutterPreeditAttribute *) g_array_free (attributes, FALSE);
  if (self->n_preedit_attributes == 0)
    g_clear_pointer (&self->preedit_attributes, g_free);
}

static void
show_preedit (GnoblinIbusInputMethod *self)
{
  clutter_input_method_set_preedit_text_with_attrs (CLUTTER_INPUT_METHOD (self),
                                                    self->preedit,
                                                    self->preedit_position,
                                                    self->preedit_anchor,
                                                    self->preedit_mode,
                                                    self->preedit_attributes,
                                                    self->n_preedit_attributes);
}

static void
hide_preedit (GnoblinIbusInputMethod *self)
{
  clutter_input_method_set_preedit_text (CLUTTER_INPUT_METHOD (self), NULL,
                                         self->preedit_position,
                                         self->preedit_anchor,
                                         self->preedit_mode);
}

static void
on_update_preedit_text (IBusInputContext       *context,
                        IBusText               *text,
                        guint                   position,
                        gboolean                visible,
                        guint                   mode,
                        GnoblinIbusInputMethod *self)
{
  const char *string;

  if (!text)
    return;

  string = ibus_text_get_text (text);
  g_clear_pointer (&self->preedit, g_free);
  if (string && *string)
    self->preedit = g_strdup (string);

  store_preedit_attributes (self, text);
  self->preedit_position = position;
  self->preedit_anchor = position;
  self->preedit_mode = mode == 0 ? CLUTTER_PREEDIT_RESET_CLEAR : CLUTTER_PREEDIT_RESET_COMMIT;

  if (visible)
    show_preedit (self);
  else if (self->preedit_visible)
    hide_preedit (self);

  self->preedit_visible = visible;
}

static void
on_show_preedit_text (IBusInputContext       *context,
                      GnoblinIbusInputMethod *self)
{
  self->preedit_visible = TRUE;
  show_preedit (self);
}

static void
on_hide_preedit_text (IBusInputContext       *context,
                      GnoblinIbusInputMethod *self)
{
  hide_preedit (self);
  self->preedit_visible = FALSE;
}

static void
on_forward_key_event (IBusInputContext       *context,
                      guint                   keyval,
                      guint                   keycode,
                      guint                   state,
                      GnoblinIbusInputMethod *self)
{
  gboolean press = (state & IBUS_RELEASE_MASK) == 0;
  guint32 time = clutter_get_current_event_time ();

  state &= ~IBUS_RELEASE_MASK;
  if (time == CLUTTER_CURRENT_TIME)
    time = (guint32) (g_get_monotonic_time () / 1000);

  /* IBus uses evdev key codes. Clutter and Wayland use XKB key codes. */
  clutter_input_method_forward_key (CLUTTER_INPUT_METHOD (self), keyval, keycode + 8,
                                    state & CLUTTER_MODIFIER_MASK, time, press);
}

static void
on_context_destroyed (IBusProxy              *proxy,
                      GnoblinIbusInputMethod *self)
{
  clear_context (self);
}

static void
on_context_created (GObject      *source,
                    GAsyncResult *result,
                    gpointer      user_data)
{
  GnoblinIbusInputMethod *self = user_data;
  g_autoptr (GError) error = NULL;
  IBusInputContext *context;

  context = ibus_bus_create_input_context_async_finish (IBUS_BUS (source), result, &error);
  if (!context)
    {
      if (!g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
        {
          g_debug ("Could not create an IBus input context: %s", error->message);
          clear_context (self);
        }
      g_object_unref (self);
      return;
    }

  self->context = context;
  ibus_input_context_set_client_commit_preedit (context, TRUE);
  ibus_input_context_set_preedit_format (context, IBUS_PREEDIT_FORMAT_HINT);
  g_signal_connect (context, "commit-text", G_CALLBACK (on_commit_text), self);
  g_signal_connect (context, "delete-surrounding-text",
                    G_CALLBACK (on_delete_surrounding_text), self);
  g_signal_connect (context, "update-preedit-text-with-mode",
                    G_CALLBACK (on_update_preedit_text), self);
  g_signal_connect (context, "show-preedit-text", G_CALLBACK (on_show_preedit_text), self);
  g_signal_connect (context, "hide-preedit-text", G_CALLBACK (on_hide_preedit_text), self);
  g_signal_connect (context, "forward-key-event", G_CALLBACK (on_forward_key_event), self);
  g_signal_connect (context, "require-surrounding-text",
                    G_CALLBACK (on_require_surrounding_text), self);
  g_signal_connect (context, "destroy", G_CALLBACK (on_context_destroyed), self);

  update_capabilities (self);
  g_object_unref (self);
}

static void
create_context (GnoblinIbusInputMethod *self)
{
  clear_context (self);
  self->cancellable = g_cancellable_new ();
  ibus_bus_create_input_context_async (self->bus, "gnoblin", -1, self->cancellable,
                                       on_context_created, g_object_ref (self));
}

static void
on_bus_connected (IBusBus                *bus,
                  GnoblinIbusInputMethod *self)
{
  create_context (self);
}

static void
on_bus_disconnected (IBusBus                *bus,
                     GnoblinIbusInputMethod *self)
{
  clear_context (self);
}

static void
gnoblin_ibus_input_method_focus_in (ClutterInputMethod *im,
                                    ClutterInputFocus  *focus)
{
  GnoblinIbusInputMethod *self = GNOBLIN_IBUS_INPUT_METHOD (im);

  if (!self->context)
    return;

  update_context (self);
  ibus_input_context_focus_in (self->context);
  request_surrounding_if_needed (self);
}

static void
gnoblin_ibus_input_method_focus_out (ClutterInputMethod *im)
{
  GnoblinIbusInputMethod *self = GNOBLIN_IBUS_INPUT_METHOD (im);

  if (self->context)
    {
      /* Forget the field's state, then drop focus. */
      ibus_input_context_set_content_type (self->context, 0, 0);
      ibus_input_context_set_cursor_location (self->context, 0, 0, 0, 0);
      ibus_input_context_reset (self->context);
      ibus_input_context_focus_out (self->context);
    }

  if (self->preedit && self->preedit_visible)
    {
      clutter_input_method_set_preedit_text (im, NULL, 0, 0, self->preedit_mode);
      g_clear_pointer (&self->preedit, g_free);
    }
}

static void
gnoblin_ibus_input_method_reset (ClutterInputMethod *im)
{
  GnoblinIbusInputMethod *self = GNOBLIN_IBUS_INPUT_METHOD (im);

  if (self->context)
    {
      ibus_input_context_reset (self->context);
      request_surrounding_if_needed (self);
    }

  g_clear_pointer (&self->surrounding_text, g_free);
  self->surrounding_cursor = 0;
  self->surrounding_anchor = 0;
  g_clear_pointer (&self->preedit, g_free);
}

static void
gnoblin_ibus_input_method_set_cursor_location (ClutterInputMethod    *im,
                                               const graphene_rect_t *rect)
{
  GnoblinIbusInputMethod *self = GNOBLIN_IBUS_INPUT_METHOD (im);

  self->cursor_rect = *rect;
  self->has_cursor_rect = TRUE;

  if (!self->context)
    return;

  ibus_input_context_set_cursor_location (self->context,
                                          (int) rect->origin.x, (int) rect->origin.y,
                                          (int) rect->size.width, (int) rect->size.height);
  request_surrounding_if_needed (self);
}

static void
gnoblin_ibus_input_method_set_surrounding (ClutterInputMethod *im,
                                           const char         *text,
                                           guint               cursor,
                                           guint               anchor)
{
  GnoblinIbusInputMethod *self = GNOBLIN_IBUS_INPUT_METHOD (im);
  gboolean had_surrounding = self->surrounding_text != NULL;
  gboolean has_surrounding = text != NULL;
  IBusText *ibus_text;

  g_free (self->surrounding_text);
  self->surrounding_text = g_strdup (text);
  self->surrounding_cursor = cursor;
  self->surrounding_anchor = anchor;

  if (!self->context || !has_surrounding)
    return;

  /* The capability must be set before the surrounding text. */
  if (had_surrounding != has_surrounding)
    update_capabilities (self);

  ibus_text = ibus_text_new_from_string (text);
  g_object_ref_sink (ibus_text);
  ibus_input_context_set_surrounding_text (self->context, ibus_text, cursor, anchor);
  g_object_unref (ibus_text);
}

static void
gnoblin_ibus_input_method_update_content_hints (ClutterInputMethod           *im,
                                                ClutterInputContentHintFlags  hints)
{
  GnoblinIbusInputMethod *self = GNOBLIN_IBUS_INPUT_METHOD (im);
  guint ibus_hints = 0;

  if (hints & CLUTTER_INPUT_CONTENT_HINT_COMPLETION)
    ibus_hints |= IBUS_INPUT_HINT_WORD_COMPLETION;
  if (hints & CLUTTER_INPUT_CONTENT_HINT_SPELLCHECK)
    ibus_hints |= IBUS_INPUT_HINT_SPELLCHECK;
  if (hints & CLUTTER_INPUT_CONTENT_HINT_AUTO_CAPITALIZATION)
    ibus_hints |= IBUS_INPUT_HINT_UPPERCASE_SENTENCES;
  if (hints & CLUTTER_INPUT_CONTENT_HINT_LOWERCASE)
    ibus_hints |= IBUS_INPUT_HINT_LOWERCASE;
  if (hints & CLUTTER_INPUT_CONTENT_HINT_UPPERCASE)
    ibus_hints |= IBUS_INPUT_HINT_UPPERCASE_CHARS;
  if (hints & CLUTTER_INPUT_CONTENT_HINT_TITLECASE)
    ibus_hints |= IBUS_INPUT_HINT_UPPERCASE_WORDS;
  /* IBus has one flag for both sensitive and hidden text. */
  if (hints & (CLUTTER_INPUT_CONTENT_HINT_SENSITIVE_DATA | CLUTTER_INPUT_CONTENT_HINT_HIDDEN_TEXT))
    ibus_hints |= IBUS_INPUT_HINT_PRIVATE;
  if (hints & CLUTTER_INPUT_CONTENT_HINT_NO_EMOJI)
    ibus_hints |= IBUS_INPUT_HINT_NO_EMOJI;

  self->ibus_hints = ibus_hints;
  if (self->context)
    ibus_input_context_set_content_type (self->context, self->ibus_purpose, self->ibus_hints);
}

static void
gnoblin_ibus_input_method_update_content_purpose (ClutterInputMethod         *im,
                                                  ClutterInputContentPurpose  purpose)
{
  GnoblinIbusInputMethod *self = GNOBLIN_IBUS_INPUT_METHOD (im);
  guint ibus_purpose = IBUS_INPUT_PURPOSE_FREE_FORM;

  switch (purpose)
    {
    case CLUTTER_INPUT_CONTENT_PURPOSE_ALPHA:
      ibus_purpose = IBUS_INPUT_PURPOSE_ALPHA;
      break;
    case CLUTTER_INPUT_CONTENT_PURPOSE_DIGITS:
      ibus_purpose = IBUS_INPUT_PURPOSE_DIGITS;
      break;
    case CLUTTER_INPUT_CONTENT_PURPOSE_NUMBER:
      ibus_purpose = IBUS_INPUT_PURPOSE_NUMBER;
      break;
    case CLUTTER_INPUT_CONTENT_PURPOSE_PHONE:
      ibus_purpose = IBUS_INPUT_PURPOSE_PHONE;
      break;
    case CLUTTER_INPUT_CONTENT_PURPOSE_URL:
      ibus_purpose = IBUS_INPUT_PURPOSE_URL;
      break;
    case CLUTTER_INPUT_CONTENT_PURPOSE_EMAIL:
      ibus_purpose = IBUS_INPUT_PURPOSE_EMAIL;
      break;
    case CLUTTER_INPUT_CONTENT_PURPOSE_NAME:
      ibus_purpose = IBUS_INPUT_PURPOSE_NAME;
      break;
    case CLUTTER_INPUT_CONTENT_PURPOSE_PASSWORD:
      ibus_purpose = IBUS_INPUT_PURPOSE_PASSWORD;
      break;
    case CLUTTER_INPUT_CONTENT_PURPOSE_TERMINAL:
      ibus_purpose = IBUS_INPUT_PURPOSE_TERMINAL;
      break;
    default:
      break;
    }

  self->ibus_purpose = ibus_purpose;
  if (self->context)
    ibus_input_context_set_content_type (self->context, self->ibus_purpose, self->ibus_hints);
}

typedef struct
{
  GnoblinIbusInputMethod *self;
  IBusInputContext *context;
  ClutterEvent *event;
} KeyRequest;

static void
key_request_free (KeyRequest *request)
{
  g_clear_object (&request->self);
  g_clear_object (&request->context);
  g_clear_pointer (&request->event, clutter_event_free);
  g_free (request);
}

static void
on_key_processed (GObject      *source,
                  GAsyncResult *result,
                  gpointer      user_data)
{
  KeyRequest *request = user_data;
  g_autoptr (GError) error = NULL;
  gboolean handled;

  handled = ibus_input_context_process_key_event_async_finish (IBUS_INPUT_CONTEXT (source),
                                                               result, &error);
  if (error)
    {
      /* A cancelled request belongs to a context that is gone. Pass the key through. */
      if (!g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED))
        g_debug ("IBus could not process a key: %s", error->message);
      handled = FALSE;
    }

  if (request->self->context == request->context || error)
    {
      clutter_input_method_notify_key_event (CLUTTER_INPUT_METHOD (request->self),
                                             request->event, handled);
    }

  key_request_free (request);
}

static gboolean
gnoblin_ibus_input_method_filter_key_event (ClutterInputMethod *im,
                                            const ClutterEvent *event)
{
  GnoblinIbusInputMethod *self = GNOBLIN_IBUS_INPUT_METHOD (im);
  KeyRequest *request;
  guint state;

  if (!self->context)
    return FALSE;

  state = clutter_event_get_state (event);
  if (state & IBUS_IGNORED_MASK)
    return FALSE;

  if (clutter_event_type (event) == CLUTTER_KEY_RELEASE)
    state |= IBUS_RELEASE_MASK;

  request = g_new0 (KeyRequest, 1);
  request->self = g_object_ref (self);
  request->context = g_object_ref (self->context);
  request->event = clutter_event_copy (event);

  /* IBus uses evdev key codes. Clutter uses XKB key codes. */
  ibus_input_context_process_key_event_async (self->context,
                                              clutter_event_get_key_symbol (event),
                                              clutter_event_get_key_code (event) - 8,
                                              state, -1, self->cancellable,
                                              on_key_processed, request);
  return TRUE;
}

static void
gnoblin_ibus_input_method_finalize (GObject *object)
{
  GnoblinIbusInputMethod *self = GNOBLIN_IBUS_INPUT_METHOD (object);

  if (self->bus)
    g_signal_handlers_disconnect_by_data (self->bus, self);
  clear_context (self);
  g_clear_object (&self->bus);
  g_clear_pointer (&self->surrounding_text, g_free);

  G_OBJECT_CLASS (gnoblin_ibus_input_method_parent_class)->finalize (object);
}

static void
gnoblin_ibus_input_method_class_init (GnoblinIbusInputMethodClass *klass)
{
  GObjectClass *object_class = G_OBJECT_CLASS (klass);
  ClutterInputMethodClass *im_class = CLUTTER_INPUT_METHOD_CLASS (klass);

  object_class->finalize = gnoblin_ibus_input_method_finalize;

  im_class->focus_in = gnoblin_ibus_input_method_focus_in;
  im_class->focus_out = gnoblin_ibus_input_method_focus_out;
  im_class->reset = gnoblin_ibus_input_method_reset;
  im_class->set_cursor_location = gnoblin_ibus_input_method_set_cursor_location;
  im_class->set_surrounding = gnoblin_ibus_input_method_set_surrounding;
  im_class->update_content_hints = gnoblin_ibus_input_method_update_content_hints;
  im_class->update_content_purpose = gnoblin_ibus_input_method_update_content_purpose;
  im_class->filter_key_event = gnoblin_ibus_input_method_filter_key_event;
}

static void
gnoblin_ibus_input_method_init (GnoblinIbusInputMethod *self)
{
  self->preedit_mode = CLUTTER_PREEDIT_RESET_CLEAR;

  /* ibus_bus_new_async does not start the daemon. It connects when one is running, and
   * again if the daemon restarts. */
  self->bus = ibus_bus_new_async ();
  g_signal_connect (self->bus, "connected", G_CALLBACK (on_bus_connected), self);
  g_signal_connect (self->bus, "disconnected", G_CALLBACK (on_bus_disconnected), self);
  if (ibus_bus_is_connected (self->bus))
    create_context (self);
}

void
gnoblin_ibus_input_method_install (MetaBackend *backend)
{
  g_autoptr (GnoblinIbusInputMethod) im = NULL;
  ClutterBackend *clutter_backend;

  g_return_if_fail (META_IS_BACKEND (backend));

  clutter_backend = meta_backend_get_clutter_backend (backend);
  if (clutter_backend_get_input_method (clutter_backend))
    return;

  ibus_init ();
  im = g_object_new (GNOBLIN_TYPE_IBUS_INPUT_METHOD, NULL);
  clutter_backend_set_input_method (clutter_backend, CLUTTER_INPUT_METHOD (im));
}
