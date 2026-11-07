local function log(text)
    gnoblin.process.spawn({ "sh", "-c", "echo \"$1\" >> /tmp/prompt-events.log", "sh", text })
end

gnoblin.on("gnoblin.prompt.requested", function(event)
    local request = event.request
    log(
        "requested id="
            .. request.id
            .. " kind="
            .. request.kind
            .. " title=["
            .. tostring(request.title)
            .. "] message=["
            .. tostring(request.message)
            .. "] new="
            .. tostring(request.password_new)
    )
    request:finished():on_complete(function(result)
        log("finished id=" .. request.id .. " result=" .. result)
    end)
    if request.kind == "password" then
        request:answer("gnoblin-keyring-test")
    else
        request:answer(true)
    end
end)
