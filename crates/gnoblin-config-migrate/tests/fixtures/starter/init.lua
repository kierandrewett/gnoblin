-- Gnoblin's minimal default config, embedded in the binary.
-- Edit these files after gnoblinctl init; no files are created during login.
-- gnoblin.load runs each matching file in place; use require for a module that returns a value.
gnoblin.load("config/*.lua")
