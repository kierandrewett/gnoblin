-- dwindle: a binary split tree, in the style of Hyprland's dwindle layout. Each new
-- window splits the focused window's cell in two. Gaps: `outer` insets the area,
-- `inner` separates the two halves of every split.
--
-- The tree lives in ctx.state.tree. A leaf is { leaf = window_id }; an internal node
-- is { dir = "h" | "v", ratio = share_of_a, pinned = bool, a = node, b = node }, where
-- "h" puts a left of b and "v" puts a above b. Reconciliation runs on every arrange:
-- closed windows collapse their parent split, and new windows split the focused cell
-- (or the last leaf when nothing focused is tiled).
--
-- arrange: every window gets its leaf's rectangle; no window is hidden.
-- on_request: declines every geometry request.
-- options:
--   split          "aspect" (split the longer side, default), "horizontal" (always
--                  side by side) or "vertical" (always stacked) for new splits.
--   preserve_split false (default): each split's direction follows its rectangle's
--                  aspect at every arrange, unless pinned by rotate. true: each
--                  split keeps the direction it was created with.
--   ratio          share of the split given to the first child, 0.1..0.9 (0.5).
-- commands:
--   swap           swap the focused window with args.window, or with its sibling
--                  when that is a single window.
--   rotate         flip the direction of the focused window's split and pin it.
--   resize_ratio   args.delta: move the focused window's split boundary (grows the
--                  focused side).
--
-- Usage: gnoblin.layout.register("dwindle", "gnoblin.layouts.dwindle")

local M = {}
M.kind = "tiling"

M.options = {
    split = "aspect",
    preserve_split = false,
    ratio = 0.5,
}

local SPLITS = { aspect = true, horizontal = true, vertical = true }

local function clamp(value, low, high)
    return math.max(low, math.min(high, value))
end

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

-- Removes the leaf `id`; the sibling subtree takes the parent's place.
local function remove(node, id)
    if node == nil then
        return nil
    end
    if node.leaf ~= nil then
        if node.leaf == id then
            return nil
        end
        return node
    end
    local a = remove(node.a, id)
    local b = remove(node.b, id)
    if a == nil then
        return b
    end
    if b == nil then
        return a
    end
    node.a, node.b = a, b
    return node
end

-- Replaces the leaf `target` with a split holding `target` and the new leaf `id`.
local function insert(node, target, id, dir, ratio)
    if node.leaf ~= nil then
        if node.leaf == target then
            return { dir = dir, ratio = ratio, a = { leaf = target }, b = { leaf = id } }
        end
        return node
    end
    node.a = insert(node.a, target, id, dir, ratio)
    node.b = insert(node.b, target, id, dir, ratio)
    return node
end

-- The internal node whose direct child is the leaf `id`.
local function parent_of(node, id)
    if node == nil or node.leaf ~= nil then
        return nil
    end
    if node.a.leaf == id or node.b.leaf == id then
        return node
    end
    return parent_of(node.a, id) or parent_of(node.b, id)
end

local function relabel(node, a, b)
    if node == nil then
        return
    end
    if node.leaf ~= nil then
        if node.leaf == a then
            node.leaf = b
        elseif node.leaf == b then
            node.leaf = a
        end
        return
    end
    relabel(node.a, a, b)
    relabel(node.b, a, b)
end

local function place(node, x, y, w, h, inner, preserve, out)
    if node.leaf ~= nil then
        out[node.leaf] = { x = x, y = y, width = w, height = h }
        return
    end
    local dir = node.dir
    if not (preserve or node.pinned) or dir == nil then
        dir = (w >= h) and "h" or "v"
    end
    local ratio = clamp(node.ratio, 0.1, 0.9)
    if dir == "h" then
        local aw = math.max(0, (w - inner) * ratio)
        local bw = math.max(0, w - inner - aw)
        place(node.a, x, y, aw, h, inner, preserve, out)
        place(node.b, x + aw + inner, y, bw, h, inner, preserve, out)
    else
        local ah = math.max(0, (h - inner) * ratio)
        local bh = math.max(0, h - inner - ah)
        place(node.a, x, y, w, ah, inner, preserve, out)
        place(node.b, x, y + ah + inner, w, bh, inner, preserve, out)
    end
