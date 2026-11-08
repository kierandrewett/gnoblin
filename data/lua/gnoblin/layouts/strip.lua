-- strip: an ordered strip of columns, in the style of niri. Each column holds one
-- or more windows, stacked top to bottom, or tabbed (only the active window shows).
-- The strip is wider than the area; a view offset decides which part is on screen.
-- Columns that fall outside the view keep their placements, with x outside the area.
--
-- Gaps: `outer` insets the area, `inner` separates columns and stacked windows.
--
-- State (ctx.state): columns = array of { windows = {id...}, fraction = 0..1 or px = n,
-- tabbed = bool, active = id }, view = strip offset in pixels, pinned = bool, focus_seen = id.
-- Reconciliation runs on every arrange: closed windows leave their column, empty
-- columns vanish, and a new window opens as a new column right after the focused
-- column (or at the end when nothing is focused).
--
-- arrange: columns are laid out left to right, each at its width (fraction of the
--          area width, or px). The view follows the focused column under `centre`,
--          until scroll_to pins it.
-- on_request: declines every geometry request.
-- options:
--   column_width  width of new columns as a fraction of the area, 0..1 (default 0.5)
--   centre        "on-overflow" (default): centre the focused column only when it is
--                 not fully visible. "always": always centre it. "never": scroll the
--                 minimum needed to bring it into view.
-- commands (args in parentheses; ctx.focus is the window they act on):
--   width(preset | px): set the focused column width. preset is "1/3", "1/2", "2/3"
--                 or "1"; px is a fixed width in pixels.
--   consume(window?): move the window (default focused) to the end of the column on
--                 its left. No-op for the first column.
--   expel(window?): move the window out of its column into a new column on its right.
--                 No-op when it is alone in its column.
--   move_column(direction): swap the focused column with its neighbour, "left" or
--                 "right". No wrap.
--   fit_all:      give every column an equal share of the area and scroll to the start.
--   toggle_tabbed: switch the focused column between stacked and tabbed.
--   focus(direction): "left", "right", "up" or "down". Returns the arrangement with the
--                 neighbouring window as the focus target, plus a focus effect that
--                 names it. The compositor focuses that window; when there is no
--                 neighbour the arrangement is returned alone.
--   scroll_to(x): set the view offset to x pixels and keep it there until the focus
--                 changes. The view is clamped to the strip, so an x past either end
--                 stops at that end.
--
-- Usage: require("gnoblin.layouts.strip"):with({ column_width = 0.5 })

local M = {}
M.kind = "tiling"
M.extent = "unbounded"

M.options = {
    column_width = 0.5,
    centre = "on-overflow",
}

local PRESETS = { ["1/3"] = 1 / 3, ["1/2"] = 1 / 2, ["2/3"] = 2 / 3, ["1"] = 1 }

local function clamp(value, low, high)
    return math.max(low, math.min(high, value))
end

local function locate(columns, id)
    if id == nil then
        return nil, nil
    end
    for ci, column in ipairs(columns) do
        for wi, window in ipairs(column.windows) do
            if window == id then
                return ci, wi
            end
        end
    end
    return nil, nil
end

