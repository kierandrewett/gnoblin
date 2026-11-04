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
---@field alive boolean False once the window has closed. Every read of one window returns this
same object, updated with each change.
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
---@field pid? integer Process ID of the client, from the display server, not the client.
---@field process? integer Process.id when gnoblin.process.spawn started this window's program;
gnoblin.process.by_id(window.process) gives that Process while the runtime knows it.
---@field monitor? integer Zero-based monitor index.
---@field frame WindowFrame Window frame rectangle.
---@field opacity number Drawn opacity from 0 to 1 that the runtime set; 1 by default.
---@field scale number Drawn scale about the window's centre that the runtime set; 1 by default.
---@field rotation number Drawn rotation in degrees about the window's centre; 0 by default.
---@field offset {x: number, y: number} Drawn offset in logical pixels; {x = 0, y = 0} by default.
---@field focused boolean Whether the window currently has focus.
---@field modal boolean Whether the window is a modal dialog.
---@field revision integer State revision for this snapshot.
---@observable title app_id gtk_app_id wm_class rule_app_id workspace_id workspace_number
---@observable monitor_index monitor_id above sticky demands_attention unresponsive closable
---@observable minimizable maximizable movable resizable role type maximized fullscreen
---@observable minimized last_user_time parent monitor frame opacity scale rotation offset
---@observable focused modal

---@class WindowVisual
---@field id string Window ID.
---@field opacity number
---@field scale number
---@field rotation number
---@field offset {x: number, y: number} Drawn offset in logical pixels.

---@alias GrantScope "focus"|"begin_move"|"begin_resize"|"snap_context"|"text_target"

---Proof that a user gesture just happened, scoped to the operations it may start. The compositor
---makes the proof; a script receives it as `event.grant` on a shortcut activation, a pointer
---binding, an input handler event for a button press, a shortcut session key and a window menu
---request. Timers, the other events and a script's own code have none.
---
---A Grant lists the operations it may take and may name one window. `narrow` returns a Grant
---with fewer of both, and nothing widens one. Each operation of a gesture runs once, whichever
---Grant takes it, so one key press can focus a window and then begin a move. A Grant ends five
---seconds after the gesture, at a configuration reload, and when the session locks.
---@class Grant
---@field scope GrantScope[] Operations this Grant may still take, in the order of GrantScope.
---@field window Window|nil The one window this Grant is narrowed to, or nil when it has no window.
---@field expires_at_us integer Monotonic time, in microseconds, when the gesture ends.
---@field alive boolean False when the Grant has expired, its runtime ended or it has no scope left.

---@class SnapContext
---@field window_id string ID of the focused window captured by this context.
---@field monitor_id string ID of the monitor containing the focused window.
---@field expires_at_us integer Monotonic expiry time in microseconds.
---@field monitor MonitorRect Monitor bounds at context creation.
---@field work_area MonitorRect Usable monitor area at context creation.
---@field original_frame WindowFrame Window frame at context creation.
---@field settings_revision integer Configuration revision captured by the context.
---@field native_generation integer Native session generation captured by the context.

---Return the current window snapshot. Select windows with `gnoblin.find("Window", filter)`.
---@return Window[]
function gnoblin.windows.list() end

---Return the focused window, or nil when no window has focus.
---@return Window|nil
function gnoblin.windows.focused() end

---Return the window with this stable ID, or nil when it does not exist.
---@param id string Stable window ID.
---@return Window|nil
function gnoblin.windows.by_id(id) end

---Return a SnapContext for the focused window and spend the "snap_context" scope of a Grant.
---Callers cannot choose the window. Take it before "focus": the context holds only while the
---window that had focus at the gesture still has it.
---@param grant Grant From a shortcut, pointer binding or input event.
---@return Operation<SnapContext>
function gnoblin.windows.snap_context(grant) end

---Return a Grant for fewer operations, and for one window if `window` is given. It shares the
---gesture with the Grant it comes from: an operation that either one took is gone from both. It
---fails when `scope` names an operation this Grant does not have or the gesture already used,
---or when `window` is not the window this Grant is narrowed to.
---@param scope GrantScope|GrantScope[] One operation, or a list of them.
---@param window? Window|string The window, or its stable ID.
---@return Grant
function Grant:narrow(scope, window) end

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

---Set whether this window is minimized.
---@param enabled boolean True to minimize; false to remove minimization.
---@return Operation<Json>
function Window:set_minimized(enabled) end

---Unmaximize this window, restore the frame it had before a snap, or minimize it, whichever
---applies first.
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
---@param x integer X coordinate, from -100000 to 100000.
---@param y integer Y coordinate, from -100000 to 100000.
---@return Operation<Json>
function Window:move(x, y) end

