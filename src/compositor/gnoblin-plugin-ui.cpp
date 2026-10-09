/* Coordinator for Gnoblin's compositor-owned diagnostic panels. */

#include "gnoblin-plugin-ui.h"

#include <cstring>
#include <cstdint>
#include <functional>
#include <memory>
#include <set>
#include <string>
#include <utility>
#include <vector>

#include <glib/gstdio.h>
#include <gio/gio.h>
#include <imgui.h>

#include "gnoblin-console-panel.hpp"
#include "gnoblin-imgui-overlay.hpp"
#include "gnoblin-recovery-panel.hpp"
#include "gnoblin-native-control.h"

struct RuntimeRequest {
  GnoblinPluginUi *ui;
  std::uint64_t id = 0;
  std::function<void (GnoblinConsoleResult)> done;
};

struct CompletionRequest {
  GnoblinPluginUi *ui;
  std::uint64_t id = 0;
  std::function<void (std::vector<GnoblinConsoleCompletion>)> done;
};

constexpr gsize kConsoleHistoryLimit = 200;
constexpr gsize kConsoleHistoryMaximumBytes = 8 * 1024 * 1024;
constexpr gsize kConsoleHistoryCommandMaximumBytes = 16 * 1024;

/* Console history is diagnostic state, not configuration. Keep its I/O off
 * compositor input handling: loading happens during UI construction and saves
 * use one coalescing asynchronous GIO write. */
struct ConsoleHistoryStore {
  std::string path;
  GCancellable *cancellable = nullptr;
  bool write_in_flight = false;
  std::string queued_contents;
  std::string active_contents;
  bool write_failure_reported = false;

  explicit ConsoleHistoryStore (std::string history_path)
    : path (std::move (history_path)), cancellable (g_cancellable_new ())
  {
  }

  ~ConsoleHistoryStore ()
  {
    g_clear_object (&cancellable);
  }
};

static void console_history_start_write (const std::shared_ptr<ConsoleHistoryStore> &store);

static void
console_history_write_finished (GObject      *source,
                                GAsyncResult *result,
                                gpointer      user_data)
{
  std::unique_ptr<std::shared_ptr<ConsoleHistoryStore>> keeper (
    static_cast<std::shared_ptr<ConsoleHistoryStore> *> (user_data));
  const auto &store = *keeper;
  g_autoptr(GError) error = nullptr;
  if (!g_file_replace_contents_finish (G_FILE (source), result, nullptr, &error) &&
      error && !g_error_matches (error, G_IO_ERROR, G_IO_ERROR_CANCELLED) &&
      !store->write_failure_reported)
    {
      g_warning ("Gnoblin console history could not be saved: %s", error->message);
      store->write_failure_reported = true;
    }
  store->active_contents.clear ();
  store->write_in_flight = false;
  if (!store->queued_contents.empty () && !g_cancellable_is_cancelled (store->cancellable))
    console_history_start_write (store);
}

static void
console_history_start_write (const std::shared_ptr<ConsoleHistoryStore> &store)
{
  if (store->write_in_flight || store->queued_contents.empty () ||
      g_cancellable_is_cancelled (store->cancellable))
    return;
  store->active_contents = std::move (store->queued_contents);
  store->queued_contents.clear ();
  store->write_in_flight = true;
  g_autoptr(GFile) file = g_file_new_for_path (store->path.c_str ());
  g_file_replace_contents_async (
    file, store->active_contents.c_str (), store->active_contents.size (), nullptr, FALSE,
    G_FILE_CREATE_PRIVATE,
    store->cancellable, console_history_write_finished,
    new std::shared_ptr<ConsoleHistoryStore> (store));
}

