/* Compositor-owned ImGui developer console; see gnoblin-console-panel.hpp. */

#include "config.h"

#include "gnoblin-console-panel.hpp"

#include <algorithm>
#include <cstdio>
#include <cstring>
#include <utility>

#include "imgui.h"

namespace {
constexpr std::size_t kMaxRows = 200;
constexpr std::size_t kInputCapacity = 16 * 1024;

std::string console_help ()
{
  return "Lua runtime console\n"
         "Runs a bounded Lua expression in the active configuration runtime.\n"
         "The configuration document is read-only here. Edit init.lua, then use :reload.\n"
         "Queued runtime operations are submitted through Gnoblin's normal API.\n\n"
         ":help  :reload";
}

ImGuiKey imgui_key_for_clutter (guint keyval)
{
  switch (keyval)
    {
    case CLUTTER_KEY_BackSpace: return ImGuiKey_Backspace;
    case CLUTTER_KEY_Delete: return ImGuiKey_Delete;
    case CLUTTER_KEY_Left: return ImGuiKey_LeftArrow;
    case CLUTTER_KEY_Right: return ImGuiKey_RightArrow;
    case CLUTTER_KEY_Home: return ImGuiKey_Home;
    case CLUTTER_KEY_End: return ImGuiKey_End;
    case CLUTTER_KEY_Page_Up: return ImGuiKey_PageUp;
    case CLUTTER_KEY_Page_Down: return ImGuiKey_PageDown;
    default: return ImGuiKey_None;
    }
}
} // namespace

struct GnoblinConsolePanel::Row {
  std::string source;
  std::string text;
  std::string error;
  std::vector<GnoblinConsoleInspectorNode> inspection;
  bool pending = false;
  bool log = false;
};

GnoblinConsolePanel::GnoblinConsolePanel (GnoblinImGuiOverlay *overlay,
                                          GnoblinConsoleCallbacks callbacks)
  : overlay_ (overlay), callbacks_ (std::move (callbacks)), input_ (kInputCapacity, '\0')
{
  if (callbacks_.load_history)
    history_ = callbacks_.load_history ();
  if (history_.size () > kMaxRows)
    history_.erase (history_.begin (), history_.end () - kMaxRows);
  history_index_ = history_.size ();
}

GnoblinConsolePanel::~GnoblinConsolePanel ()
{
  completion_generation_.reset ();
}

void
GnoblinConsolePanel::set_visible (bool visible)
{
  if (visible_ == visible)
    return;
  visible_ = visible;
  if (!visible)
    {
      ++*completion_generation_;
      completion_visible_ = false;
    }
  if (visible)
    scroll_to_bottom_ = true;
  if (callbacks_.visibility_changed)
    callbacks_.visibility_changed ();
  queue_redraw ();
}

bool
GnoblinConsolePanel::visible () const
{
  return visible_;
}

void
GnoblinConsolePanel::toggle ()
{
  set_visible (!visible ());
}

void
GnoblinConsolePanel::clear ()
{
  rows_.clear ();
  scroll_to_bottom_ = true;
  queue_redraw ();
}

void
GnoblinConsolePanel::draw_frame (GnoblinImGuiOverlay *overlay)
{
  if (overlay == overlay_)
    draw ();
}

void
GnoblinConsolePanel::add_text (const char *utf8)
{
  if (visible ())
    gnoblin_imgui_overlay_add_text (overlay_, utf8);
}

