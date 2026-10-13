/* Describes auth and prompt operation arguments in src/config/lua/gnoblin-lua-operations.c. */

/*** lua
---@class AuthBeginArgs
---@field request_id integer Positive request identifier.
---@field identity? integer Positive identity identifier; omitted when no identity is supplied.

---@class AuthRespondArgs
---@field request_id integer Positive request identifier.
---@field response string UTF-8 response of at most 4096 bytes.

---@class AuthCancelArgs
---@field request_id integer Positive request identifier.

---@class PromptRespondArgs
---@field request_id integer Positive request identifier.
---@field password? string|nil UTF-8 password of at most 4096 bytes, or nil.
---@field confirmed? boolean Confirmation value.
---@field choice? boolean Choice value.

---@class PromptCancelArgs
---@field request_id integer Positive request identifier.

---Begin an authentication request for an identity.
---@param request AuthBeginArgs
---@return Operation<any>
function gnoblin.auth.begin(request) end

---Submit a UTF-8 authentication response of at most 4096 bytes.
---@param request AuthRespondArgs
---@return Operation<any>
function gnoblin.auth.respond(request) end

---Cancel an authentication request.
---@param request AuthCancelArgs
---@return Operation<any>
function gnoblin.auth.cancel(request) end

---Answer a prompt with an optional password, confirmation, or choice.
---@param request PromptRespondArgs
---@return Operation<any>
function gnoblin.prompt.respond(request) end

---Cancel a prompt request.
---@param request PromptCancelArgs
---@return Operation<any>
function gnoblin.prompt.cancel(request) end
*/