static std::vector<std::string>
console_history_load (const std::shared_ptr<ConsoleHistoryStore> &store)
{
  GStatBuf history_stat;
  if (g_stat (store->path.c_str (), &history_stat) != 0 || !S_ISREG (history_stat.st_mode) ||
      history_stat.st_size < 0 ||
      static_cast<guint64> (history_stat.st_size) > kConsoleHistoryMaximumBytes)
    return {};
  g_autoptr(GKeyFile) key_file = g_key_file_new ();
  g_autoptr(GError) error = nullptr;
  if (!g_key_file_load_from_file (key_file, store->path.c_str (), G_KEY_FILE_NONE, &error))
    return {};
  gsize count = 0;
  g_auto(GStrv) entries = g_key_file_get_string_list (key_file, "Console", "History", &count, nullptr);
  std::vector<std::string> history;
  const gsize first = count > kConsoleHistoryLimit ? count - kConsoleHistoryLimit : 0;
  for (gsize i = first; entries && i < count; i++)
    if (std::strlen (entries[i]) <= kConsoleHistoryCommandMaximumBytes &&
        g_utf8_validate (entries[i], -1, nullptr))
      history.emplace_back (entries[i]);
  return history;
}

static void
console_history_save (const std::shared_ptr<ConsoleHistoryStore> &store,
                      const std::vector<std::string>              &history)
{
  g_autoptr(GKeyFile) key_file = g_key_file_new ();
  std::vector<const gchar *> entries;
  const std::size_t first = history.size () > kConsoleHistoryLimit
    ? history.size () - kConsoleHistoryLimit : 0;
  entries.reserve (history.size () - first);
  for (std::size_t i = first; i < history.size (); i++)
    if (history[i].size () <= kConsoleHistoryCommandMaximumBytes)
      entries.push_back (history[i].c_str ());
  g_key_file_set_string_list (key_file, "Console", "History", entries.data (), entries.size ());
  gsize length = 0;
  g_autofree gchar *contents = g_key_file_to_data (key_file, &length, nullptr);
  if (!contents)
    return;
  store->queued_contents.assign (contents, length);
  console_history_start_write (store);
}

static std::shared_ptr<ConsoleHistoryStore>
console_history_store_new ()
{
  g_autofree char *directory = g_build_filename (g_get_user_state_dir (), "gnoblin", nullptr);
  /* This runs while the diagnostics UI is created, before it can receive input. */
  if (g_mkdir_with_parents (directory, 0700) != 0)
    return nullptr;
  g_autofree char *path = g_build_filename (directory, "console-history.ini", nullptr);
  return std::make_shared<ConsoleHistoryStore> (path);
}

struct _GnoblinPluginUi {
  GnoblinImGuiOverlay *overlay;
  MetaDisplay *display;
  guint recovery_poll_id;
  guint recovery_buttons = 0;
  bool session_locked = false;
  std::string terminal;
  std::shared_ptr<ConsoleHistoryStore> console_history;
  std::set<std::uint64_t> requests;
  std::unique_ptr<GnoblinRecoveryPanel, decltype (&gnoblin_recovery_panel_free)> recovery;
  std::unique_ptr<GnoblinConsolePanel> console;

  _GnoblinPluginUi ()
    : overlay (nullptr), display (nullptr), recovery_poll_id (0),
      recovery (nullptr, gnoblin_recovery_panel_free)
  {
  }
};

static void
update_visibility (GnoblinPluginUi *ui)
{
  const gboolean visible = !ui->session_locked &&
    (gnoblin_recovery_panel_should_draw (ui->recovery.get ()) || ui->console->visible ());
  gnoblin_imgui_overlay_set_visible (ui->overlay, visible);
  if (visible)
    gnoblin_imgui_overlay_queue_redraw (ui->overlay);
}

static void
draw_panels (GnoblinImGuiOverlay *overlay,
             void                *user_data)
{
  auto *ui = static_cast<GnoblinPluginUi *> (user_data);
  gnoblin_recovery_panel_draw (overlay, ui->recovery.get ());
  if (ui->console->visible ())
    ui->console->draw_frame (overlay);
}

static void
console_visibility_changed (GnoblinPluginUi *ui)
{
  update_visibility (ui);
}

/* The runtime writes this marker after falling back, before the compositor
 * launches. It is advisory presentation state only: config loading remains in
 * the supervisor and no untrusted file is evaluated here. */
