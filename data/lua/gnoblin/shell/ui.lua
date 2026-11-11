-- data/lua/gnoblin/shell/ui.lua: the stock widgets. Each is a Box, as each HTML element is a box.
Canvas:extend("Box")({
    fill = "#00000000",
    radius = 0,
    direction = "none",
    gap = 0,
    padding = 0,
    grow = 0,
    draw = function(b)
        return { { rect = { 0, 0, b.width, b.height }, radius = b.radius, fill = b.fill } }
    end,
})
function Box:arrange() -- "row" or "column": a small flexbox; "none": children stay put
    if self.direction == "none" then
        return
    end
    local main, size, cross, across = "x", "width", "y", "height"
    if self.direction == "column" then
        main, size, cross, across = "y", "height", "x", "width"
    end
    local kids, used, grow, pad = self:all("> Box"), 0, 0, self.padding
    for _, c in ipairs(kids) do
        if c.grow > 0 then
            grow = grow + c.grow
        else
            used = used + c[size]
        end
    end
    local free = self[size] - 2 * pad - self.gap * math.max(#kids - 1, 0) - used
    local at = pad
    for _, c in ipairs(kids) do
        if c.grow > 0 then
            c[size] = math.max(0, free * c.grow // grow)
        end
        c[main], c[cross], c[across] = at, pad, self[across] - 2 * pad
        at = at + c[size] + self.gap
    end
end

-- Text width is what the compositor measures. `Canvas:measure` asks it for the width the painter will draw the text
-- at, and the answer arrives a frame later, so until it does the width is an estimate: a sans glyph averages about
-- half the font size. The answer is kept with the text, size and font it was for, so a change of any of them
-- falls back to the estimate and asks again.
local asked = setmetatable({}, { __mode = "k" }) -- node -> the key its outstanding question was about
local function estimate(t)
    return (utf8.len(t.text or "") or #(t.text or "")) * t.size // 2
end
local function measured_width(t)
    local key = ("%s\1%s\1%s"):format(t.font, t.size, t.text)
    local known = t.measured
    if known.key == key then
        return known.width
    end
    if asked[t] ~= key then
        asked[t] = key
        local answer = t:measure(t.text, t.size, t.font)
        go(function()
            local result, err = await(answer)
            if asked[t] ~= key then
                return -- the text changed meanwhile and a newer question is out
            end
            local width = tostring(result):match("^(%d+) %d+$")
            -- An error keeps the estimate for this text: asking again would get the same refusal.
            t.measured = { key = key, width = width and tonumber(width) or estimate(t) }
            if err ~= "" then
                report(("Text %q: measure failed: %s"):format(t.text, err))
            end
        end)
    end
    return estimate(t)
end
Box:extend("Text")({
    text = "",
    size = 14,
    color = "#cdd6f4",
    font = "sans",
    measured = {},
    width = measured_width,
    height = function(t)
        return t.size * 3 // 2
    end,
    -- The line box is centred in the node, so a row that squeezes the node shorter than a line (a titlebar) trims
    -- both ends of the box evenly and not just the bottom.
    draw = function(t)
        local y = (t.height - t.size * 3 // 2) // 2
        return { { text = t.text, y = y, size = t.size, color = t.color, font = t.font } }
    end,
})

Box:extend("Icon")({
    press = signal,
    icon = "",
    size = 16,
    color = "#cdd6f4",
    width = function(i)
        return i.size
    end,
    height = function(i)
        return i.size
    end,
    draw = function(i)
        return { { icon = i.icon, size = i.size, color = i.color } }
    end,
})

Box:extend("Grid")({})
function Grid:arrange() -- children in a near-square grid
    local kids, pad, gap = self.children, self.padding, self.gap
    local cols = math.max(1, math.ceil(math.sqrt(#kids)))
    local rows = math.max(1, math.ceil(#kids / cols))
    local w = (self.width - 2 * pad - (cols - 1) * gap) // cols
    local h = (self.height - 2 * pad - (rows - 1) * gap) // rows
    for i, c in ipairs(kids) do
        c.x, c.y = pad + (i - 1) % cols * (w + gap), pad + (i - 1) // cols * (h + gap)
        c.width, c.height = w, h
    end
end

Box:extend("Bar")({ edge = "top", height = 32, fill = "#1e1e2e", padding = 6, gap = 12, direction = "row" })

Text:extend("Clock")({ format = "%H:%M" })
function Clock:on_ready()
    every(1, function()
        self.text = date(self.format)
    end)
end

Box:extend("Entry")({
    text = "",
    secret = false,
    size = 14,
    padding = 12,
    direction = "row",
    input = true,
    submit = signal,
})
function Entry:build()
    return {
        Text({
            grow = 1,
            size = self.size,
            text = function()
                return self.secret and ("*"):rep(utf8.len(self.text)) or self.text
            end,
        }),
    }
end
function Entry:on_key(e)
    if e.key == "Return" then
        self.submit:emit(self.text)
    elseif e.key == "BackSpace" then
        self.text = self.text:sub(1, (utf8.offset(self.text, -1) or 1) - 1)
    elseif e.text then
        self.text = self.text .. e.text
    end
end
