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

---@class SessionActivity
---@field available boolean Whether idle activity data is available.
---@field idle boolean Whether the session has been idle for at least threshold_ms.
---@field threshold_ms integer Idle threshold in milliseconds.
---@field idle_for_ms integer Time since the last activity in milliseconds.
---@field revision integer Activity snapshot revision.

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

---@class LocationAuthorizationResult
---@field request_id integer Identifier of the answered authorization request.
---@field submitted boolean Whether the response was submitted.

---@class ConsoleToggleResult
---@field toggled boolean Whether the console was toggled.

---@alias ColorScheme "default"|"prefer-dark"|"prefer-light"

---Read the current compositor session and lock state.
---@return SessionStatus
function gnoblin.session.status() end

---Read the session idle state and elapsed idle time.
---@return SessionActivity
function gnoblin.session.activity() end

---Request that the compositor lock the session.
---@return Operation<SessionLockResult>
function gnoblin.session.lock() end

---Request that the session log out.
---@return Operation<AcceptedResult>
function gnoblin.session.logout() end

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

---Answer a pending location authorization request. The request ID must have been received from an
active authorization event subscription.
---@param request LocationAuthorizationArgs
---@return Operation<LocationAuthorizationResult>
function gnoblin.location.authorize_app(request) end

---@class LocationAuthorizationArgs
---@field request_id integer Positive ID from a pending location authorization request.
---@field allow boolean Whether to allow location access.
---@field accuracy integer 0 when denied; when allowed, one of 1, 4, 5, 6, or 8 and no greater than
the requested accuracy.

---Read installed Gnoblin, GNOME, Mutter, Lua, API, and build version information.
---@return VersionInfo
function gnoblin.version() end

---Toggle the shell console.
---@return Operation<ConsoleToggleResult>
function gnoblin.console.toggle() end

---Read the current desktop color scheme, or nil when unavailable.
---@return ColorScheme|nil
function gnoblin.appearance.color_scheme() end

---Return a detached copy of the active Lua configuration table.
---@return Json
function gnoblin.snapshot() end
*/
