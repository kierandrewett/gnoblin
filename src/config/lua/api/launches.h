/* Annotations for launch APIs implemented in src/config/lua/gnoblin-lua-state-queries.c,
 * src/config/lua/gnoblin-lua-objects.c, src/control/gnoblin-control-launches.c, and
 * src/config/lua/gnoblin-lua-read.c. */

/*** lua
---A launch the compositor is watching for. It is the same object for as long as the launch
---exists: every read of one token returns it, and the compositor's changes update it. It is
---alive while state is "pending". It ends when its window appears ("started"), when the
---deadline passes ("timed_out"), or when it is finished ("ended"). The compositor does not
---record which window or Process answered a launch, so a Launch has no link to either; match the
---window by its app_id.
---@class Launch
---@field token string Caller-supplied launch token.
---@field alive boolean False once the launch has left "pending".
---@field application string Application ID, WM class, or WM class instance to observe.
---@field started_at integer Start time in Unix milliseconds.
---@field timeout_ms integer Launch observation deadline in milliseconds.
---@field state "pending"|"started"|"timed_out"|"ended" Current launch state.
---@field revision integer Launch snapshot revision; 0 until the compositor confirms the launch.
---@observable state

---@class LaunchBeginOptions
---@field timeout_ms? integer Timeout in milliseconds, from 100 to 10000. Default 3000.

---@class LaunchEndResult
---@field ok boolean True when the request was accepted.
---@field token string Launch token.

---Begin tracking a matching application launch and return the Launch. It exists at once, pending,
---with the values asked for; the compositor's confirmation and changes update it. A token that
---is already in use ends the earlier Launch with the reason "replaced". If the compositor refuses
---the launch, it ends with the reason and its state becomes "ended".
---@param token string Non-empty token, at most 128 characters.
---@param application string Non-empty application identifier, at most 512 characters.
---@param options? LaunchBeginOptions
---@return Launch
function gnoblin.launches.begin(token, application, options) end

---Return the current launch records, each the Launch object for its token.
---@return Launch[]
function gnoblin.launches.list() end

---Stop watching for this launch. A launch that is not pending is unchanged.
---@return Operation<LaunchEndResult>
function Launch:finish() end

---Call fn(launch, ...) on a signal of this launch: "changed" (with the list of changed fields),
---"started" when its window appeared, "timed_out" when the deadline passed, or "closed" when it
---left "pending" for any reason, after one of the other two. A launch that the compositor
---refused or that a new begin replaced passes the reason as a second argument to "closed".
---Listening ends after "closed".
---@param signal "changed"|"started"|"timed_out"|"closed"
---@param fn fun(launch: Launch, changed_or_reason?: string[]|string)
---@return Subscription
function Launch:on(signal, fn) end

---Call fn(launch, value, old) when one field of this launch changes.
---@param property string A field of Launch, such as "state".
---@param fn fun(launch: Launch, value: any, old: any)
---@return Subscription
function Launch:watch(property, fn) end
*/
