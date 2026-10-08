-- Logs each gnoblin.monitor.changed event with its changed list, so a test can see work_area changes.
gnoblin.on("gnoblin.monitor.changed", function(event)
    local changed = table.concat(event.changed or {}, ",")
    gnoblin.commands.run({"sh", "-c", 'echo "$1" >> /tmp/workarea-events.log', "sh", "changed=" .. changed})
end)
