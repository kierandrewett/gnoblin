/* Describes workspace Lua APIs registered in gnoblin-lua-install.c and implemented by
 * gnoblin-lua-queries.c, gnoblin-lua-operations.c, and gnoblin-lua-variant.c. */

/*** lua
---@class Workspace
---@field alive boolean False once the workspace has been removed. Every read of one workspace
returns this same object, updated with each change.
---@field id string Stable workspace ID: the one declared in `gnoblin.configure.workspaces`, the
one `create` was given, or the session ID `@session-N` that Gnoblin assigns.
---@field number integer One-based workspace number.
---@field name string Workspace name.
---@field active boolean Whether this is the active workspace.
---@field windows integer Number of windows on this workspace.
---@field revision integer State revision of this snapshot; it goes up with every change of the
workspace list, so a rename gives a higher number.
---@observable number name active windows

---@class WorkspaceCreateOptions
---@field id? string Optional ID. Must start with a letter or number and contain only letters,
numbers, '.', '_' or '-'; at most 64 characters. The workspace keeps it for the session only;
an ID declared in `gnoblin.configure.workspaces` lasts across sessions.
---@field activate? boolean Activate the new workspace after creation. Default false.

---@class WorkspaceMoveOptions
---@field follow? boolean Activate the destination workspace after moving. Default false.

---Return the current workspace snapshot.
---@return Workspace[]
function gnoblin.workspaces.list() end

---Return the active workspace, or nil when unavailable.
---@return Workspace|nil
function gnoblin.workspaces.active() end

---Find a workspace by its stable ID. Returns nil when no workspace has it. The ID comes from
---`gnoblin.configure.workspaces` or `create`, or is the session ID `@session-N` that Gnoblin gave
---it.
---@param id string Stable workspace ID.
---@return Workspace|nil
function gnoblin.workspaces.by_id(id) end

---Create a workspace with a name and optional activation.
---@param name string Nonempty UTF-8 name of at most 80 characters.
---@param options? WorkspaceCreateOptions
---@return Operation<Workspace>
function gnoblin.workspaces.create(name, options) end

---Activate the next workspace, wrapping to the first.
---@return Operation<Workspace>
function gnoblin.workspaces.next() end

---Activate the previous workspace, wrapping to the last.
---@return Operation<Workspace>
function gnoblin.workspaces.previous() end

---Activate this workspace.
---@return Operation<Workspace>
function Workspace:activate() end

---Rename this workspace.
---@param name string Nonempty UTF-8 name of at most 80 characters.
---@return Operation<Workspace>
function Workspace:rename(name) end

---Remove this workspace if it is inactive and empty.
---@return Operation<Workspace>
function Workspace:remove() end

---Call fn(workspace, ...) on a signal of this workspace: "changed" (with the list of changed
---fields), "activated" or "closed". Listening ends after "closed".
---@param signal "changed"|"activated"|"closed"
---@param fn fun(workspace: Workspace, changed?: string[])
---@return Subscription
function Workspace:on(signal, fn) end

---Call fn(workspace, value, old) when one field of this workspace changes.
---@param property string A field of Workspace, such as "name" or "active".
---@param fn fun(workspace: Workspace, value: any, old: any)
---@return Subscription
function Workspace:watch(property, fn) end
*/
