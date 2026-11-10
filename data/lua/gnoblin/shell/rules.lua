-- data/lua/gnoblin/shell/rules.lua: window rules (window_rule.md). A rule is { match = {...}, <field> = value, ... }.
-- Patterns are Lua string patterns matched unanchored with string.find; use ^ and $ to anchor. A malformed
-- pattern, an unknown match or field name, or a wrong value type raises at compile time, never at match time.
-- A window with no app_id matches app_id patterns against its WM class.
-- Pure functions over window-like tables: { app_id, class, title, focused, workspace_id, ... }.
local M = {}

local MATCH_KEYS = { app_id = "pattern", title = "pattern", focused = "boolean", workspace_id = "string" }

local FIELDS = {
    workspace = function(v)
        return type(v) == "table" and v.id ~= nil
    end,
    fullscreen = function(v)
        return type(v) == "boolean"
    end,
    secure = function(v)
        return type(v) == "boolean"
    end,
    corners = function(v)
        return type(v) == "table" and type(v.radius) == "number"
    end,
}

local function fail(i, msg)
    error(("window rule %d: %s"):format(i, msg), 3)
end

-- Validate and freeze the rule list. Returns a list of { match = {...}, values = {...} }.
function M.compile(list)
    local out = {}
    for i, rule in ipairs(list) do
        if type(rule.match) ~= "table" then
            fail(i, "a rule needs a match table")
        end
        local match = {}
        for k, want in pairs(rule.match) do
            local kind = MATCH_KEYS[k]
            if not kind then
                fail(i, ("unknown match key %q"):format(tostring(k)))
            end
            if kind == "pattern" then
                if type(want) ~= "string" then
                    fail(i, ("match.%s must be a string pattern"):format(k))
                end
                -- Parse the pattern now so a malformed one is a load error.
                local ok, err = pcall(string.find, "x", want)
                if not ok then
                    fail(i, ("match.%s is a malformed pattern: %s"):format(k, tostring(err)))
                end
            elseif kind == "boolean" and type(want) ~= "boolean" then
                fail(i, ("match.%s must be a boolean"):format(k))
            elseif kind == "string" and type(want) ~= "string" then
                fail(i, ("match.%s must be a string"):format(k))
            end
            match[k] = want
        end
        local values = {}
        for k, v in pairs(rule) do
            if k ~= "match" then
                local check = FIELDS[k]
                if not check then
                    fail(i, ("unknown field %q"):format(tostring(k)))
                end
                if not check(v) then
                    fail(i, ("field %q has the wrong type or shape"):format(k))
                end
                values[k] = v
            end
        end
        out[i] = { match = match, values = values }
    end
    return out
end

local function matches_pattern(value, pat)
    return string.find(value, pat) ~= nil
end

-- Whether one compiled rule matches a window.
local function rule_matches(rule, w)
    for k, want in pairs(rule.match) do
        local ok
        if k == "app_id" then
            local id = w.app_id
            if id == nil or id == "" then
                id = w.class or ""
            end
            ok = matches_pattern(id, want)
        elseif k == "title" then
            ok = matches_pattern(w.title or "", want)
        elseif k == "focused" then
            ok = (w.focused == true) == want
        else
            ok = w.workspace_id == want
        end
        if not ok then
            return false
        end
    end
    return true
end

-- The field values a window takes from the rules. The last matching rule wins for each field.
function M.effects(compiled, w)
    local out = {}
    for _, rule in ipairs(compiled) do
        if rule_matches(rule, w) then
            for k, v in pairs(rule.values) do
                out[k] = v
            end
        end
    end
    return out
end

-- A key naming the set of rules that match a window. Re-apply the rules only when it changes.
function M.key(compiled, w)
    local hit = {}
    for i, rule in ipairs(compiled) do
        if rule_matches(rule, w) then
            hit[#hit + 1] = tostring(i)
        end
    end
    return table.concat(hit, ",")
end

return M