end

-- Rectangles for every leaf of `tree`, keyed by window id.
local function layout_tree(ctx, tree)
    local out = {}
    if tree == nil then
        return out
    end
    local area = ctx.area
    local outer = ctx.params.outer or 0
    place(
        tree,
        area.x + outer,
        area.y + outer,
        math.max(0, area.width - 2 * outer),
        math.max(0, area.height - 2 * outer),
        ctx.params.inner or 0,
        ctx.params.preserve_split,
        out
    )
    return out
end

local function direction_for(ctx, rect)
    local split = ctx.params.split
    if not SPLITS[split] then
        error("dwindle split must be aspect, horizontal or vertical")
    end
    if split == "horizontal" then
        return "h"
    end
    if split == "vertical" then
        return "v"
    end
    return (rect.width >= rect.height) and "h" or "v"
end

local function reconcile(ctx)
    local live = {}
    for _, window in ipairs(ctx.windows) do
        live[window.id] = true
    end

    local tree = ctx.state.tree
    for _, id in ipairs(leaves(tree)) do
        if not live[id] then
            tree = remove(tree, id)
        end
    end

    for _, window in ipairs(ctx.windows) do
        local id = window.id
        local present = {}
        local order = leaves(tree)
        for _, leaf in ipairs(order) do
            present[leaf] = true
        end
        if not present[id] then
            if tree == nil then
                tree = { leaf = id }
            else
                local target = order[#order]
                if ctx.focus ~= nil and present[ctx.focus] then
                    target = ctx.focus
                end
                local rect = layout_tree(ctx, tree)[target]
                local ratio = clamp(ctx.params.ratio, 0.1, 0.9)
                tree = insert(tree, target, id, direction_for(ctx, rect), ratio)
            end
        end
    end

    ctx.state.tree = tree
    return tree
end

function M.arrange(ctx)
    local tree = reconcile(ctx)
    local rects = layout_tree(ctx, tree)
    local placements = {}
    for index, window in ipairs(ctx.windows) do
        local rect = rects[window.id]
        placements[index] = {
            window = window.id,
            x = rect.x,
            y = rect.y,
            width = rect.width,
            height = rect.height,
        }
    end
    return { placements = placements }
end

function M.on_request(_ctx)
    return nil
end

M.command = {
    swap = function(ctx, args)
        args = args or {}
        local tree = reconcile(ctx)
        local focus = ctx.focus
        if focus == nil then
            return
        end
        local other = args.window
        if other == nil then
            local parent = parent_of(tree, focus)
            if parent == nil then
                return
            end
            local sibling = parent.a.leaf == focus and parent.b or parent.a
            other = sibling.leaf
        end
        if other == nil or other == focus then
            return
        end
        relabel(tree, focus, other)
    end,

    rotate = function(ctx, _args)
        local tree = reconcile(ctx)
        local parent = ctx.focus ~= nil and parent_of(tree, ctx.focus) or nil
        if parent == nil then
            return
        end
        parent.dir = (parent.dir == "h") and "v" or "h"
        parent.pinned = true
    end,

    resize_ratio = function(ctx, args)
        args = args or {}
        local tree = reconcile(ctx)
        local parent = ctx.focus ~= nil and parent_of(tree, ctx.focus) or nil
        if parent == nil then
            return
        end
        local delta = args.delta or 0
        if parent.a.leaf == ctx.focus then
            parent.ratio = clamp(parent.ratio + delta, 0.1, 0.9)
        else
            parent.ratio = clamp(parent.ratio - delta, 0.1, 0.9)
        end
    end,
}

return M
