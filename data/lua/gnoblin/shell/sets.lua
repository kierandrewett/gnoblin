-- data/lua/gnoblin/shell/sets.lua: workspace sets. A set is { name, entries = { { workspace, output? } } }: each entry
-- names a workspace and the output it should show on. Pure functions over plain tables; the caller keeps the list and
-- applies the plan that `activate` returns.
-- `define` validates a whole list and returns a copy, or nil and an error. A refused list is never partly applied:
-- the previous list is untouched and stays with the caller.
-- `activate` returns a plan { set, placements = { { workspace, output } }, warnings, changed }, or nil and an error.
-- An entry whose output is missing falls back to the primary output with a warning.
local M = {}

local function copy(list)
    local out = {}
    for i, s in ipairs(list) do
        local entries = {}
        for j, e in ipairs(s.entries) do
            entries[j] = { workspace = e.workspace, output = e.output }
        end
        out[i] = { name = s.name, entries = entries }
    end
    return out
end

-- Validates `incoming` against the declared workspaces (a set keyed by workspace id). Returns a copy of it on success.
function M.define(_, incoming, declared)
    local names = {}
    for _, s in ipairs(incoming) do
        if s.name == "" then
            return nil, { kind = "empty_name" }
        end
        if names[s.name] then
            return nil, { kind = "duplicate_name", name = s.name }
        end
        names[s.name] = true
        if #s.entries == 0 then
            return nil, { kind = "empty_set", name = s.name }
        end
        local outputs = {}
        for _, e in ipairs(s.entries) do
            if not declared[e.workspace] then
                return nil, { kind = "undeclared_workspace", set = s.name, workspace = e.workspace }
            end
            if e.output ~= nil then
                if outputs[e.output] then
                    return nil, { kind = "duplicate_output", set = s.name, output = e.output }
                end
                outputs[e.output] = true
            end
        end
    end
    return copy(incoming)
end

-- opts: declared (workspace ids), outputs (output names, in order), primary (the fallback output's name), active
-- (the name of the set now live, if any).
function M.activate(list, name, opts)
    local set
    for _, s in ipairs(list) do
        if s.name == name then
            set = s
        end
    end
    if not set then
        return nil, { kind = "unknown_set", name = name }
    end
    local shown = {}
    for _, o in ipairs(opts.outputs) do
        shown[o] = true
    end
    local placements, warnings = {}, {}
    for i, e in ipairs(set.entries) do
        if not opts.declared[e.workspace] then
            return nil, { kind = "undeclared_workspace", set = name, workspace = e.workspace }
        end
        local output = opts.primary
        if e.output ~= nil and shown[e.output] then
            output = e.output
        elseif e.output ~= nil then
            warnings[#warnings + 1] = { kind = "output_fallback", set = name, output = e.output }
        end
        placements[i] = { workspace = e.workspace, output = output }
    end
    return { set = name, placements = placements, warnings = warnings, changed = opts.active ~= name }
end

return M
