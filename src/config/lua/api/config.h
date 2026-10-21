/* Describes config declaration and loading functions in src/config/lua/gnoblin-lua-named.c,
 * gnoblin-lua-merge.c, gnoblin-lua-input.c, and gnoblin-lua-install.c. */

/*** lua
---@class WindowRuleMatch
---@field type? "window"|"layer" Match ordinary windows or layer surfaces.
---@field focused? boolean Match windows according to their current focused state.
---@field app_id? string Lua pattern matched against the window application ID.
---@field title? string Lua pattern matched against the window title.
---@field layer? string Lua pattern matched against the window layer.
---@field workspace_id? string Exact current workspace ID.
---@field workspace_number? integer Current one-based workspace number.

---@class WindowRuleWorkspace
---@field id? string Nonempty workspace ID of at most 128 bytes; set exactly one of `id` or
`number`.
---@field number? integer Workspace number from 1 to 1024; set exactly one of `id` or `number`.

---@class WindowRuleCorners
---@field radius? number Corner radius.
---@field smoothing? number Corner smoothing.
---@field border_width? number Border width; clamped by the compositor to -40 through 40.
---@field border_color? string Border color.
---@field padding? number[] Four finite numbers, each from -128 to 128, in top, right, bottom, left
order.
---@field keep_maximized? boolean Preserve corner treatment for maximized windows.
---@field keep_fullscreen? boolean Preserve corner treatment for fullscreen windows.
---@field keep_tiled? boolean Preserve corner treatment for tiled windows.
---@field skip_libadwaita? boolean Skip windows identified as libadwaita.
---@field skip_libhandy? boolean Skip windows identified as libhandy.
---@field remove_csd? boolean Remove client-side decorations.
---@field keep_shadow? boolean Preserve the window shadow.
---@field shadow boolean|WindowShadowLayer|WindowShadowLayer[] Enable the default shadow, configure
one layer, or configure up to four layers.
---@field shadow_animation table Shadow transition settings read by the compositor.
---@field mode? "auto"|"force"|"off" Corner handling mode; `off` disables corner, border, and shadow
effects.

---@class WindowShadowLayer
---@field x? number Horizontal offset.
---@field y? number Vertical offset.
---@field blur? number Blur radius.
---@field spread? number Shadow spread.
---@field opacity? number Shadow opacity.
---@field color? string Shadow color.

---@class WindowRuleFrame
---@field mode? "off"|"auto"|"prefer-server"|"replace" Frame mode.
---@field extents? integer[] Frame extents.
---@field crop? integer[] Frame crop values.
---@field renderer? string Frame renderer.
---@field style? string Frame style.
---@field background? string Frame background color.
---@field foreground? string Frame foreground color.
---@field inactive_background? string Inactive frame background color.
---@field button_layout? string[] Native frame buttons, without duplicates; an empty list hides
them.

---Animation names by event. The reserved `in` key aliases `layer_open`; `out` aliases
`layer_close`.
---@class WindowRuleAnimation
---@field out? string Animation used when a layer closes; alias for `layer_close`.
---@field open? string Animation used when a layer opens.
---@field close? string Animation used when a layer closes.
---@field dialog_open? string Animation used when a dialog opens.
---@field dialog_close? string Animation used when a dialog closes.
---@field layer_open? string Animation used when a layer opens.
---@field layer_close? string Animation used when a layer closes.
---@field minimize? string Animation used when a window minimizes.
---@field restore? string Animation used when a window restores.
---@field workspace_switch? string Animation used when switching workspaces.
---@field shadow_change? string Animation used when a shadow changes.
---@field resize? string Animation used when a window resizes.
---@field duration? integer Animation duration in milliseconds, from 0 to 5000.
---@field easing? string Animation easing.
---@field ease? string Alias for `easing`.

---@class WindowRuleArgs
---@field match? WindowRuleMatch Match patterns; all supplied patterns must match.
---@field workspace? WindowRuleWorkspace Move a matching window to this workspace.
---@field opacity? number Opacity from 0 to 1; compositor clamps values to this range.
---@field blur? number Background blur from 0 to 100.
---@field blur_ignore_shadows? boolean Exclude shadows from the blurred region.
---@field shader? string Shader path.
---@field shader_uniforms? table Shader uniform values.
---@field animation? string|WindowRuleAnimation Animation name or per-event animation map.
---@field frame? WindowRuleFrame Frame options.
---@field corners? WindowRuleCorners Corner, border, and shadow options.

---Append a window rule to the ordered window-rules list.
---@param rule WindowRuleArgs
function gnoblin.window_rule(rule) end

---Load and merge Lua configuration files matching a path or glob.
---@param pattern string Path or glob resolved relative to the current config file and configured
search directories.
---@return boolean success
function gnoblin.load(pattern) end

---Mark a table as an array so configuration conversion preserves it as a list.
---@param values table Array values; numeric keys must form a dense sequence starting at 1 when
converted.
---@return table values
function gnoblin.array(values) end

---Merge settings into the active Lua configuration.
---@param settings table Settings table to merge into the active configuration.
function gnoblin.configure(settings) end

---Return the active configuration file path using GNOBLIN_CONFIG and XDG defaults.
---@return string path
function gnoblin.config.path() end

---Return the bundled default Lua configuration source.
---@return string source
function gnoblin.config.default() end

---Replace the standard config directory with bundled defaults, save the old directory as a backup,
---and reload the restored configuration.
---@return Operation<AcceptedResult>
function gnoblin.config.restore_default() end
*/
