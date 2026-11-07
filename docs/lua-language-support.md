# Lua language and library support

This page is generated from `crates/gnoblin-lua-sandbox/tests/conformance.rs`. Regenerate it with `GNOBLIN_WRITE_LUA_DOCS=1 cargo test -p gnoblin-lua-sandbox --test conformance`.

A yes means the probe snippet returns `true` in that profile. The sandbox removes `io`, `package`, `debug`, `require`, `dofile` and `loadfile` in both profiles, and removes `os.execute`, `os.remove`, `os.rename`, `os.getenv`, `os.tmpname` and `os.exit` in both profiles. This is the only deliberate exception to full Lua: those functions reach the host system, which is the sandbox boundary. Coroutines inside `arrange` are control flow, not async scheduling: the compositor does not resume them across frames, and the same budget bounds them.

| Feature                          | Config | Layout |
| -------------------------------- | ------ | ------ |
| integer and float subtypes       | yes    | yes    |
| floor division //                | yes    | yes    |
| bitwise operators                | yes    | yes    |
| goto                             | yes    | yes    |
| const attribute                  | yes    | yes    |
| close attribute                  | yes    | no     |
| varargs and table.pack           | yes    | yes    |
| arithmetic metamethods           | yes    | no     |
| concat and len metamethods       | yes    | no     |
| call metamethod                  | yes    | no     |
| __gc accepted                    | yes    | no     |
| closures                         | yes    | yes    |
| multiple assignment              | yes    | yes    |
| string methods via metatable     | yes    | yes    |
| string.upper                     | yes    | no     |
| integer for-loop at maxinteger   | yes    | yes    |
| utf8 escapes in strings          | yes    | yes    |
| pcall                            | yes    | yes    |
| xpcall                           | yes    | yes    |
| error and assert                 | yes    | yes    |
| select, type, tostring, tonumber | yes    | yes    |
| next, pairs, ipairs              | yes    | yes    |
| rawget                           | yes    | no     |
| getmetatable and setmetatable    | yes    | no     |
| load of text                     | yes    | no     |
| load refuses binary              | yes    | no     |
| collectgarbage                   | yes    | no     |
| print                            | yes    | no     |
| coroutine library                | yes    | yes    |
| os.clock absent in layout        | no     | yes    |
| os host-reaching functions       | yes    | yes    |
| io                               | yes    | yes    |
| debug                            | yes    | yes    |
| package and require              | yes    | yes    |
| dofile and loadfile              | yes    | yes    |
| string.find                      | yes    | no     |
| string.match                     | yes    | no     |
| string.gmatch                    | yes    | no     |
| string.gsub                      | yes    | no     |
| string.rep                       | yes    | no     |
| string.format bounded            | yes    | yes    |
| string.format wide width         | no     | no     |
| table.insert, remove, concat     | yes    | yes    |
| table.move                       | yes    | yes    |
| table.unpack                     | yes    | yes    |
| table.sort                       | yes    | yes    |
| math.floor, math.huge            | yes    | yes    |
| math.random                      | yes    | no     |
| utf8.len                         | yes    | no     |