---Resize this window.
---@param width integer Width, from 1 to 32768.
---@param height integer Height, from 1 to 32768.
---@return Operation<Json>
function Window:resize(width, height) end

---Move this window to a workspace.
---@param workspace Workspace|string|integer The Workspace, its stable ID, or its one-based number
---from 1 to 1024.
---@param options? WorkspaceMoveOptions
---@return Operation<Json>
function Window:move_to_workspace(workspace, options) end

---Move this window to a monitor.
---@param monitor Output|string The Output that shows the monitor, or the monitor ID.
---@return Operation<Json>
function Window:move_to_monitor(monitor) end

---Focus this window. This takes the "focus" scope of a Grant, so the user's key press or click
---gives focus without the compositor's focus-stealing rules getting in the way.
---@param grant Grant From a shortcut, pointer binding, input or window menu event.
---@return Operation<Json>
function Window:focus(grant) end

---Focus a window whose program the runtime started with gnoblin.process.spawn. No key press is
---needed. It waits until the window is shown and no key press or click reached a window for
---250 ms, at most 5 seconds, then succeeds when the window has focus.
---@return Operation<{id: string, focused: boolean}>
function Window:present() end

---Draw this window at an opacity from 0 to 1. Visual changes affect only how the window is drawn
---and picked, not its frame or what the client sees. The runtime's reload clears them.
---@param opacity number
---@return Operation<WindowVisual>
function Window:set_opacity(opacity) end

---Draw this window scaled about its centre, from 0.01 to 10.
---@param scale number
---@return Operation<WindowVisual>
function Window:set_scale(scale) end

---Draw this window rotated about its centre, in degrees from -3600 to 3600.
---@param degrees number
---@return Operation<WindowVisual>
function Window:set_rotation(degrees) end

---Draw this window moved by an offset in logical pixels, each from -100000 to 100000.
---@param x number
---@param y number
---@return Operation<WindowVisual>
function Window:set_offset(x, y) end

---Draw this window without any visual change the runtime set.
---@return Operation<WindowVisual>
function Window:reset_visuals() end

---Layer one window rule effect over this window and return the Override. The effects are those
---that `gnoblin.window_rule` supports: `opacity` (0 to 1), `blur` (0 to 100),
`blur_ignore_shadows`,
---`shader` (a GLSL file path, or "" to clear it), `shader_uniforms`, `frame` and `corners`. The
---last three take the same table as the rule. The value goes over every rule that matches the
---window, as one more rule after them, so it changes only what it sets. The latest override of
---one effect on one window wins; an older one ends with the reason "replaced". The override ends
---when it is revoked, when the window closes, and when a configuration reload commits, and the
---window then shows what its rules give. This is not `set_opacity`: that call changes how the
---window actor is drawn, and `opacity` here is the rule's opacity of the window content, which
---the two multiply. Call this while handling an event, as with every operation.
---@param name string The effect, such as "blur".
---@param value any The value for that effect.
---@return Override
function Window:set_effect(name, value) end

---Call fn(window, ...) on a signal of this window: "changed" (with the list of changed fields),
---"focused", "unfocused" or "closed". Listening ends after "closed".
---@param signal "changed"|"focused"|"unfocused"|"closed"
---@param fn fun(window: Window, changed?: string[])
---@return Subscription
function Window:on(signal, fn) end

---Call fn(window, value, old) when one field of this window changes.
---@param property string A field of Window, such as "title" or "frame".
---@param fn fun(window: Window, value: any, old: any)
---@return Subscription
function Window:watch(property, fn) end

---Begin an interactive move. This takes the "begin_move" scope of a Grant.
---@param grant Grant From a shortcut, pointer binding, input or window menu event.
---@return Operation<Json>
function Window:begin_move(grant) end

---Begin an interactive resize. This takes the "begin_resize" scope of a Grant.
---@param edge "north"|"south"|"east"|"west"|"north_east"|"north_west"|"south_east"|"south_west"
---@param grant Grant From a shortcut, pointer binding, input or window menu event.
---@return Operation<Json>
function Window:begin_resize(edge, grant) end

---Capture a thumbnail at the requested dimensions.
---@param width integer Thumbnail width from 1 to 480 pixels.
---@param height integer Thumbnail height from 1 to 320 pixels.
---@return Operation<Json>
function Window:thumbnail(width, height) end

---Commit a snap context to a monitor and frame. A context can be committed only once.
---@param monitor Output|string The Output that shows the frame, or its monitor ID.
---@param x integer X coordinate, from -100000 to 100000.
---@param y integer Y coordinate, from -100000 to 100000.
---@param width integer Width, from 1 to 32768.
---@param height integer Height, from 1 to 32768.
---@return Operation<Json>
function SnapContext:commit(monitor, x, y, width, height) end
*/
