-- data/lua/gnoblin/shell/tiling.lua: the stock tiled layouts, ported from data/lua/gnoblin/layouts (which stay
-- untouched until the compositor loads this shell). Each class is a Workspace whose arrange() is a binding over its
-- window children, so adding, removing, reordering or resizing, and changing an option field, re-arranges with no
-- handler. Positions and sizes are floored to whole pixels. Coordinates are workspace-relative.
-- Not yet loaded by the compositor: the old layouts remain the live ones until the shell VM is wired.
-- Dropped from the old API: the M.arrange/ctx/on_request contract and the command tables (inc, dec, swap, rotate,
-- resize_ratio, next, prev). Options are class fields; the focused window comes from the seat.

local ORIENTATIONS = { left = true, right = true, top = true, bottom = true }
local FILLS = { row = true, column = true }
local SPLITS = { aspect = true, horizontal = true, vertical = true }

local function clamp(value, low, high)
    return math.max(low, math.min(high, value))
end

-- Sets a window's rectangle to whole pixels. Visibility is left alone unless the class sets it.
local function place(win, x, y, width, height)
    win.x, win.y = math.floor(x), math.floor(y)
    win.width, win.height = math.floor(width), math.floor(height)
end

-- The window the seat focuses, if it is one of `wins`; otherwise the first window.
local function shown_of(wins)
    local focus = seat and seat.focus
    for _, w in ipairs(wins) do
        if w == focus then
            return w
        end
    end
    return wins[1]
end

-- Maps a rectangle in the master-left frame to the real area for `orientation`. `w0` and `h0` are the workspace size.
local function to_area(orientation, w0, h0, rect)
    local u, v, p, q = rect[1], rect[2], rect[3], rect[4]
    if orientation == "left" then
        return u, v, p, q
    elseif orientation == "right" then
        return w0 - u - p, v, p, q
    elseif orientation == "top" then
        return v, u, q, p
    end
    return v, h0 - u - p, q, p
end

-- Rectangles { u, v, p, q } in a W by H frame whose master column sits at u = 0: the first `master` windows share
-- the master region top to bottom, the rest share the stack to its right. With no stack the master fills the frame.
local function master_stack_rects(W, H, n, master, ratio, gap)
    local rects = {}
    local m = math.min(master, n)
    local s = n - m
    local master_w = 0
    if m > 0 then
        master_w = s == 0 and W or math.max(0, (W - gap) * ratio)
        local h = math.max(0, (H - (m - 1) * gap) / m)
        for i = 1, m do
            rects[i] = { 0, (i - 1) * (h + gap), master_w, h }
        end
    end
    if s > 0 then
        local start = m > 0 and (master_w + gap) or 0
        local w = math.max(0, W - start)
        local h = math.max(0, (H - (s - 1) * gap) / s)
        for i = 1, s do
            rects[m + i] = { start, (i - 1) * (h + gap), w, h }
        end
    end
    return rects
end

-- Tiling: a master column of `master_count` windows and a stack of the rest. The master takes `ratio` of the free
-- width (or height for top and bottom orientations); `gap` separates the tiles.
Workspace:extend("Tiling")({ ratio = 0.5, gap = 0, master_count = 1, orientation = "left" })
function Tiling:arrange()
    local wins = self:all("> Window")
    local n = #wins
    if n == 0 then
        return
    end
    if not ORIENTATIONS[self.orientation] then
        error("Tiling orientation must be left, right, top or bottom")
    end
    local master = math.max(0, math.floor(self.master_count))
    local ratio = clamp(self.ratio, 0.1, 0.9)
    local w0, h0 = self.width, self.height
    local W, H = w0, h0
    if self.orientation == "top" or self.orientation == "bottom" then
        W, H = h0, w0
    end
    local rects = master_stack_rects(W, H, n, master, ratio, self.gap)
    for i, w in ipairs(wins) do
        place(w, to_area(self.orientation, w0, h0, rects[i]))
    end
end

-- Monocle: the shown window fills the workspace; the rest are hidden, not moved. The shown window is the seat's
-- focus when it is a child, otherwise the first child.
Workspace:extend("Monocle")({})
function Monocle:arrange()
    local wins = self:all("> Window")
    if #wins == 0 then
        return
    end
    local shown = shown_of(wins)
    for _, w in ipairs(wins) do
        place(w, 0, 0, self.width, self.height)
        w.visible = w == shown
    end
end

-- GridLayout: a near-square grid of the windows in child order. `fill` is "row" (left to right, then down) or "column"
-- (top to bottom, then right). A partly filled last row or column is left-aligned.
Workspace:extend("GridLayout")({ fill = "row", gap = 0 })
function GridLayout:arrange()
    local wins = self:all("> Window")
    local n = #wins
    if n == 0 then
        return
    end
    if not FILLS[self.fill] then
        error("GridLayout fill must be row or column")
    end
    local side = math.ceil(math.sqrt(n))
    local cols, rows
    if self.fill == "column" then
        rows = side
        cols = math.ceil(n / rows)
    else
        cols = side
        rows = math.ceil(n / side)
    end
    local gap = self.gap
    local cell_w = math.max(0, (self.width - (cols - 1) * gap) / cols)
    local cell_h = math.max(0, (self.height - (rows - 1) * gap) / rows)
    for i, w in ipairs(wins) do
        local col, row
        if self.fill == "column" then
            col = math.floor((i - 1) / rows)
            row = (i - 1) % rows
        else
            col = (i - 1) % cols
            row = math.floor((i - 1) / cols)
        end
        place(w, col * (cell_w + gap), row * (cell_h + gap), cell_w, cell_h)
    end
