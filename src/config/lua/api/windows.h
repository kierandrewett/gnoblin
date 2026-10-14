/* Describes the window and focus Lua API registered in gnoblin-lua-install.c and implemented by
 * gnoblin-lua-queries.c, gnoblin-lua-contexts.c, and gnoblin-lua-variant.c. */

/*** lua
---@class WindowFrame
---@field x integer Frame origin on the compositor layout, from -100000 to 100000.
---@field y integer Frame origin on the compositor layout, from -100000 to 100000.
---@field width integer Frame width, from 1 to 32768.
---@field height integer Frame height, from 1 to 32768.

---@class Window
---@field id string Stable window ID.
---@field title string Window title; empty when the native window has no title.
---@field app_id string Application ID used for matching.
---@field gtk_app_id? string GTK application ID, when present.
---@field wm_class? string Window manager class, when present.
---@field rule_app_id? string Application ID used by window rules, when present.
---@field workspace_id? string ID of the workspace containing the window.
---@field workspace_number? integer One-based workspace number.
---@field monitor_index? integer Zero-based monitor index.
---@field monitor_id? string ID of the monitor containing the window.
---@field above? boolean Whether the window is kept above others.
---@field sticky? boolean Whether the window appears on all workspaces.
---@field demands_attention? boolean Whether the window requests attention.
---@field unresponsive? boolean Whether the client has stopped answering liveness pings.
---@field closable? boolean Whether the window can be closed.
---@field minimizable? boolean Whether the window can be minimized.
---@field maximizable? boolean Whether the window can be maximized.
---@field movable? boolean Whether the window can be moved.
---@field resizable? boolean Whether the window can be resized.
---@field role? string Window role.
---@field type? integer Native window type value.
---@field maximized? boolean Whether the window is maximized.
---@field fullscreen? boolean Whether the window is fullscreen.
---@field minimized? boolean Whether the window is minimized.
---@field last_user_time? integer Timestamp of the most recent user interaction.
---@field parent? string ID of the parent window, when present.
---@field monitor? integer Zero-based monitor index.
---@field frame WindowFrame Window frame rectangle.
---@field focused boolean Whether the window currently has focus.
---@field modal boolean Whether the window is a modal dialog.
---@field revision integer State revision for this snapshot.

---@class WindowFilter
---@field app_id? string Exact application ID.
---@field title? string Case-insensitive substring of the title.
---@field focused? boolean Include focused windows when true, or unfocused windows when false.
---@field workspace_id? string Exact workspace ID.
---@field monitor_id? string Exact monitor ID.

---@class WindowPosition
---@field x integer X coordinate, from -100000 to 100000.
---@field y integer Y coordinate, from -100000 to 100000.

---@class WindowSize
---@field width integer Width, from 1 to 32768.
---@field height integer Height, from 1 to 32768.

---@class WindowMonitorSelector
---@field id string Monitor ID.

---@class WindowThumbnailSize
---@field width integer Thumbnail width from 1 to 480 pixels.
---@field height integer Thumbnail height from 1 to 320 pixels.

---@class FocusContext

---@class SnapContextFrame
---@field x integer X coordinate, from -100000 to 100000.
---@field y integer Y coordinate, from -100000 to 100000.
---@field width integer Width, from 1 to 32768.
---@field height integer Height, from 1 to 32768.

---@class SnapContextCommitArgs
---@field monitor_id string Nonempty UTF-8 monitor ID of at most 128 bytes.
---@field frame SnapContextFrame Target frame rectangle.

---@class SnapContext
---@field window_id string ID of the focused window captured by this context.
---@field monitor_id string ID of the monitor containing the focused window.
---@field expires_at_us integer Monotonic expiry time in microseconds.
---@field monitor MonitorRect Monitor bounds at context creation.
---@field work_area MonitorRect Usable monitor area at context creation.
---@field original_frame WindowFrame Window frame at context creation.
---@field settings_revision integer Configuration revision captured by the context.
---@field native_generation integer Native session generation captured by the context.

---Return the current window snapshot, optionally filtered by application, title, focus, workspace,
or monitor.
---@param filter? WindowFilter
---@return Window[]
function gnoblin.windows.list(filter) end

---Identity and window-rule fields for one window, the record that gnoblinctl window match
---prints. Selecting "active" means the focused window.
---@param window? string "active" or a stable window ID; the default is "active".
---@return WindowMatch
function Window:match(window) end

---The record that Window:match returns.
---@class WindowMatch
---@field id string Stable window ID of the matched window.
---@field identity WindowMatchIdentity Verified identity.
---@field match WindowRuleMatch Fields a window rule can match on.

---Identity verified from the window record.
---@class WindowMatchIdentity
---@field app_id string Application ID used for matching.
---@field gtk_app_id? string GTK application ID, when present.
---@field wm_class? string Window manager class, when present.

---Close this window.
---@return Operation<Json>
function Window:close() end

---Raise this window above the others without focusing it.
---@return Operation<Json>
function Window:raise() end

---Lower this window below the others.
---@return Operation<Json>
function Window:lower() end

---Send SIGKILL to the client process that owns this window. Use it for a window whose
---`unresponsive` field is true; the client loses unsaved work.
---@return Operation<Json>
function Window:kill() end

---Minimize this window.
---@return Operation<Json>
function Window:minimize() end

---Toggle this window's minimized state.
---@return Operation<Json>
function Window:toggle_minimize() end

---Remove this window from its minimized state.
---@return Operation<Json>
function Window:unminimize() end

---Restore this window from its minimized, maximized, or fullscreen state.
---@return Operation<Json>
function Window:restore() end

---Restore this window, or minimize it when already restored.
---@return Operation<Json>
function Window:restore_or_minimize() end

---Set whether this window is maximized.
---@param enabled boolean True to maximize; false to restore.
---@return Operation<Json>
function Window:set_maximized(enabled) end

---Set whether this window is fullscreen.
---@param enabled boolean True to enter fullscreen; false to leave it.
---@return Operation<Json>
function Window:set_fullscreen(enabled) end

---Set whether this window stays above other windows.
---@param enabled boolean True to keep above; false to clear.
---@return Operation<Json>
function Window:set_above(enabled) end

---Set whether this window appears on every workspace.
---@param enabled boolean True to make sticky; false to clear.
---@return Operation<Json>
function Window:set_sticky(enabled) end

---Move this window to a position in the compositor layout.
---@param position WindowPosition
---@return Operation<Json>
function Window:move(position) end

---Resize this window.
---@param size WindowSize
---@return Operation<Json>
function Window:resize(size) end

---Move this window to a workspace.
---@param workspace WorkspaceSelector Select by exactly one of id or one-based number.
---@param options? WorkspaceMoveOptions
---@return Operation<Json>
function Window:move_to_workspace(workspace, options) end

---Move this window to a monitor.
---@param monitor string|WindowMonitorSelector Monitor ID or a table containing id.
---@return Operation<Json>
function Window:move_to_monitor(monitor) end

---Focus this window using a live shortcut-event context.
---@param context FocusContext Live context from the current shortcut event.
---@return Operation<Json>
function Window:focus(context) end

---Begin an interactive move using a live shortcut-event context.
---@param context FocusContext Live context from the current shortcut event.
---@return Operation<Json>
function Window:begin_move(context) end

---Begin an interactive resize using a live shortcut-event context.
---@param edge "north"|"south"|"east"|"west"|"north_east"|"north_west"|"south_east"|"south_west"
---@param context FocusContext Live context from the current shortcut event.
---@return Operation<Json>
function Window:begin_resize(edge, context) end

---Capture a thumbnail at the requested dimensions.
---@param size WindowThumbnailSize
---@return Operation<Json>
function Window:thumbnail(size) end

---Commit a snap context to a monitor and frame. A context can be committed only once.
---@param request SnapContextCommitArgs
---@return Operation<Json>
function SnapContext:commit(request) end
*/
