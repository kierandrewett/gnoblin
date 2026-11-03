/* Annotations for the Lua API: the Request object, in src/config/lua/gnoblin-lua-request.c. */

/*** lua
---Something a shell answers: a passphrase prompt, a polkit authentication, a location
---authorization or a session-lock request. The event that offers it carries it as event.request,
---and so do its follow-up events. Fields are read-only. Methods exist only on the kinds that use
---them; calling another raises an error.
---@class Request
---@field id integer Identifier of the request among requests of its kind.
---@field kind "password"|"confirm"|"authentication"|"location"|"lock"
---@field status "pending"|"claimed"|"answered"|"finished"
---@field result? string The outcome once status is "finished".

---Answer the request. A password prompt takes (password, choice?), a confirm prompt takes
---(confirmed, choice?), authentication takes the response for the current prompt, and location
---takes (allow, accuracy?), where accuracy defaults to the requested accuracy when allowed.
---@param value string|boolean Password, confirmed, response or allow, by kind.
---@param option? boolean|integer The choice checkbox for prompts, or the accuracy for location.
---@return Operation<{request_id: integer, submitted: boolean}>
function Request:answer(value, option) end

---Refuse the request: cancel a prompt or authentication, or deny location.
---@return Operation<{request_id: integer, submitted: boolean}>
function Request:cancel() end

---Take an authentication request and choose the identity, counted from 1 (the default). Answer
---each later prompt with answer().
---@param identity? integer
---@return Operation<{request_id: integer, submitted: boolean}>
function Request:claim(identity) end

---An Operation that succeeds once with the outcome: "answered", "cancelled" or "timeout" for
---prompts; "authorized", "denied" or "cancelled" for authentication; "allowed", "denied" or
---"timeout" for location; "locked" or "timeout" for lock.
---@return Operation<string>
function Request:finished() end

---Call fn(request) for each later authentication prompt or message.
---@param signal "prompt"|"message"
---@param fn fun(request: Request)
function Request:on(signal, fn) end
*/
