-- tests/lua/fake_core.lua: a deterministic stand-in for the native core of design section 5a.4.
-- Plain Lua. The chunk returns a constructor; calling it gives `core, host`. Nothing here reads a clock or a
-- random source, so the same operations always give the same dump.
--
-- The mirror changes at once, as the native mirror does. The writes made since the last frame form one batch,
-- committed when `host.frame` ends, so a frame is one transaction whatever the number of writes in it.
local SCHEMA = {
    {
        name = "Node",
        props = {
            name = { type = "string", default = "" },
            x = { type = "number", default = 0 },
            y = { type = "number", default = 0 },
            width = { type = "number", default = 0 },
            height = { type = "number", default = 0 },
            opacity = { type = "number", default = 1 },
            scale = { type = "table", default = { 1, 1, 1 } },
            visible = { type = "boolean", default = true },
            clip = { type = "boolean", default = false },
            parent = { type = "node", readonly = true },
            children = { type = "nodes", readonly = true },
        },
        signals = {
            "ready",
            "destroyed",
            "child_added",
            "key",
            "press",
            "release",
            "motion",
            "scroll",
            "enter",
            "leave",
        },
    },
    { name = "Surface", base = "Node" },
    {
        name = "Window",
        base = "Surface",
        props = {
            app_id = { type = "string", default = "" },
            title = { type = "string", default = "" },
            secure = { type = "boolean", default = false },
            parent_window = { type = "node" },
            activated = { type = "boolean", default = false },
            -- The size the client last committed (0 before its first commit). Only the client writes it.
            committed_width = { type = "number", default = 0, readonly = true },
            committed_height = { type = "number", default = 0, readonly = true },
            -- True once a shell has positioned the window. Only the host sets it, on a shell's write to x or y.
            placed = { type = "boolean", default = false, readonly = true },
            title = { type = "string", default = "" },
            fullscreen = { type = "boolean", default = false },
        },
        signals = { "closed" },
        methods = { "close" },
    },
    { name = "Canvas", base = "Node", methods = { "measure" } },
    {
        name = "View",
        base = "Node",
        props = { source = { type = "node" } },
        methods = { "camera", "capture" },
    },
    { name = "Screen", base = "View", props = { connector = { type = "string", default = "" } } },
    { name = "Camera", base = "Node" },
    {
        name = "Seat",
        base = "Node",
        props = { focus = { type = "node" }, screen = { type = "node" } },
        signals = { "chord" },
        methods = { "claim", "unclaim", "grab" },
    },
    -- `index` is derived from the position in the parent, as the native Workspace reports it.
    {
        name = "Workspace",
        base = "Node",
        props = {
            active = { type = "boolean", default = false },
            index = { type = "number", default = 0, readonly = true },
        },
        signals = { "activate" },
    },
    {
        name = "Lock",
        base = "Node",
        props = { engaged = { type = "boolean", default = false } },
        signals = { "requested" },
        methods = { "authenticate" },
        slow = { authenticate = true },
    },
}

local auth_n = 0 -- token counter for slow methods: each call names a token the host answers under
-- Method bodies. Each receives the mirror record of the node it is called on.
local METHODS = {
    measure = function(_, text, size)
        return #tostring(text) * (size or 14) * 0.5
    end,
    claim = function() end,
    unclaim = function() end,
    grab = function() end,
    close = function() end,
    camera = function()
        return 0
    end,
    capture = function()
        return 0
    end,
    authenticate = function()
        auth_n = auth_n + 1
        return "auth" .. auth_n
    end,
}

