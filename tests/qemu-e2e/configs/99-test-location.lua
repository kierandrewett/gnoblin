-- Turns the location policy on and allows every request, so a test can check the GeoClue agent path.
-- location.enabled must be set here or in org.gnome.system.location. Without it the agent denies all requests.
gnoblin.configure({ location = { enabled = true, max_accuracy = "city" } })

local function log(text)
    gnoblin.process.spawn({ "sh", "-c", "echo \"$1\" >> /tmp/location-events.log", "sh", text })
end

gnoblin.on("gnoblin.location.authorization-requested", function(event)
    local request = event.request
    log("requested id=" .. request.id .. " app=" .. request.app_id .. " accuracy=" .. request.requested_accuracy)
    request:answer(true)
end)