bool
GnoblinConsolePanel::handle_key (guint keyval, ClutterModifierType modifiers, bool pressed)
{
  if (!visible ())
    return false;

  ImGui::SetCurrentContext (gnoblin_imgui_overlay_get_context (overlay_));
  ImGuiIO &io = ImGui::GetIO ();
  io.AddKeyEvent (ImGuiMod_Ctrl, (modifiers & CLUTTER_CONTROL_MASK) != 0);
  io.AddKeyEvent (ImGuiMod_Shift, (modifiers & CLUTTER_SHIFT_MASK) != 0);
  io.AddKeyEvent (ImGuiMod_Alt, (modifiers & CLUTTER_MOD1_MASK) != 0);
  const ImGuiKey imgui_key = imgui_key_for_clutter (keyval);
  if (!pressed)
    {
      if (imgui_key != ImGuiKey_None)
        io.AddKeyEvent (imgui_key, false);
      queue_redraw ();
      return true;
    }

  if (keyval == CLUTTER_KEY_Escape)
    {
      if (completion_visible_)
        completion_visible_ = false;
      else
        set_visible (false);
      queue_redraw ();
      return true;
    }
  if (keyval == CLUTTER_KEY_Return || keyval == CLUTTER_KEY_KP_Enter)
    {
      if ((modifiers & CLUTTER_SHIFT_MASK) != 0)
        {
          /* Enter is intercepted for submission, so pass Shift+Enter as text
           * instead of relying on InputTextMultiline observing a key event. */
          gnoblin_imgui_overlay_add_text (overlay_, "\n");
          queue_redraw ();
          return true;
        }
      if (completion_visible_)
        accept_completion ();
      else
        submit ();
      return true;
    }

  if (imgui_key != ImGuiKey_None)
    io.AddKeyEvent (imgui_key, true);
  if (keyval == CLUTTER_KEY_Tab)
    {
      if (completion_visible_)
        accept_completion ();
      else
        complete ();
      return true;
    }
  if (keyval == CLUTTER_KEY_Up || keyval == CLUTTER_KEY_Down)
    {
      if (completion_visible_)
        {
          const int count = static_cast<int> (completions_.size ());
          if (count > 0)
            completion_index_ = (completion_index_ + (keyval == CLUTTER_KEY_Up ? count - 1 : 1)) % count;
        }
      else if (std::strchr (input_.data (), '\n') == nullptr || (modifiers & CLUTTER_MOD1_MASK) != 0)
        history_step (keyval == CLUTTER_KEY_Up ? -1 : 1);
      queue_redraw ();
      return true;
    }
  if ((modifiers & CLUTTER_CONTROL_MASK) != 0 && (keyval == CLUTTER_KEY_l || keyval == CLUTTER_KEY_L))
    {
      clear ();
      return true;
    }
  queue_redraw ();
  return true;
}

void
GnoblinConsolePanel::append_log (std::string_view text, bool error)
{
  Row row;
  row.text = text;
  row.log = true;
  if (error)
    row.error = row.text;
  rows_.push_back (std::move (row));
  if (rows_.size () > kMaxRows)
    rows_.erase (rows_.begin ());
  scroll_to_bottom_ = true;
  queue_redraw ();
}