local function reconcile(ctx)
    local state = ctx.state
    local live = {}
    for _, window in ipairs(ctx.windows) do
        live[window.id] = true
    end

    local columns = {}
    local seen = {}
    for _, column in ipairs(state.columns or {}) do
        local kept = {}
        for _, id in ipairs(column.windows) do
            if live[id] and not seen[id] then
                kept[#kept + 1] = id
                seen[id] = true
            end
        end
        if #kept > 0 then
            column.windows = kept
            local active_kept = false
            for _, id in ipairs(kept) do
                if id == column.active then
                    active_kept = true
                end
            end
            if not active_kept then
                column.active = nil
            end
            columns[#columns + 1] = column
        end
    end

    for _, window in ipairs(ctx.windows) do
        if not seen[window.id] then
            local ci = locate(columns, ctx.focus)
            local at = ci and ci + 1 or #columns + 1
            table.insert(columns, at, {
                windows = { window.id },
                fraction = clamp(ctx.params.column_width, 0.1, 1),
            })
            seen[window.id] = true
        end
    end

    state.columns = columns
    return columns
end

local function column_px(column, area_width)
    if column.px ~= nil then
        return math.max(0, column.px)
    end
    return math.max(0, (column.fraction or 0.5) * area_width)
end

-- Left edge of each column in strip coordinates, plus the strip's total length.
local function geometry(columns, area_width, inner)
    local geo = {}
    local x = 0
    for ci, column in ipairs(columns) do
        local w = column_px(column, area_width)
        geo[ci] = { x = x, w = w }
        x = x + w + inner
    end
    local total = #geo > 0 and (geo[#geo].x + geo[#geo].w) or 0
    return geo, total
end

-- Moves the view as the centre policy requires for the focused column.
local function follow(ctx, columns, geo, total, vw)
    local state = ctx.state
    if ctx.focus ~= state.focus_seen then
        state.pinned = false
        state.focus_seen = ctx.focus
    end
    local ci = locate(columns, ctx.focus)
    local view = state.view or 0
    if ci ~= nil and not state.pinned then
        local x, w = geo[ci].x, geo[ci].w
        local policy = ctx.params.centre
        local centred = x + w / 2 - vw / 2
        if policy == "always" then
            view = centred
        elseif policy == "never" then
            if x < view or w >= vw then
                view = x
            elseif x + w > view + vw then
                view = x + w - vw
            end
        elseif policy == "on-overflow" then
            if not (x >= view and x + w <= view + vw) then
                view = centred
            end
        else
            error("strip centre must be always, never or on-overflow")
        end
    end
    local max_view = math.max(0, total - vw)
    state.view = clamp(view, 0, max_view)
end

function M.arrange(ctx)
    local area = ctx.area
    local outer = ctx.params.outer or 0
    local inner = ctx.params.inner or 0
    local columns = reconcile(ctx)

    local x0, y0 = area.x + outer, area.y + outer
    local vw = math.max(0, area.width - 2 * outer)
    local H = math.max(0, area.height - 2 * outer)

    local geo, total = geometry(columns, vw, inner)

    -- A focused window in a tabbed column becomes that column's active tab.
    local fci, fwi = locate(columns, ctx.focus)
    if fci ~= nil and columns[fci].tabbed then
        columns[fci].active = columns[fci].windows[fwi]
    end

    follow(ctx, columns, geo, total, vw)
    local view = ctx.state.view

    local placements = {}
    for ci, column in ipairs(columns) do
        local X = x0 + geo[ci].x - view
        local W = geo[ci].w
        if column.tabbed then
            local active = column.active or column.windows[1]
            for _, id in ipairs(column.windows) do
                placements[#placements + 1] = {
                    window = id,
                    x = X,
                    y = y0,
                    width = W,
                    height = H,
                    visible = id == active,
                }
            end
        else
            local n = #column.windows
            local h = math.max(0, (H - (n - 1) * inner) / n)
            for k, id in ipairs(column.windows) do
                placements[#placements + 1] = {
                    window = id,
                    x = X,
                    y = y0 + (k - 1) * (h + inner),
                    width = W,
                    height = h,
                }
            end
        end
    end
    return { placements = placements }
end

function M.on_request(_ctx)
    return nil
end

local function target_of(ctx, args)
    args = args or {}
    return args.window or ctx.focus
end

M.command = {
    width = function(ctx, args)
        args = args or {}
        local columns = reconcile(ctx)
        local ci = locate(columns, ctx.focus)
        if ci == nil then
            return
        end
        local column = columns[ci]
        if args.px ~= nil then
            column.px = math.max(1, args.px)
            column.fraction = nil
        else
            local fraction = PRESETS[args.preset]
            if fraction == nil then
                error("strip width preset must be 1/3, 1/2, 2/3 or 1")
            end
            column.fraction = fraction
            column.px = nil
        end
    end,

    consume = function(ctx, args)
        local columns = reconcile(ctx)
        local id = target_of(ctx, args)
        local ci, wi = locate(columns, id)
        if ci == nil or ci == 1 then
            return
        end
        local source = columns[ci]
        table.remove(source.windows, wi)
        if source.active == id then
            source.active = nil
        end
        local dest = columns[ci - 1]
        dest.windows[#dest.windows + 1] = id
        if #source.windows == 0 then
            table.remove(columns, ci)
        end
    end,

    expel = function(ctx, args)
        local columns = reconcile(ctx)
        local id = target_of(ctx, args)
        local ci, wi = locate(columns, id)
        if ci == nil or #columns[ci].windows == 1 then
            return
        end
        local source = columns[ci]
        table.remove(source.windows, wi)
        if source.active == id then
            source.active = nil
        end
        table.insert(columns, ci + 1, {
            windows = { id },
            fraction = source.fraction,
            px = source.px,
        })
    end,

    move_column = function(ctx, args)
        args = args or {}
        local columns = reconcile(ctx)
        local ci = locate(columns, ctx.focus)
        if ci == nil then
            return
        end
        local other
        if args.direction == "left" then
            other = ci - 1
        elseif args.direction == "right" then
            other = ci + 1
        else
            error("strip move_column direction must be left or right")
        end
        if other >= 1 and other <= #columns then
            columns[ci], columns[other] = columns[other], columns[ci]
        end
    end,

    fit_all = function(ctx, _args)
        local columns = reconcile(ctx)
        local n = #columns
        for _, column in ipairs(columns) do
            column.px = nil
            column.fraction = 1 / n
        end
        ctx.state.view = 0
    end,

    toggle_tabbed = function(ctx, _args)
        local columns = reconcile(ctx)
        local ci, wi = locate(columns, ctx.focus)
        if ci == nil then
            return
        end
        local column = columns[ci]
        column.tabbed = not column.tabbed
        column.active = column.windows[wi]
    end,

    focus = function(ctx, args)
        args = args or {}
        local columns = reconcile(ctx)
        local ci, wi = locate(columns, ctx.focus)
        if ci == nil then
            return M.arrange(ctx)
        end
        local column = columns[ci]
        local direction = args.direction
        local target
        if direction == "up" or direction == "down" then
            local step = direction == "up" and -1 or 1
            target = column.windows[wi + step]
        elseif direction == "left" or direction == "right" then
            local neighbour = columns[ci + (direction == "left" and -1 or 1)]
            if neighbour ~= nil then
                if neighbour.tabbed and neighbour.active ~= nil then
                    target = neighbour.active
                else
                    target = neighbour.windows[1]
                end
            end
        else
            error("strip focus direction must be left, right, up or down")
        end
        if target == nil then
            return M.arrange(ctx)
        end
        -- The arrangement is taken with the target focused, so the view follows it and
        -- the target is placed visibly, as the focus effect requires.
        local moved = setmetatable({ focus = target }, { __index = ctx })
        local reply = M.arrange(moved)
        reply.effects = { { kind = "focus", window = target } }
        return reply
    end,

    scroll_to = function(ctx, args)
        args = args or {}
        local x = args.x
        if type(x) ~= "number" or x ~= x then
            error("strip scroll_to needs a number x, in pixels")
        end
        ctx.state.view = x
        ctx.state.pinned = true
        ctx.state.focus_seen = ctx.focus
    end,
}

-- A command that returns nothing answers with the arrangement it left, so the host
-- applies it. A command that returns a reply table of its own keeps that reply.
for name, command in pairs(M.command) do
    M.command[name] = function(ctx, args)
        local reply = command(ctx, args)
        if type(reply) == "table" then
            return reply
        end
        return M.arrange(ctx)
    end
end

return M