static gboolean
load_recovery_marker (GnoblinPluginUi *ui)
{
  g_autofree char *path =
    g_build_filename (g_get_user_runtime_dir (), "gnoblin", "config-fallback", nullptr);
  g_autofree char *contents = nullptr;
  gsize length = 0;
  GnoblinRecoveryFallbackSource fallback = GnoblinRecoveryFallbackSource::None;

  const char *error_message = nullptr;
  if (g_file_get_contents (path, &contents, &length, nullptr))
    {
      if (g_str_has_prefix (contents, "last-good"))
        fallback = GnoblinRecoveryFallbackSource::LastKnownGood;
      else if (g_str_has_prefix (contents, "defaults"))
        fallback = GnoblinRecoveryFallbackSource::BuiltIn;

      /* The first line is the stable marker. A later supervisor supplies its
       * human-readable load failure after it, without changing that contract. */
      char *newline = std::strchr (contents, '\n');
      if (newline != nullptr && newline[1] != '\0')
        {
          error_message = newline + 1;
          char *trailing = std::strrchr (newline + 1, '\n');
          if (trailing != nullptr)
            *trailing = '\0';
        }
    }

  /* A configuration fallback explains the broken configuration and stays the
   * visible diagnostic. Input policy only replaces the notice when the
   * supervisor did not report a configuration failure. */
  if (fallback == GnoblinRecoveryFallbackSource::None &&
      (error_message == nullptr || *error_message == '\0'))
    error_message = gnoblin_native_control_input_policy_error (ui->display);

  return gnoblin_recovery_panel_set_state (ui->recovery.get (), error_message, fallback, false);
}

static gboolean
refresh_recovery_marker (gpointer user_data)
{
  auto *ui = static_cast<GnoblinPluginUi *> (user_data);
  load_recovery_marker (ui);
  update_visibility (ui);
  return G_SOURCE_CONTINUE;
}

static void
runtime_request_destroy (gpointer data)
{
  delete static_cast<RuntimeRequest *> (data);
}

constexpr guint kConsoleInspectionMaximumDepth = 8;
constexpr guint kConsoleInspectionMaximumNodes = 256;
constexpr guint kConsoleInspectionMaximumChildren = 64;
constexpr gsize kConsoleInspectionMaximumTextBytes = 4096;

static bool
console_inspection_string (GVariant *dictionary,
                           const char *key,
                           std::string *destination)
{
  const char *value = nullptr;
  if (!g_variant_lookup (dictionary, key, "&s", &value) || !value ||
      std::strlen (value) > kConsoleInspectionMaximumTextBytes ||
      !g_utf8_validate (value, -1, nullptr))
    return false;
  *destination = value;
  return true;
}

static bool
console_inspection_node (GVariant                    *dictionary,
                         guint                        depth,
                         guint                       *node_count,
                         GnoblinConsoleInspectorNode *destination)
{
  if (!g_variant_is_of_type (dictionary, G_VARIANT_TYPE_VARDICT) ||
      depth > kConsoleInspectionMaximumDepth ||
      *node_count >= kConsoleInspectionMaximumNodes ||
      !console_inspection_string (dictionary, "label", &destination->label) ||
      !console_inspection_string (dictionary, "value", &destination->value))
    return false;
  (*node_count)++;
  g_variant_lookup (dictionary, "has_more", "b", &destination->has_more);

  g_autoptr(GVariant) children =
    g_variant_lookup_value (dictionary, "children", G_VARIANT_TYPE ("aa{sv}"));
  if (!children)
    return true;
  GVariantIter iterator;
  g_variant_iter_init (&iterator, children);
  while (destination->children.size () < kConsoleInspectionMaximumChildren &&
         *node_count < kConsoleInspectionMaximumNodes)
    {
      g_autoptr(GVariant) child = g_variant_iter_next_value (&iterator);
      if (!child)
        break;
      GnoblinConsoleInspectorNode decoded;
      if (console_inspection_node (child, depth + 1, node_count, &decoded))
        destination->children.push_back (std::move (decoded));
      else
        destination->has_more = true;
    }
  g_autoptr(GVariant) remaining = g_variant_iter_next_value (&iterator);
  if (remaining)
    destination->has_more = true;
  return true;
}

