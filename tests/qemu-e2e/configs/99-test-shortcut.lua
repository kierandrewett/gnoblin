local function log(text)
    gnoblin.commands.run({"sh", "-c", 'echo "$1" >> /tmp/shortcut-events.log', "sh", text})
end

gnoblin.on("gnoblin.shortcut.binding-activated", function(event)
    log("activated id=" .. tostring(event.id) .. " accelerator=" .. tostring(event.accelerator))
end)

gnoblin.on("gnoblin.shortcut.binding-deactivated", function(event)
    log("deactivated id=" .. tostring(event.id))
end)

gnoblin.on("gnoblin.config.reloaded", function(event)
    gnoblin.shortcuts.bind {id = "regression-shortcut", accelerator = "<Super><Alt>F9"}
    log("bound regression-shortcut")
end)
