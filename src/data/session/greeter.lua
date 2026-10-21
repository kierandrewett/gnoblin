-- Default greeter configuration for `gnoblin --greeter`.
--
-- Login managers such as greetd pass their own file with `--config`, so this
-- file is only read when no `--config` is given and /etc/gnoblin/greeter.lua
-- does not exist. It starts no autostart entries.
--
-- Autostart children of the greeter receive GREETD_SOCK from the login
-- manager, which they use to talk to greetd. GNOBLIN_SESSION_CLASS is set to
-- "greeter" for them.
return {}
