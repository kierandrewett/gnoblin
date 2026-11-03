local function log(text)
    gnoblin.process.spawn({ "sh", "-c", "echo \"$1\" >> /tmp/process-events.log", "sh", text })
end

local function describe(value)
    if type(value) == "table" then
        return tostring(value.message or value.code or value)
    end
    return tostring(value)
end

local steps = {
    { "echo", { "sh", "-c", "echo hello" }, { stdout = "capture" } },
    { "exit3", { "sh", "-c", "exit 3" }, { stdout = "capture" } },
    { "stdin", { "cat" }, { stdin = "from-stdin", stdout = "capture" } },
    { "timeout", { "sleep", "30" }, { timeout = 2 } },
    { "big", { "sh", "-c", "head -c 100000 /dev/zero | tr '\\0' a" }, { stdout = "capture" } },
    { "missing", { "gnoblin-no-such-program" } },
    { "signal", { "sh", "-c", "kill -TERM $$" } },
}

local function run_step(index)
    local step = steps[index]
    if not step then
        log("all steps done")
        return
    end
    gnoblin.process.spawn(step[2], step[3]):wait():on_complete(function(result, err)
        if err then
            log(step[1] .. " error=" .. describe(err))
        else
            local stdout = result.stdout and result.stdout:gsub("\n", "\\n") or ""
            log(
                step[1]
                    .. " exit="
                    .. tostring(result.exit_code)
                    .. " signal="
                    .. tostring(result.signal)
                    .. " timed_out="
                    .. tostring(result.timed_out)
                    .. " bytes="
                    .. #(result.stdout or "")
                    .. " truncated="
                    .. tostring(result.stdout_truncated)
                    .. " stdout=["
                    .. stdout:sub(1, 40)
                    .. "]"
            )
        end
        run_step(index + 1)
    end)
end

gnoblin.on("gnoblin.config.reloaded", function(event)
    run_step(1)
end)