static std::vector<GnoblinConsoleInspectorNode>
console_inspection_nodes (GVariant *result)
{
  std::vector<GnoblinConsoleInspectorNode> nodes;
  g_autoptr(GVariant) inspection =
    g_variant_lookup_value (result, "inspection", G_VARIANT_TYPE ("aa{sv}"));
  if (!inspection)
    return nodes;
  guint node_count = 0;
  GVariantIter iterator;
  g_variant_iter_init (&iterator, inspection);
  while (nodes.size () < kConsoleInspectionMaximumChildren &&
         node_count < kConsoleInspectionMaximumNodes)
    {
      g_autoptr(GVariant) node = g_variant_iter_next_value (&iterator);
      if (!node)
        break;
      GnoblinConsoleInspectorNode decoded;
      if (console_inspection_node (node, 0, &node_count, &decoded))
        nodes.push_back (std::move (decoded));
    }
  return nodes;
}

static GnoblinConsoleResult
runtime_result (gboolean success, GVariant *result, const char *error)
{
  GnoblinConsoleResult converted;
  converted.success = success;
  if (!success)
    converted.error = error ? error : "runtime request failed";
  else if (result != nullptr)
    {
      const char *value = nullptr;
      if (g_variant_lookup (result, "value", "&s", &value) && value)
        converted.value = value;
      else
        converted.value = "nil";
      const char *type = nullptr;
      if (g_variant_lookup (result, "type", "&s", &type) && type)
        converted.value += "\n(type: " + std::string (type) + ")";
      gint64 operations = 0;
      if (g_variant_lookup (result, "operations", "x", &operations) && operations > 0)
        converted.value += "\n" + std::to_string (operations) + " runtime operation(s) queued";
      converted.inspection = console_inspection_nodes (result);
    }
  return converted;
}

static std::vector<GnoblinConsoleCompletion>
console_completions (GVariant *result)
{
  constexpr guint maximum_completions = 64;
  std::vector<GnoblinConsoleCompletion> completions;
  g_autoptr(GVariant) entries =
    g_variant_lookup_value (result, "completions", G_VARIANT_TYPE ("aa{sv}"));
  if (!entries)
    return completions;
  GVariantIter iterator;
  g_variant_iter_init (&iterator, entries);
  while (completions.size () < maximum_completions)
    {
      g_autoptr(GVariant) entry = g_variant_iter_next_value (&iterator);
      if (!entry)
        break;
      GnoblinConsoleCompletion completion;
      if (!g_variant_is_of_type (entry, G_VARIANT_TYPE_VARDICT) ||
          !console_inspection_string (entry, "label", &completion.label) ||
          !console_inspection_string (entry, "detail", &completion.detail) ||
          !console_inspection_string (entry, "documentation", &completion.documentation))
        continue;
      completions.push_back (std::move (completion));
    }
  return completions;
}

static void
completion_request_destroy (gpointer data)
{
  delete static_cast<CompletionRequest *> (data);
}

static void
completion_response (gboolean success, GVariant *result, const char *error, gpointer user_data)
{
  auto *request = static_cast<CompletionRequest *> (user_data);
  request->ui->requests.erase (request->id);
  if (!success)
    {
      request->ui->console->append_log (
        std::string ("Lua completion: ") + (error ? error : "runtime request failed"), true);
      request->done ({ });
      return;
    }
  request->done (result ? console_completions (result) : std::vector<GnoblinConsoleCompletion> { });
}

static void
complete_lua (GnoblinPluginUi *ui, std::string_view source,
              std::function<void (std::vector<GnoblinConsoleCompletion>)> done)
{
  auto *request = new CompletionRequest { ui, 0, std::move (done) };
  const std::string code (source);
  g_autoptr(GError) error = nullptr;
  request->id = gnoblin_native_control_console_complete (
    ui->display, code.c_str (), completion_response, request, completion_request_destroy, &error);
  if (request->id != 0)
    {
      ui->requests.insert (request->id);
      return;
    }
  ui->console->append_log (
    std::string ("Lua completion: ") +
      (error ? error->message : "could not queue completion request"), true);
  request->done ({ });
  delete request;
}