end

-- Dwindle: a binary split tree in the style of Hyprland. Each new window splits the focused window's cell (or the
-- last window's cell) in two. A split keeps the direction it was created with: "horizontal" and "vertical" fix it,
-- "aspect" takes the longer side of the cell at creation. `ratio` is the share given to the first child of every
-- split, and `gap` separates the two halves of each split. The tree is kept per workspace and reconciled every
-- arrange: closed windows collapse their split, and new windows are inserted.
Workspace:extend("Dwindle")({ split = "aspect", ratio = 0.5, gap = 0 })

-- Tree nodes: a leaf is { leaf = window }; a split is { dir = "h" | "v", a = node, b = node }, with "h" putting a
-- left of b and "v" putting a above b. Weak keys: a workspace's tree goes with the workspace.
local trees = setmetatable({}, { __mode = "k" })

local function leaves(node, out)
    out = out or {}
    if node == nil then
        return out
    end
    if node.leaf ~= nil then
        out[#out + 1] = node.leaf
        return out
    end
    leaves(node.a, out)
    leaves(node.b, out)
    return out
end

-- Removes the leaf `win`; the sibling subtree takes the split's place.
local function remove(node, win)
    if node == nil then
        return nil
    end
    if node.leaf ~= nil then
        return node.leaf ~= win and node or nil
    end
    local a = remove(node.a, win)
    local b = remove(node.b, win)
    if a == nil then
        return b
    end
    if b == nil then
        return a
    end
    node.a, node.b = a, b
    return node
end

-- Replaces the leaf `target` with a split of `target` and the new leaf `win`.
local function insert(node, target, win, dir)
    if node.leaf ~= nil then
        if node.leaf == target then
            return { dir = dir, a = { leaf = target }, b = { leaf = win } }
        end
        return node
    end
    node.a = insert(node.a, target, win, dir)
    node.b = insert(node.b, target, win, dir)
    return node
end

-- Rectangles for every leaf of `tree`, keyed by window.
local function layout_tree(tree, w0, h0, gap, ratio, out)
    out = out or {}
    local function walk(node, x, y, w, h)
        if node.leaf ~= nil then
            out[node.leaf] = { x = x, y = y, w = w, h = h }
            return
        end
        if node.dir == "h" then
            local aw = math.max(0, (w - gap) * ratio)
            walk(node.a, x, y, aw, h)
            walk(node.b, x + aw + gap, y, math.max(0, w - gap - aw), h)
        else
            local ah = math.max(0, (h - gap) * ratio)
            walk(node.a, x, y, w, ah)
            walk(node.b, x, y + ah + gap, w, math.max(0, h - gap - ah))
        end
    end
    walk(tree, 0, 0, w0, h0)
    return out
end

function Dwindle:arrange()
    local wins = self:all("> Window")
    if not SPLITS[self.split] then
        error("Dwindle split must be aspect, horizontal or vertical")
    end
    local ratio = clamp(self.ratio, 0.1, 0.9)
    local gap = self.gap
    local live = {}
    for _, w in ipairs(wins) do
        live[w] = true
    end
    local tree = trees[self]
    for _, w in ipairs(leaves(tree)) do
        if not live[w] then
            tree = remove(tree, w)
        end
    end
    for _, w in ipairs(wins) do
        local present = {}
        local order = leaves(tree)
        for _, leaf in ipairs(order) do
            present[leaf] = true
        end
        if not present[w] then
            if tree == nil then
                tree = { leaf = w }
            else
                local focus = seat and seat.focus
                local target = order[#order]
                if present[focus] then
                    target = focus
                end
                local rect = layout_tree(tree, self.width, self.height, gap, ratio)[target]
                local dir = self.split
                if dir == "aspect" then
                    dir = rect.w >= rect.h and "horizontal" or "vertical"
                end
                tree = insert(tree, target, w, dir == "horizontal" and "h" or "v")
            end
        end
    end
    trees[self] = tree
    if tree == nil then
        return
    end
    local rects = layout_tree(tree, self.width, self.height, gap, ratio)
    for _, w in ipairs(wins) do
        local r = rects[w]
        place(w, r.x, r.y, r.w, r.h)
    end
end

-- Kiosk: one window at a time, full area. The shown window is the seat's focus when it is a child, otherwise the
-- first child. Every other window is hidden, and stays a child, so it is still tiled when shown again.
Workspace:extend("Kiosk")({})
function Kiosk:arrange()
    local wins = self:all("> Window")
    if #wins == 0 then
        return
    end
    local shown = shown_of(wins)
    for _, w in ipairs(wins) do
        place(w, 0, 0, self.width, self.height)
        w.visible = w == shown
    end
end
