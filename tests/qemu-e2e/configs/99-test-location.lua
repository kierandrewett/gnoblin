-- Turns the location policy on and allows every request, so a test can check the GeoClue agent path.
-- location.enabled must be set here or in org.gnome.system.location. Without it the agent denies all requests.
gnoblin.configure {location = {enabled = true, max_accuracy = "city"}}

local function log(text)
    gnoblin.commands.run({"sh", "-c", 'echo "$1" >> /tmp/location-events.log', "sh", text})
end

gnoblin.on("gnoblin.location.authorization-requested", function(event)
    log("requested id=" .. event.request_id .. " app=" .. event.app_id .. " accuracy=" .. event.requested_accuracy)
    gnoblin.location.authorize_app {request_id = event.request_id, allow = true, accuracy = event.requested_accuracy}
end)
