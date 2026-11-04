/* Annotations for the Lua API implemented by src/config/lua/gnoblin-lua-state-queries.c,
 * gnoblin-lua-operations.c, gnoblin-lua-install.c, and the corresponding handlers in
 * src/control/gnoblin-control-requests.c and gnoblin-control-runtime-channel.c. */

/*** lua
---@class SessionStatus
---@field state "running" Session status returned by the compositor.
---@field lock_available boolean Whether the compositor can report session lock state.
---@field revision integer Lock state revision.
---@field lock_state? "unlocked"|"covering"|"locked"|"failsafe" Present when lock_available is true.
---@field session_state? "starting"|"running"|"stopping" Session lifecycle state when supported.
---@field session_revision? integer Lifecycle revision, present with session_state.

---@class RuntimeStatus
---@field state "starting"|"running"|"restarting"|"unavailable" Runtime worker state.
---@field generation integer Active runtime generation.

---@class PrivacyAvailability
---@field screen_sharing boolean Whether screen sharing state is available.
---@field recording boolean Whether recording state is available.
---@field microphone_in_use boolean Whether microphone use can be observed.
---@field camera_in_use boolean Whether camera use can be observed.
---@field location_in_use boolean Whether location use can be observed.

---@class PrivacyState
---@field available PrivacyAvailability Availability of each privacy signal.
---@field screen_sharing boolean Whether a screen sharing session is active.
---@field recording boolean Whether a recording session is active.
---@field microphone_in_use? boolean Present when microphone monitoring is available.
---@field camera_in_use? boolean Present when camera monitoring is available.
---@field location_in_use? boolean Present when location monitoring is available.
---@field revision integer Privacy state revision.

---@class VersionInfo
---@field gnoblin string Gnoblin version, or "unknown" when unavailable.
---@field gnome string GNOME version, or "unknown" when unavailable.
---@field mutter string Mutter version, or "unknown" when unavailable.
---@field lua string Lua version.
---@field api string Native API version, or "unknown" when unavailable.
---@field git_remote string Sanitized source remote, or "unknown" when unavailable.
---@field git_sha string Source revision, or "unknown" when unavailable.
---@field build_id string Build identifier, or "unknown" when unavailable.

---@class SessionLockResult
---@field dispatched boolean Whether the lock request was dispatched.
---@field subscribers integer Number of session lock subscribers.

---@class AcceptedResult
---@field accepted boolean Whether the request was accepted.

---@class ConfigReloadResult
---@field ok boolean Whether the reload request succeeded.
---@field action string Reload action reported by the runtime.

---@class PrivacyStopResult
---@field requested integer Number of privacy sessions asked to stop.

---@class ConsoleToggleResult
---@field toggled boolean Whether the console was toggled.

---@alias ColorScheme "default"|"prefer-dark"|"prefer-light"

---Read the current compositor session and lock state.
---@return SessionStatus
function gnoblin.session.status() end

---Request that the compositor lock the session.
---@return Operation<SessionLockResult>
function gnoblin.session.lock() end

---Request that the session log out.
---@return Operation<AcceptedResult>
function gnoblin.session.logout() end

---Ask logind to suspend the machine. The Operation finishes with logind's answer: it succeeds
---when logind accepts, and fails with `denied` (policy or polkit), `unsupported`, `busy` or
---`internal` (the D-Bus error is in the message).
---@return Operation<AcceptedResult>
function gnoblin.session.suspend() end

---Ask logind to hibernate the machine. The Operation finishes with logind's answer, as for
---`suspend`.
---@return Operation<AcceptedResult>
function gnoblin.session.hibernate() end

---Ask logind to reboot the machine. The Operation finishes with logind's answer, as for `suspend`.
---@return Operation<AcceptedResult>
function gnoblin.session.reboot() end

---Ask logind to power off the machine. The Operation finishes with logind's answer, as for
---`suspend`.
---@return Operation<AcceptedResult>
function gnoblin.session.poweroff() end

---Read the Lua runtime worker state and generation.
---@return RuntimeStatus
function gnoblin.runtime.status() end

---Ask the runtime to reload its configuration.
---@return Operation<ConfigReloadResult>
function gnoblin.runtime.reload_config() end

---Read the available privacy signals and current activity.
---@return PrivacyState
function gnoblin.privacy.state() end

---Request that active screen sharing sessions stop.
---@return Operation<PrivacyStopResult>
function gnoblin.privacy.stop_sharing() end

---Request that active recording sessions stop.
---@return Operation<PrivacyStopResult>
function gnoblin.privacy.stop_recording() end

---Read installed Gnoblin, GNOME, Mutter, Lua, API, and build version information.
---@return VersionInfo
function gnoblin.version() end

---Toggle the shell console.
---@return Operation<ConsoleToggleResult>
function gnoblin.console.toggle() end

---Read the current desktop color scheme, or nil when unavailable.
---@return ColorScheme|nil
function gnoblin.appearance.color_scheme() end

---Return a detached copy of the configuration that the files loaded so far have built, with the
---internal hyphenated names, such as `snapshot()["window-rules"]`. Use it only while the config
---loads; at run time it is an error, and `gnoblin.settings` holds the committed settings.
---@return Json
function gnoblin.snapshot() end

---@class Capability
---@field id string Capability ID.
---@field description string What the capability provides.
---@field available boolean Whether the capability is available now.
---@field reason? string Why the capability is unavailable, when it is not.
---@field revision integer State revision for this snapshot.

---Return the native compositor and protocol capabilities as read-only records.
---@return Capability[]
function gnoblin.capabilities.list() end
*/