static void
runtime_response (gboolean success, GVariant *result, const char *error, gpointer user_data)
{
  auto *request = static_cast<RuntimeRequest *> (user_data);
  request->ui->requests.erase (request->id);
  if (request->done)
    request->done (runtime_result (success, result, error));
}

static void
evaluate_lua (GnoblinPluginUi *ui, std::string_view source,
              std::function<void (GnoblinConsoleResult)> done)
{
  auto *request = new RuntimeRequest { ui, 0, std::move (done) };
  const std::string code (source);
  g_autoptr(GError) error = nullptr;
  request->id = gnoblin_native_control_console_eval (ui->display, code.c_str (),
                                                      runtime_response, request,
                                                      runtime_request_destroy, &error);
  if (request->id != 0)
    {
      ui->requests.insert (request->id);
      return;
    }
  GnoblinConsoleResult failed;
  failed.success = false;
  failed.error = error ? error->message : "could not queue Lua evaluation";
  request->done (std::move (failed));
  delete request;
}

static void
control_lua (GnoblinPluginUi *ui, GnoblinConsoleControl control,
             std::function<void (GnoblinConsoleResult)> done)
{
  if (control != GnoblinConsoleControl::ReloadConfiguration)
    {
      GnoblinConsoleResult failed;
      failed.success = false;
      failed.error = "unsupported console control";
      done (std::move (failed));
      return;
    }
  auto *request = new RuntimeRequest { ui, 0, std::move (done) };
  g_autoptr(GError) error = nullptr;
  request->id = gnoblin_native_control_reload_runtime_config (
    ui->display, runtime_response, request, runtime_request_destroy, &error);
  if (request->id != 0)
    {
      ui->requests.insert (request->id);
      return;
    }
  GnoblinConsoleResult failed;
  failed.success = false;
  failed.error = error ? error->message : "could not queue configuration reload";
  request->done (std::move (failed));
  delete request;
}

static void
open_config_folder (void *user_data)
{
  (void) user_data;
  const char *configured = g_getenv ("GNOBLIN_CONFIG");
  g_autofree char *path = configured ? g_path_get_dirname (configured) :
    g_build_filename (g_get_user_config_dir (), "gnoblin", nullptr);
  g_autofree char *uri = g_filename_to_uri (path, nullptr, nullptr);
  if (uri)
    g_app_info_launch_default_for_uri_async (uri, nullptr, nullptr, nullptr, nullptr);
}

static void
open_terminal (void *user_data)
{
  auto *ui = static_cast<GnoblinPluginUi *> (user_data);
  const char *argv[] = { ui->terminal.c_str (), nullptr };
  g_autoptr(GError) error = nullptr;
  if (!g_spawn_async (nullptr, const_cast<char **> (argv), nullptr,
                      G_SPAWN_DEFAULT, nullptr, nullptr, nullptr, &error))
    ui->console->append_log (error ? error->message : "Could not open terminal", true);
}

static void
reload_config (void *user_data)
{
  auto *ui = static_cast<GnoblinPluginUi *> (user_data);
  control_lua (ui, GnoblinConsoleControl::ReloadConfiguration,
               [ui] (GnoblinConsoleResult result) {
                 if (!result.success)
                   ui->console->append_log (result.error, true);
                 if (load_recovery_marker (ui))
                   update_visibility (ui);
               });
}

