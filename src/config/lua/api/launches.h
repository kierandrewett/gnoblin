/* Annotations for launch APIs implemented in src/config/lua/gnoblin-lua-state-queries.c,
 * src/config/lua/gnoblin-lua-install.c, src/control/gnoblin-control-launches.c, and
 * src/config/lua/gnoblin-lua-read.c. */

/*** lua
---@class Launch
---@field token string Caller-supplied launch token.
---@field application string Application ID, WM class, or WM class instance to observe.
---@field started_at integer Start time in Unix milliseconds.
---@field timeout_ms integer Launch observation deadline in milliseconds.
---@field state "pending"|"started"|"timed_out"|"ended" Current launch state.
---@field revision integer Launch snapshot revision.

---@class LaunchBeginArgs
---@field token string Non-empty token, at most 128 characters.
---@field application string Non-empty application identifier, at most 512 characters.
---@field timeout_ms? integer Timeout in milliseconds, from 100 to 10000. Default 3000.

---@class NativeLaunchBeginArgs
---@field token string Non-empty token, at most 128 characters.
---@field application string Non-empty application identifier, at most 512 characters.
---@field milliseconds? integer Timeout in milliseconds. Default 3000; native requests are clamped
to 100–10000.

---@class LaunchEndArgs
---@field token string Non-empty token, at most 128 characters.

---@class LaunchEndResult
---@field ok boolean True when the request was accepted.
---@field token string Launch token.

---@class LaunchSnapshot
---@field launches Launch[] Current launch records.
---@field revision integer Revision of the launch snapshot.

---Begin tracking a matching application launch.
---@param launch LaunchBeginArgs
---@return Operation<Launch>
function gnoblin.launches.begin(launch) end

---Finish tracking a launch token.
---@param token string Non-empty token, at most 128 characters.
---@return Operation<LaunchEndResult>
function gnoblin.launches.finish(token) end

---Alias for launches.finish.
---@param token string Non-empty token, at most 128 characters.
---@return Operation<LaunchEndResult>
function gnoblin.launches.end(token) end

---Return the current launch records.
---@return Launch[]
function gnoblin.launches.list() end

---Return the launch records and their snapshot revision.
---@return LaunchSnapshot
function gnoblin.launches.snapshot() end

---Begin tracking a matching application launch through the singular API.
---@param launch NativeLaunchBeginArgs
---@return Operation<Launch>
function gnoblin.launch.begin(launch) end

---Stop tracking a launch token through the singular API.
---@param request LaunchEndArgs
---@return Operation<LaunchEndResult>
function gnoblin.launch.end(request) end

---Return the current launch snapshot through the singular API.
---@return LaunchSnapshot
function gnoblin.launch.status() end
*/
