local function log(text)
    gnoblin.commands.run({"sh", "-c", 'echo "$1" >> /tmp/capture-events.log', "sh", text})
end

local function describe(value)
    if type(value) == "table" then
        return tostring(value.message or value.code or value)
    end
    return tostring(value)
end

local steps = {
    {"echo", {argv = {"sh", "-c", "echo hello"}}},
    {"exit3", {argv = {"sh", "-c", "exit 3"}}},
    {"stdin", {argv = {"cat"}, stdin = "from-stdin"}},
    {"timeout", {argv = {"sleep", "30"}, timeout = 2}},
    {"big", {argv = {"sh", "-c", "head -c 100000 /dev/zero | tr '\\0' a"}}},
    {"missing", {argv = {"gnoblin-no-such-program"}}},
    {"signal", {argv = {"sh", "-c", "kill -TERM $$"}}},
}

local function run_step(index)
    local step = steps[index]
    if not step then
        log("all steps done")
        return
    end
    gnoblin.commands.capture(step[2]):on_complete(function(result, err)
        if err then
            log(step[1] .. " error=" .. describe(err))
        else
            log(step[1] .. " exit=" .. result.exit_code .. " stdout=[" .. result.stdout:gsub("\n", "\\n") .. "]")
        end
        run_step(index + 1)
    end)
end

gnoblin.on("gnoblin.config.reloaded", function(event)
    run_step(1)
end)
