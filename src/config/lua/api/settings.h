/* Annotations for runtime overrides implemented in src/config/lua/gnoblin-lua-state-queries.c,
 * src/config/lua/gnoblin-lua-objects.c and src/control/gnoblin-control-overrides.c. */

/*** lua
---A value that the runtime layers over the committed configuration, or an effect that it layers
---over the window rules of one window, until the override ends. It is the same object for as long
---as it is alive. The latest override of one setting, or of one effect on one window, wins: an
---older one ends at once with the reason "replaced". An override ends too when it is revoked
---("revoked"), when its window closes ("window_closed"), and when a configuration reload commits
---("config_reloaded"). A stopped or restarted Lua worker drops its overrides as well. An override
---never writes the configuration file, and `gnoblin.settings` keeps showing the committed values.
---@class Override
---@field id string Identifier, "override-" and the number of the request that made it.
---@field alive boolean False once the override has ended.
---@field scope "settings"|"window" What it changes: a setting, or an effect of one window.
---@field name string The setting path, such as "window_management.focus_mode", or the effect name,
such as "blur".
---@field window? string The ID of the window an effect belongs to. Absent for a setting.
---@field value any The value it layers.

---@class OverrideRevokeResult
---@field revoked boolean True when the compositor ended the override.

---Layer a value over one committed setting, for this runtime, and return the Override. The path
---is "section.key" with the names of the configuration, such as `window_management.focus_mode`.
---Only a setting that can change while the session runs is accepted: `window_management.*`,
---`compositor.*`, `cursor.theme`, `cursor.size`, `layer_shell.preserve_active_window`,
---`monitors.privacy_screen` and `input.orientation_lock`. Any other path raises an error that
---says why it cannot change at runtime. The value is checked the way the configuration checks it,
---so a value the configuration would refuse raises an error too. The orientation lock and the
---privacy screen take `true` or `false`; a privacy screen needs an output that has one. If the
---compositor refuses the override, it ends and its "closed" listeners get the reason. Call this
---while handling an event, as with every operation.
---@param path string The setting, such as "compositor.visual_bell".
---@param value boolean|number|string The value to layer.
---@return Override
function gnoblin.settings.override(path, value) end

---End this override. The setting or effect returns to the next older layer, which is the
---committed value when there is none. The Override ends, and its "closed" listeners run with the
---reason "revoked". An override that has ended fails with `not_found`.
---@return Operation<OverrideRevokeResult>
function Override:revoke() end

---Call fn(override, reason) when this override ends. `reason` is "revoked", "replaced",
---"window_closed" or "config_reloaded", or the message of the compositor when it refused the
---override. On "config_reloaded" the listener runs just before the old runtime goes, so it can
---read but cannot start an operation. Listening ends after "closed".
---@param signal "closed"
---@param fn fun(override: Override, reason?: string)
---@return {unsubscribe: fun()}
function Override:on(signal, fn) end
*/
