/* Describes monitor and layer Lua APIs registered in gnoblin-lua-install.c and implemented by
 * gnoblin-lua-queries.c, gnoblin-lua-read.c, and gnoblin-lua-animation.c. */

/*** lua
---@class MonitorRect
---@field x integer X coordinate in compositor layout.
---@field y integer Y coordinate in compositor layout.
---@field width integer Width in logical pixels.
---@field height integer Height in logical pixels.

---@class Monitor
---@field id string Monitor ID.
---@field index integer Zero-based monitor index.
---@field x integer X coordinate in compositor layout.
---@field y integer Y coordinate in compositor layout.
---@field width integer Width in logical pixels.
---@field height integer Height in logical pixels.
---@field primary boolean Whether this is the primary monitor.
---@field scale number Display scale factor.
---@field enabled boolean Whether the monitor is enabled.
---@field name? string Display name when provided by the monitor.
---@field make? string Manufacturer when provided by the monitor.
---@field model? string Model when provided by the monitor.
---@field serial? string Serial number when provided by the monitor.
---@field refresh_rate? number Refresh rate in hertz when a current mode is available.
---@field transform? string Display transform, such as "normal" or "90".
---@field work_area? MonitorRect Work area after layer surfaces reserve exclusive zones.
---@field revision integer State revision for this snapshot.

---@class PrivacyScreenMonitor
---@field id string Monitor ID.
---@field available boolean Whether the monitor supports privacy screen control.
---@field enabled? boolean Whether privacy screen is enabled when available.
---@field locked? boolean Whether privacy screen state is locked when available.

---@class PrivacyScreenSnapshot
---@field requested_enabled boolean Requested global privacy screen state.
---@field source "runtime"|"system"|"config" Where the requested state comes from.
---@field revision integer Snapshot revision.
---@field monitors PrivacyScreenMonitor[] Per-monitor availability and state.

---@class LayerFilter
---@field monitor_id? string Exact monitor ID.
---@field namespace? string Exact layer namespace.
---@field layer? string Exact layer type.

---@class LayerSurface
---@field id string Stable layer surface ID.
---@field namespace string Layer surface namespace.
---@field title string Layer surface title.
---@field layer string Layer-shell layer value.
---@field monitor_id? string Monitor ID when assigned.
---@field revision integer State revision for this snapshot.

---@class LayerAnimationPolicy
---@field namespace string Layer namespace.
---@field enter any Enter animation policy.
---@field exit any Exit animation policy.
---@field window_shadow any Shadow policy for windows matching the layer namespace.
---@field revision integer Configuration revision.

---@class PrivacyScreenSetArgs
---@field value boolean|"inherit" true or false sets an override; "inherit" clears the runtime
override.

---Return the current monitor snapshot.
---@return Monitor[]
function gnoblin.monitors.list() end

---Return the primary monitor, or nil when none is available.
---@return Monitor|nil
function gnoblin.monitors.primary() end

---Return the current privacy screen settings and per-monitor states.
---@return PrivacyScreenSnapshot
function gnoblin.monitors.privacy_screen() end

---Set the runtime privacy screen override.
---@param value boolean|"inherit" true or false sets the override; "inherit" restores the configured
or system value.
---@return Operation<Json>
function gnoblin.monitors.set_privacy_screen(value) end

---Return monitors through the legacy singular namespace alias.
---@return Monitor[]
function gnoblin.monitor.list() end

---Return the current layer surface snapshot, optionally filtered.
---@param filter? LayerFilter
---@return LayerSurface[]
function gnoblin.layers.list(filter) end

---Return the configured animation policy for a layer namespace.
---@param namespace string UTF-8 namespace from 1 to 128 bytes.
---@return LayerAnimationPolicy
function gnoblin.layers.animation_policy(namespace) end

---Return layer surfaces through the legacy singular namespace alias.
---@return LayerSurface[]
function gnoblin.layer.list() end
*/
