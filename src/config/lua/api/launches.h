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

---@class LaunchBeginOptions
---@field timeout_ms? integer Timeout in milliseconds, from 100 to 10000. Default 3000.

---@class LaunchEndResult
---@field ok boolean True when the request was accepted.
---@field token string Launch token.

---Begin tracking a matching application launch.
---@param token string Non-empty token, at most 128 characters.
---@param application string Non-empty application identifier, at most 512 characters.
---@param options? LaunchBeginOptions
---@return Operation<Launch>
function gnoblin.launches.begin(token, application, options) end

---Finish tracking a launch token.
---@param token string Non-empty token, at most 128 characters.
---@return Operation<LaunchEndResult>
function gnoblin.launches.finish(token) end

---Return the current launch records.
---@return Launch[]
function gnoblin.launches.list() end
*/