void
GnoblinConsolePanel::draw ()
{
  if (!visible ())
    return;
  const ImGuiViewport *viewport = ImGui::GetMainViewport ();
  ImGui::SetNextWindowPos (viewport->WorkPos);
  ImGui::SetNextWindowSize (ImVec2 (viewport->WorkSize.x, viewport->WorkSize.y * 0.55f));
  constexpr ImGuiWindowFlags flags = ImGuiWindowFlags_NoCollapse | ImGuiWindowFlags_NoMove |
                                     ImGuiWindowFlags_NoResize | ImGuiWindowFlags_NoSavedSettings;
  if (!ImGui::Begin ("Lua console", nullptr, flags))
    {
      ImGui::End ();
      return;
    }

  ImGui::TextUnformatted ("Lua");
  ImGui::SameLine ();
  if (ImGui::Button ("Clear")) clear ();
  ImGui::Separator ();

  const float input_height = ImGui::GetTextLineHeightWithSpacing () * 6.0f + ImGui::GetStyle ().ItemSpacing.y * 2.0f;
  if (ImGui::BeginChild ("##transcript", ImVec2 (0, -input_height), ImGuiChildFlags_Borders))
    {
      for (std::size_t index = 0; index < rows_.size (); index++)
        {
          const Row &row = rows_[index];
          ImGui::PushID (static_cast<int> (index));
          if (row.log)
            {
              if (!row.error.empty ()) ImGui::TextColored (ImVec4 (1, .35f, .35f, 1), "%s", row.error.c_str ());
              else ImGui::TextWrapped ("%s", row.text.c_str ());
            }
          else
            {
              ImGui::TextColored (ImVec4 (.55f, .8f, 1, 1), "> %s", row.source.c_str ());
              if (row.pending) ImGui::TextDisabled ("Pending...");
              else if (!row.error.empty ()) ImGui::TextColored (ImVec4 (1, .35f, .35f, 1), "%s", row.error.c_str ());
              else if (!row.text.empty ()) ImGui::TextWrapped ("%s", row.text.c_str ());
              for (std::size_t child = 0; child < row.inspection.size (); child++)
                draw_node (row.inspection[child], std::to_string (index) + ":" + std::to_string (child));
            }
          ImGui::PopID ();
        }
      if (scroll_to_bottom_)
        {
          ImGui::SetScrollHereY (1.0f);
          scroll_to_bottom_ = false;
        }
    }
  ImGui::EndChild ();

  ImGui::TextUnformatted ("Input (Enter runs, Shift+Enter adds a line)");
  const ImVec2 input_size (ImGui::GetContentRegionAvail ().x, ImGui::GetTextLineHeightWithSpacing () * 4.0f);
  ImGui::InputTextMultiline ("##input", input_.data (), input_.size (), input_size,
                             ImGuiInputTextFlags_WordWrap);
  if (ImGui::Button (pending_ ? "Running..." : "Run") && !pending_)
    submit ();
  ImGui::SameLine ();
  if (ImGui::Button ("Complete")) complete ();
  if (completion_visible_ && completion_source_ != input_.data ())
    completion_visible_ = false;
  if (completion_visible_)
    {
      ImGui::SameLine ();
      ImGui::TextDisabled ("Tab/Enter accepts completion");
      for (std::size_t i = 0; i < completions_.size (); i++)
        {
          const auto &item = completions_[i];
          const bool selected = static_cast<int> (i) == completion_index_;
          if (ImGui::Selectable (item.label.c_str (), selected))
            {
              completion_index_ = static_cast<int> (i);
              accept_completion ();
            }
          if (selected && !item.documentation.empty ())
            ImGui::SetItemTooltip ("%s\n%s", item.detail.c_str (), item.documentation.c_str ());
        }
    }
  ImGui::End ();
}

void
GnoblinConsolePanel::submit ()
{
  if (pending_)
    return;
  std::string source (input_.data ());
  const auto first = source.find_first_not_of (" \t\n\r");
  if (first == std::string::npos)
    return;
  source.erase (0, first);
  const auto last = source.find_last_not_of (" \t\n\r");
  source.erase (last + 1);
  append_history (source);
  std::fill (input_.begin (), input_.end (), '\0');
  completion_visible_ = false;
  if (source == ":help")
    {
      append_result (source, { true, console_help (), {}, {} });
      return;
    }
  if (source == ":reload") { submit_control (GnoblinConsoleControl::ReloadConfiguration, source); return; }
  if (!callbacks_.evaluate)
    {
      append_result (source, { false, {}, "No console evaluator is attached to this compositor session.", {} });
      return;
    }
  Row row;
  row.source = source;
  row.pending = true;
  rows_.push_back (std::move (row));
  if (rows_.size () > kMaxRows) rows_.erase (rows_.begin ());
  pending_ = true;
  scroll_to_bottom_ = true;
  callbacks_.evaluate (source, [this, source] (GnoblinConsoleResult result) {
    pending_ = false;
    append_result (source, std::move (result));
  });
  queue_redraw ();
}

void
GnoblinConsolePanel::submit_control (GnoblinConsoleControl control, std::string_view source)
{
  if (!callbacks_.control)
    {
      append_result (std::string (source), { false, {}, "No live configuration controller is attached.", {} });
      return;
    }
  pending_ = true;
  callbacks_.control (control, [this, source = std::string (source)] (GnoblinConsoleResult result) {
    pending_ = false;
    append_result (std::move (source), std::move (result));
  });
  queue_redraw ();
}