GnoblinPluginUi *
gnoblin_plugin_ui_new (ClutterActor *parent, MetaDisplay *display)
{
  auto *ui = new GnoblinPluginUi;
  ui->display = display;
  ui->overlay = gnoblin_imgui_overlay_new (parent);
  /* Stage capture routes the modal console and the recovery panel's pointer
   * bounds. The full-stage actor must not intercept other client input. */
  clutter_actor_set_reactive (gnoblin_imgui_overlay_get_actor (ui->overlay), FALSE);
  ui->recovery = { gnoblin_recovery_panel_new (), gnoblin_recovery_panel_free };
  ui->console_history = console_history_store_new ();

  GnoblinConsoleCallbacks callbacks;
  callbacks.evaluate = [ui] (std::string_view source,
                             std::function<void (GnoblinConsoleResult)> done) {
    evaluate_lua (ui, source, std::move (done));
  };
  callbacks.control = [ui] (GnoblinConsoleControl control,
                            std::function<void (GnoblinConsoleResult)> done) {
    control_lua (ui, control, std::move (done));
  };
  callbacks.complete = [ui] (std::string_view source,
                             std::function<void (std::vector<GnoblinConsoleCompletion>)> done) {
    complete_lua (ui, source, std::move (done));
  };
  if (ui->console_history)
    {
      const auto history = ui->console_history;
      callbacks.load_history = [history] { return console_history_load (history); };
      callbacks.save_history = [history] (const std::vector<std::string> &entries) {
        console_history_save (history, entries);
      };
    }
  callbacks.visibility_changed = [ui] { console_visibility_changed (ui); };
  ui->console = std::make_unique<GnoblinConsolePanel> (ui->overlay, std::move (callbacks));
  const char *terminals[] = { "xdg-terminal-exec", "ptyxis", "kgx", "konsole", "foot",
                              "kitty", "alacritty", "gnome-terminal", "xterm", nullptr };
  for (guint i = 0; terminals[i]; i++) {
    g_autofree char *program = g_find_program_in_path (terminals[i]);
    if (program) {
      ui->terminal = program;
      break;
    }
  }
  GnoblinRecoveryPanelActions actions = {
    open_config_folder, ui->terminal.empty () ? nullptr : open_terminal, reload_config, ui
  };
  gnoblin_recovery_panel_set_actions (ui->recovery.get (), &actions);
  gnoblin_imgui_overlay_set_draw_callback (ui->overlay, draw_panels, ui, nullptr);
  load_recovery_marker (ui);
  /* The supervisor publishes startup recovery state after the compositor's
   * first runtime handshake, which happens after this plugin is constructed. */
  ui->recovery_poll_id = g_timeout_add_seconds (1, refresh_recovery_marker, ui);
  update_visibility (ui);
  return ui;
}

void
gnoblin_plugin_ui_free (GnoblinPluginUi *ui)
{
  if (ui == nullptr)
    return;
  if (ui->recovery_poll_id)
    g_source_remove (ui->recovery_poll_id);
  for (const std::uint64_t id : ui->requests)
    gnoblin_native_control_cancel_runtime_request (ui->display, id);
  ui->requests.clear ();
  ui->console.reset ();
  if (ui->console_history)
    g_cancellable_cancel (ui->console_history->cancellable);
  ui->console_history.reset ();
  ui->recovery.reset ();
  gnoblin_imgui_overlay_free (ui->overlay);
  delete ui;
}

void
gnoblin_plugin_ui_toggle_console (GnoblinPluginUi *ui)
{
  if (ui == nullptr || ui->session_locked)
    return;
  ui->console->toggle ();
  update_visibility (ui);
}

void
gnoblin_plugin_ui_hide_console (GnoblinPluginUi *ui)
{
  if (ui == nullptr || !ui->console->visible ())
    return;
  ui->console->set_visible (false);
  update_visibility (ui);
}

void
gnoblin_plugin_ui_set_session_locked (GnoblinPluginUi *ui, gboolean locked)
{
  if (ui == nullptr || ui->session_locked == static_cast<bool> (locked))
    return;
  ui->session_locked = locked;
  ui->recovery_buttons = 0;
  if (locked)
    ui->console->set_visible (false);
  ImGui::SetCurrentContext (gnoblin_imgui_overlay_get_context (ui->overlay));
  ImGui::GetIO ().ClearEventsQueue ();
  ImGui::GetIO ().ClearInputKeys ();
  ImGui::GetIO ().ClearInputMouse ();
  gnoblin_imgui_overlay_set_pointer (ui->overlay, -1.0f, -1.0f, FALSE, FALSE, FALSE);
  update_visibility (ui);
}

gboolean
gnoblin_plugin_ui_console_visible (GnoblinPluginUi *ui)
{
  return ui != nullptr && ui->console->visible ();
}