return function()
    local decl, by_name = {}, {}
    for _, s in ipairs(SCHEMA) do
        decl[#decl + 1] = {
            name = s.name,
            base = s.base,
            props = s.props or {},
            signals = s.signals or {},
            methods = s.methods or {},
            slow = s.slow or {},
        }
        -- Every class sees its own declarations and everything it inherits.
        local all = { name = s.name, props = {}, signals = {}, methods = {} }
        local base = s.base and by_name[s.base]
        if base then
            for k, p in pairs(base.props) do
                all.props[k] = p
            end
            for k in pairs(base.signals) do
                all.signals[k] = true
            end
            for k in pairs(base.methods) do
                all.methods[k] = true
            end
        end
        for k, p in pairs(s.props or {}) do
            all.props[k] = p
        end
        for _, sig in ipairs(s.signals or {}) do
            all.signals[sig] = true
        end
        for _, m in ipairs(s.methods or {}) do
            all.methods[m] = true
        end
        by_name[s.name] = all
    end

    local nodes, next_id = {}, 0 -- id -> { class, props, parent, children }
    local batch, batches = {}, {} -- writes since the last frame; committed batches
    local changes, closing, close_events = {}, {}, {}
    local reports, calls = {}, {}
    local core, host = {}, { reports = reports, batches = batches, calls = calls }

    local function op(t)
        batch[#batch + 1] = t
    end
    local function sorted_ids()
        local ids = {}
        for id in pairs(nodes) do
            ids[#ids + 1] = id
        end
        table.sort(ids)
        return ids
    end
    local function record(id, what)
        local r = nodes[id]
        if not r then
            error(what .. ": no node " .. tostring(id), 3)
        end
        return r
    end
    local function new_node(class)
        local c = by_name[class] or error("no class " .. tostring(class), 3)
        next_id = next_id + 1
        local props = {}
        for k, p in pairs(c.props) do
            props[k] = p.default
        end
        nodes[next_id] = { class = class, props = props, parent = nil, children = {} }
        return next_id
    end
    local function unlink(id)
        local r = nodes[id]
        if r.parent then
            local kids = nodes[r.parent].children
            for i, c in ipairs(kids) do
                if c == id then
                    table.remove(kids, i)
                    break
                end
            end
            r.parent = nil
        end
    end
    -- Removes `id` and its subtree. Returns the node references that became nil, sorted, and the ids removed.
    local function remove(id)
        local gone = {}
        local function mark(i)
            gone[i] = true
            for _, c in ipairs(nodes[i].children) do
                mark(c)
            end
        end
        mark(id)
        unlink(id)
        local cleared = {}
        for _, hid in ipairs(sorted_ids()) do
            if not gone[hid] then
                local h = nodes[hid]
                local keys = {}
                for k, v in pairs(h.props) do
                    if gone[v] and by_name[h.class].props[k].type == "node" then
                        keys[#keys + 1] = k
                    end
                end
                table.sort(keys)
                for _, k in ipairs(keys) do
                    h.props[k] = nil
                    cleared[#cleared + 1] = { hid, k }
                end
            end
        end
        local removed = {}
        for i in pairs(gone) do
            removed[#removed + 1] = i
            nodes[i] = nil
        end
        table.sort(removed)
        return cleared, removed
    end

    local ROOT = new_node("Node")

    function core.classes()
        return decl
    end
    function core.root()
        return ROOT
    end
    function core.new(class)
        local id = new_node(class)
        op({ op = "new", id = id, class = class })
        return id
    end
    function core.get(id, key)
        local r = nodes[id]
        if not r then
            if key == "children" then
                return {}
            end
            return nil
        end
        if key == "class" then
            return r.class
        end
        if key == "parent" then
            return r.parent
        end
        if key == "children" then
            return { table.unpack(r.children) }
        end
        if key == "index" and r.class == "Workspace" then
            local kids = r.parent and nodes[r.parent].children or {}
            for i, c in ipairs(kids) do
                if c == id then
                    return i
                end
            end
            return 0
        end
        if not by_name[r.class].props[key] then
            error(("%s has no property %s"):format(r.class, tostring(key)), 2)
        end
        return r.props[key]
    end
    function core.set(id, key, value, seconds, samples)
        local r = record(id, "set")
        if key == "class" or key == "parent" or key == "children" or key == "index" then
            error(key .. " is read-only", 2)
        end
        local p = by_name[r.class].props[key] or error(("%s has no property %s"):format(r.class, tostring(key)), 2)
        if p.readonly then
            error(key .. " is read-only", 2)
        end
        r.props[key] = value
        -- A shell positioning a node that is already in the tree marks it placed. Construction writes the
        -- defaults of x and y before the node has a parent, so they do not count.
        if (key == "x" or key == "y") and by_name[r.class].props.placed and r.parent then
            r.props.placed = true
        end
        op({ op = "set", id = id, key = key, value = value, seconds = seconds, samples = samples and #samples })
    end
    function core.insert(parent, id, index)
        local p = record(parent, "insert")
        local c = record(id, "insert")
        local up = parent
        while up do
            if up == id then
                error("insert: a node cannot contain itself", 2)
            end
            up = nodes[up].parent
        end
        unlink(id)
        local at = index or #p.children + 1
        if at < 1 or at > #p.children + 1 then
            error("insert: index " .. tostring(at) .. " is out of range", 2)
        end
        table.insert(p.children, at, id)
        c.parent = parent
        op({ op = "insert", parent = parent, id = id, index = at })
    end
    function core.destroy(id)
        if not nodes[id] then
            return {}
        end
        local cleared, removed = remove(id)
        for _, i in ipairs(removed) do
            op({ op = "destroy", id = i })
        end
        return cleared
    end
    -- As the native core: `args` is the packed argument bytes, `text` the op's text (a claim's chord).
    function core.call(id, method, args, text)
        local r = record(id, "call")
        if not by_name[r.class].methods[method] then
            error(("%s has no method %s"):format(r.class, method), 2)
        end
        local result = METHODS[method](r, args, text)
        calls[#calls + 1] = { id = id, method = method, args = { args }, text = text, result = result }
        return result
    end
    function core.report(message)
        reports[#reports + 1] = tostring(message)
    end

    -- Runs one frame: the tick gets the native changes and the events, then the batch of writes is committed.
    -- Changes and removals made during the frame are delivered on the next one.
    function host.frame(now, events)
        local pending = changes
        changes = {}
        local list = {}
        for _, e in ipairs(close_events) do
            list[#list + 1] = e
        end
        close_events = {}
        for _, e in ipairs(events or {}) do
            list[#list + 1] = e
        end
        local wake = host.tick(now, pending, list)
        for _, id in ipairs(closing) do
            if nodes[id] then
                local cleared = remove(id)
                for _, c in ipairs(cleared) do
                    changes[#changes + 1] = c
                end
            end
        end
        closing = {}
        batches[#batches + 1] = batch
        batch = {}
        return wake
    end
    -- A change Gnoblin made on its own (a window's title, a pointer position). Delivered in the next frame.
    function host.external(id, key, value)
        local r = record(id, "external")
        r.props[key] = value
        changes[#changes + 1] = { id, key }
    end
    -- Gnoblin ends a node and its subtree. The destroyed events go out in the next frame, children first.
    function host.close(id)
        local order = {}
        local function walk(i)
            for _, c in ipairs(record(i, "close").children) do
                walk(c)
            end
            order[#order + 1] = i
        end
        walk(id)
        for _, i in ipairs(order) do
            close_events[#close_events + 1] = { type = "destroyed", target = i, args = {} }
            closing[#closing + 1] = i
        end
    end
    -- Ids of the live nodes of one class, ascending.
    function host.ids(class)
        local out = {}
        for _, id in ipairs(sorted_ids()) do
            if nodes[id].class == class then
                out[#out + 1] = id
            end
        end
        return out
    end
    -- Table values (such as `scale`) print by content, so two runs dump the same bytes.
    local function stringify(v)
        if type(v) ~= "table" then
            return tostring(v)
        end
        local items = {}
        for _, x in ipairs(v) do
            items[#items + 1] = tostring(x)
        end
        return "{" .. table.concat(items, ",") .. "}"
    end
    -- The whole mirror as text: one line per node, props sorted, so two runs can be compared byte for byte.
    function host.dump()
        local out = {}
        for _, id in ipairs(sorted_ids()) do
            local r = nodes[id]
            local keys = {}
            for k in pairs(r.props) do
                keys[#keys + 1] = k
            end
            table.sort(keys)
            local parts = {}
            for _, k in ipairs(keys) do
                local v = r.props[k]
                parts[#parts + 1] = k .. "=" .. (type(v) == "string" and ("%q"):format(v) or stringify(v))
            end
            out[#out + 1] = ("%d %s parent=%s children=[%s] %s"):format(
                id,
                r.class,
                tostring(r.parent),
                table.concat(r.children, ","),
                table.concat(parts, " ")
            )
        end
        return table.concat(out, "\n")
    end

    return core, host
end
