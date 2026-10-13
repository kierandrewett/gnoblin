/* Describes workspace Lua APIs registered in gnoblin-lua-install.c and implemented by
 * gnoblin-lua-queries.c, gnoblin-lua-operations.c, and gnoblin-lua-variant.c. */

/*** lua
---@class Workspace
---@field id? string Stable workspace ID. The current native workspace snapshot does not provide
this field.
---@field number integer One-based workspace number.
---@field name string Workspace name.
---@field active boolean Whether this is the active workspace.
---@field windows integer Number of windows on this workspace.

---@class WorkspaceCreateArgs
---@field name string Nonempty UTF-8 name of at most 80 characters.
---@field id? string Optional ID. Must start with a letter or number and contain only letters,
numbers, '.', '_' or '-'; at most 64 characters. Lua accepts it, but the current native compositor
rejects it.
---@field activate? boolean Activate the new workspace after creation. Default false.

---@class WorkspaceRenameArgs
---@field id? string Stable workspace ID; provide exactly one of id or number.
---@field number? integer One-based workspace number from 1 to 1024; provide exactly one of id or
number.
---@field name string Nonempty UTF-8 name of at most 80 characters.

---@class WorkspaceSelector
---@field id? string Stable workspace ID; provide exactly one of id or number.
---@field number? integer One-based workspace number from 1 to 1024; provide exactly one of id or
number.

---@class WorkspaceMoveOptions
---@field follow? boolean Activate the destination workspace after moving. Default false.

---@class WorkspaceMoveActiveArgs
---@field workspace WorkspaceSelector Destination workspace.
---@field follow? boolean Activate the destination after moving. Default false.

---@class WorkspaceMoveWindowArgs
---@field window string Stable window ID, or "active" for the focused window.
---@field workspace WorkspaceSelector Destination workspace.
---@field follow? boolean Activate the destination after moving. Default false.

---@class WorkspaceMoveResult
---@field workspace Workspace Destination workspace snapshot.
---@field window string ID of the moved window.
---@field follow boolean Whether the destination workspace was activated.

---Return the current workspace snapshot.
---@return Workspace[]
function gnoblin.workspaces.list() end

---Return the active workspace, or nil when unavailable.
---@return Workspace|nil
function gnoblin.workspaces.active() end

---Find a workspace by its stable ID when the snapshot supplies IDs. Current native workspace
snapshots omit IDs, so this returns nil there.
---@param id string Stable workspace ID.
---@return Workspace|nil
function gnoblin.workspaces.by_id(id) end

---Create a workspace with a name and optional activation.
---@param workspace WorkspaceCreateArgs
---@return Operation<Workspace>
function gnoblin.workspaces.create(workspace) end

---Rename a workspace selected by ID or one-based number.
---@param workspace WorkspaceRenameArgs
---@return Operation<Workspace>
function gnoblin.workspaces.rename(workspace) end

---Remove an inactive, empty workspace when at least one other workspace remains.
---@param workspace WorkspaceSelector
---@return Operation<Workspace>
function gnoblin.workspaces.remove(workspace) end

---Activate a workspace selected by ID or one-based number. The current native compositor supports
number selectors and rejects ID selectors.
---@param workspace WorkspaceSelector
---@return Operation<Workspace>
function gnoblin.workspaces.activate(workspace) end

---Activate the next workspace, wrapping to the first.
---@return Operation<Workspace>
function gnoblin.workspaces.next() end

---Activate the previous workspace, wrapping to the last.
---@return Operation<Workspace>
function gnoblin.workspaces.previous() end

---Move the focused window to a workspace.
---@param request WorkspaceMoveActiveArgs
---@return Operation<WorkspaceMoveResult>
function gnoblin.workspaces.move_active(request) end

---Move a window to a workspace.
---@param request WorkspaceMoveWindowArgs
---@return Operation<WorkspaceMoveResult>
function gnoblin.workspaces.move_window(request) end

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

---Move a window into this workspace.
---@param window Window|string Window snapshot or stable window ID.
---@param options? WorkspaceMoveOptions
---@return Operation<WorkspaceMoveResult>
function Workspace:move_here(window, options) end
*/
