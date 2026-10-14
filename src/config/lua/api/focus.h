/* Runtime focus reads implemented by gnoblin-lua-subscriptions.c and
 * gnoblin-lua-queries.c. */

/*** lua
---@class FocusHistoryFilter
---@field workspace_id? string Exact workspace ID to include.
---@field monitor_id? string Exact monitor ID to include.
---@field limit? integer Maximum number of records, from 1 to 256; defaults to 50.

---@class FocusPolicy
---@field focus_mode "click"|"hover"|"hover-strict" How pointer movement changes focus.
---@field focus_new_windows "allow"|"prevent" Whether new windows may take focus.
---@field raise_on_click boolean Whether clicking a window raises it.
---@field auto_raise boolean Whether the focused window is raised automatically.
---@field focus_change_on_pointer_rest boolean Whether focus changes after pointer rest.
---@field auto_raise_delay integer Delay in milliseconds before automatic raise.
---@field revision integer Revision of the committed settings snapshot.

---Return windows in most-recently-focused order, optionally filtered.
---@param filter? FocusHistoryFilter
---@return Window[]
function gnoblin.focus.history(filter) end

---Read the committed focus policy.
---@return FocusPolicy
function gnoblin.focus.policy() end
*/