void
GnoblinConsolePanel::complete ()
{
  const std::string source (input_.data ());
  completion_visible_ = false;
  completions_.clear ();
  completion_source_ = source;
  const std::uint64_t generation = ++*completion_generation_;
  const std::weak_ptr<std::uint64_t> lifetime = completion_generation_;
  if (callbacks_.complete)
    callbacks_.complete (source, [this, lifetime, generation, source] (
        std::vector<GnoblinConsoleCompletion> result) {
      const auto current = lifetime.lock ();
      if (!current || *current != generation)
        return;
      if (!visible_ || source != input_.data ())
        return;
      completions_ = std::move (result);
      if (completions_.size () > 64) completions_.resize (64);
      completion_index_ = 0;
      completion_visible_ = !completions_.empty ();
      queue_redraw ();
    });
  queue_redraw ();
}

void
GnoblinConsolePanel::accept_completion ()
{
  if (!completion_visible_ || completion_index_ < 0 || completion_index_ >= static_cast<int> (completions_.size ())) return;
  if (completion_source_ != input_.data ())
    {
      completion_visible_ = false;
      return;
    }
  const auto &completion = completions_[completion_index_];
  const std::string current (input_.data ());
  const auto token = current.find_last_of (" \t\n;(){}[]");
  const std::string replacement = completion.label.substr (0, completion.label.find ('('));
  const std::string updated = current.substr (0, token == std::string::npos ? 0 : token + 1) + replacement;
  std::snprintf (input_.data (), input_.size (), "%s", updated.c_str ());
  completion_visible_ = false;
  queue_redraw ();
}

void
GnoblinConsolePanel::append_result (std::string source, GnoblinConsoleResult result)
{
  auto pending = std::find_if (rows_.rbegin (), rows_.rend (), [&source] (const Row &row) {
    return row.pending && row.source == source;
  });
  if (pending != rows_.rend ())
    {
      pending->pending = false;
      pending->text = std::move (result.value);
      pending->error = std::move (result.error);
      pending->inspection = std::move (result.inspection);
    }
  else
    {
      Row row;
      row.source = std::move (source);
      row.text = std::move (result.value);
      row.error = std::move (result.error);
      row.inspection = std::move (result.inspection);
      rows_.push_back (std::move (row));
    }
  if (rows_.size () > kMaxRows) rows_.erase (rows_.begin ());
  scroll_to_bottom_ = true;
  queue_redraw ();
}

void
GnoblinConsolePanel::append_history (const std::string &source)
{
  history_.erase (std::remove (history_.begin (), history_.end (), source), history_.end ());
  history_.push_back (source);
  if (history_.size () > kMaxRows) history_.erase (history_.begin ());
  history_index_ = history_.size ();
  history_draft_.clear ();
  if (callbacks_.save_history) callbacks_.save_history (history_);
}

void
GnoblinConsolePanel::history_step (int delta)
{
  if (history_.empty ()) return;
  if (history_index_ == history_.size ()) history_draft_ = input_.data ();
  const int next = std::clamp (static_cast<int> (history_index_) + delta, 0, static_cast<int> (history_.size ()));
  history_index_ = static_cast<std::size_t> (next);
  const std::string &value = history_index_ == history_.size () ? history_draft_ : history_[history_index_];
  std::snprintf (input_.data (), input_.size (), "%s", value.c_str ());
}

void
GnoblinConsolePanel::draw_node (const GnoblinConsoleInspectorNode &node, const std::string &id)
{
  ImGui::PushID (id.c_str ());
  const std::string label = node.label + (node.value.empty () ? "" : ": " + node.value);
  if (node.children.empty ())
    {
      ImGui::TextWrapped ("%s", label.c_str ());
      if (node.has_more) ImGui::TextDisabled ("Result preview is limited or contains a repeated reference.");
    }
  else if (ImGui::TreeNode (label.c_str ()))
    {
      for (std::size_t i = 0; i < node.children.size (); i++)
        draw_node (node.children[i], id + ":" + std::to_string (i));
      if (node.has_more) ImGui::TextDisabled ("Result preview is limited or contains a repeated reference.");
      ImGui::TreePop ();
    }
  ImGui::PopID ();
}

void
GnoblinConsolePanel::queue_redraw ()
{
  gnoblin_imgui_overlay_queue_redraw (overlay_);
}