gboolean
gnoblin_plugin_ui_handle_event (GnoblinPluginUi *ui,
                                ClutterEvent    *event)
{
  if (ui == nullptr || ui->session_locked)
    return FALSE;
  const bool modal = ui->console->visible ();
  const ClutterEventType type = clutter_event_type (event);
  if (!modal) {
    if (type != CLUTTER_MOTION && type != CLUTTER_BUTTON_PRESS &&
        type != CLUTTER_BUTTON_RELEASE)
      return FALSE;
    float x = 0.0f, y = 0.0f;
    clutter_event_get_coords (event, &x, &y);
    const bool inside = gnoblin_recovery_panel_contains (ui->recovery.get (), x, y);
    if (type == CLUTTER_BUTTON_PRESS) {
      const guint button = clutter_event_get_button (event);
      if (!inside || button > 3 || button == 0)
        return FALSE;
      ui->recovery_buttons |= 1u << button;
    } else if (type == CLUTTER_BUTTON_RELEASE) {
      const guint button = clutter_event_get_button (event);
      if (button > 3 || button == 0 || !(ui->recovery_buttons & (1u << button)))
        return FALSE;
      ui->recovery_buttons &= ~(1u << button);
    } else if (!inside && !ui->recovery_buttons) {
      gnoblin_imgui_overlay_set_pointer (ui->overlay, -1.0f, -1.0f, FALSE, FALSE, FALSE);
      return FALSE;
    }
  }

  switch (type)
    {
    case CLUTTER_MOTION:
      {
        float x = 0.0f, y = 0.0f;
        clutter_event_get_coords (event, &x, &y);
        if (modal)
          gnoblin_imgui_overlay_set_pointer (ui->overlay, x, y,
            clutter_event_get_state (event) & CLUTTER_BUTTON1_MASK,
            clutter_event_get_state (event) & CLUTTER_BUTTON3_MASK,
            clutter_event_get_state (event) & CLUTTER_BUTTON2_MASK);
        else
          gnoblin_imgui_overlay_set_pointer (ui->overlay, x, y,
            ui->recovery_buttons & (1u << 1), ui->recovery_buttons & (1u << 3),
            ui->recovery_buttons & (1u << 2));
        return modal || ui->recovery_buttons != 0;
      }
    case CLUTTER_BUTTON_PRESS:
    case CLUTTER_BUTTON_RELEASE:
      {
        float x = 0.0f, y = 0.0f;
        ClutterModifierType state = clutter_event_get_state (event);
        if (!modal)
          state = static_cast<ClutterModifierType> (0);
        const guint button = clutter_event_get_button (event);
        const gboolean pressed = clutter_event_type (event) == CLUTTER_BUTTON_PRESS;
        clutter_event_get_coords (event, &x, &y);
        if (modal)
          gnoblin_imgui_overlay_set_pointer (
          ui->overlay, x, y,
          ((state & CLUTTER_BUTTON1_MASK) && !(button == 1 && !pressed)) ||
            (button == 1 && pressed),
          ((state & CLUTTER_BUTTON3_MASK) && !(button == 3 && !pressed)) ||
            (button == 3 && pressed),
          ((state & CLUTTER_BUTTON2_MASK) && !(button == 2 && !pressed)) ||
            (button == 2 && pressed));
        if (!modal)
          gnoblin_imgui_overlay_set_pointer (ui->overlay, x, y,
            ui->recovery_buttons & (1u << 1), ui->recovery_buttons & (1u << 3),
            ui->recovery_buttons & (1u << 2));
        return TRUE;
      }
    case CLUTTER_KEY_PRESS:
    case CLUTTER_KEY_RELEASE:
      {
        const gboolean pressed = clutter_event_type (event) == CLUTTER_KEY_PRESS;
        const gboolean consumed = ui->console->handle_key (
          clutter_event_get_key_symbol (event), clutter_event_get_state (event), pressed);
        if (pressed)
          {
            const gunichar character = clutter_event_get_key_unicode (event);
            if (character != 0 && !g_unichar_iscntrl (character))
              {
                char text[7] = {};
                const int length = g_unichar_to_utf8 (character, text);
                text[length] = '\0';
                ui->console->add_text (text);
              }
          }
        return consumed || pressed;
      }
    default:
      return TRUE;
    }
}
