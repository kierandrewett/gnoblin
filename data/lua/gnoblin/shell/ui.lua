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

-- Text metrics are what the compositor measures. `Canvas:measure` asks it for the width, height and ascent the
-- painter will lay the text out in, and the answer arrives a frame later, so until it does the metrics are an
-- estimate: a sans glyph averages about half the font size, a line box is one and a half sizes tall and the baseline
-- sits a little over one size below its top. The answer is kept with the text, size and font it was for, so a change
-- of any of them falls back to the estimate and asks again.
local asked = setmetatable({}, { __mode = "k" }) -- node -> the key its outstanding question was about
local function estimate(t)
    local text, widest, lines = t.text or "", 0, 0
    for line in (text .. "\n"):gmatch("(.-)\n") do
        widest = math.max(widest, (utf8.len(line) or #line) * t.size // 2)
        lines = lines + 1
    end
    return { width = widest, height = lines * (t.size * 3 // 2), ascent = t.size * 11 // 10 }
end
local function metrics(t)
    local key = ("%s\1%s\1%s"):format(t.font, t.size, t.text)
    local known = t.measured
    if known.key == key then
        return known
    end
    if asked[t] ~= key then
        asked[t] = key
        local answer = t:measure(t.text, t.size, t.font)
        go(function()
            local result, err = await(answer)
            if asked[t] ~= key then
                return -- the text changed meanwhile and a newer question is out
            end
            local width, height, ascent = tostring(result):match("^(%d+) (%d+) (%d+)$")
            -- An error keeps the estimate for this text: asking again would get the same refusal.
            local taken = estimate(t)
            if width then
                taken = { width = tonumber(width), height = tonumber(height), ascent = tonumber(ascent) }
            end
            taken.key = key
            t.measured = taken
            if err ~= "" then
                report(("Text %q: measure failed: %s"):format(t.text, err))
            end
        end)
    end
    return estimate(t)
end

-- The text a Text draws. With `ellipsize` a text wider than the room its row gave it is cut to the share that fits,
-- by the measured width per character, and ends in an ellipsis; the row never has to know how wide a title is.
local function shown(t)
    local m = metrics(t)
    local chars = utf8.len(t.text)
    if not t.ellipsize or not chars or t.width >= m.width then
        return t.text
    end
    local keep = math.max(0, chars * t.width // m.width - 1)
    return t.text:sub(1, (utf8.offset(t.text, keep + 1) or #t.text + 1) - 1) .. "…"
end

Box:extend("Text")({
    text = "",
    size = 14,
    color = "#cdd6f4",
    font = "sans",
    measured = {},
    ellipsize = false,
    width = function(t)
        return metrics(t).width
    end,
    height = function(t)
        return metrics(t).height
    end,
    -- Pixels from the top of the first line to its baseline: what two texts of different sizes, or a text and an
    -- icon, are lined up by.
    ascent = function(t)
        return metrics(t).ascent
    end,
    -- The text block is centred in the node, so a row that squeezes the node shorter than the block (a titlebar)
    -- trims both ends evenly and not just the bottom.
    draw = function(t)
        local y = (t.height - metrics(t).height) // 2
        return { { text = shown(t), y = y, size = t.size, color = t.color, font = t.font } }
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
    if e.pressed == false then -- the key coming up types nothing
        return
    end
    if e.key == "Return" then
        self.submit:emit(self.text)
    elseif e.key == "BackSpace" then
        self.text = self.text:sub(1, (utf8.offset(self.text, -1) or 1) - 1)
    elseif e.text and e.text ~= "" then
        self.text = self.text .. e.text
    end
end
