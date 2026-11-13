# Shell object model

The shell is a tree of objects. The root holds the screens, and each screen holds
its desks, workspaces, windows and panels. Every object is a node.

A node has an identity, a lifetime, properties you read and write, signals it
sends, and handlers that run when it receives an event. Your shell files work with
these objects; they do not call the compositor directly.

This is the same model as the DOM in a browser or the object tree in Godot. It
is not a set of separate APIs for each kind of window.

## Classes and instances

A class is a node type. Define one with `extend`, which takes a base class and
returns a constructor. Give the constructor a table of defaults:

```lua
local Panel = Canvas:extend("Panel")({ width = 200, height = 40 })

local a = Panel({ name = "clock" })
local b = Panel({ height = 60 })
```

An instance gets its defaults, and any field in the constructor table overrides
them. A subclass inherits every default and handler of its base. Selectors (below)
match on class names.

`extend` rejects a base that is not a class.

## Properties

A property is a typed value on a node. Assigning to it writes the value; reading
it returns the current value:

```lua
local root = Canvas({ width = 100 })
root.width = 300
print(root.width) -- 300
```

Property types are checked. A Lua property rejects a value of the wrong type,
and assigning to a property that does not exist raises an error. Native
properties are mirrored into Lua: a value you write reads back at once, and
reaches the compositor on the next frame.

A node reference is a property value too, and it reads back as the same Lua node.

## Bindings

A property can be a function, which makes it a binding. The function runs when
the binding is created and again whenever a property it read changes:

```lua
local outer = Canvas({ width = 100 })
local inner = Canvas({})
outer:add(inner)

inner.width = function(n)
    return n.parent.width / 2
end

outer.width = 200
print(inner.width) -- 100
```

A binding never re-runs because of its own write. Two bindings that feed each
other stop and report once, and the next frame does not report again. A binding
that raises an error is reported, and the error does not stop other bindings.

Use a binding for any value derived from other values, rather than writing a
handler that sets it again whenever its inputs change.

## Signals and handlers

A signal is a named event a node can send. Declare one by giving a field the
`signal` marker. Connect a function to it with `:connect`, and send it with
`:emit`:

```lua
local Button = Canvas:extend("Button")({ pressed = signal })

local b = Button({})
b.pressed:connect(function() print("pressed") end)
b.pressed:emit()
```

A handler named `on_<event>` on a class runs for every instance of that class
that receives the event:

```lua
local Panel = Canvas:extend("Panel")({})
function Panel:on_press()
    print("a panel was pressed")
end
```

### Input events bubble

An input event starts at its target and walks up through the parents. Each
handler on the way sees the event, and `stop()` ends the walk. A press on a
child reaches its parent unless a handler stops it.

The details of an input event (position, button, key) are fields of the table
its handlers receive. A request event reaches its node alone, with its request
name.

## Time: `await`, `every` and motion

Code that waits does so with `go`, `await` and `every`. There is no sleep call.
A wait resumes on the frame that reaches its time:

```lua
go(function()
    await(0.5)            -- resumes on the first frame at or after 0.5 s
    print("half a second later")
end)

every(1, function()       -- runs once a second
    print("tick")
end)
```

`await` inside a binding is an error, because a binding must finish before the
next frame. A binding can start a task with `go`, and that task can await.

A ready handler can await too. A window created with no parent gets its ready
handler once, and that handler can add the window to a parent later.

Motion is a method on a node. `to` changes properties over a duration and
returns a signal that fires when the motion ends:

```lua
local done = panel:to(0.2, { x = 10 })
go(function()
    await(done)
    print("moved")
end)
```

With no duration, `to` writes the values at once. Its signal still fires, on the
next frame.

## Selectors

A selector finds nodes under a node. A selector matches a class name, a node name
(`#name`), or both:

```lua
root:find("Panel#two")       -- the first match in the subtree
root:all("Panel")            -- every Panel below root
p2:closest("Panel#one")      -- walks up from p2, including p2
outer:all("> Canvas")        -- direct children only
```

`find` and `all` search the whole subtree. `closest` walks up the parents. A
leading `>` matches direct children only.

## Lifetime

`destroy` removes a node and its subtree. A handler of a destroyed node never
resumes, and a change to a destroyed node is dropped without a report, so you
do not need to check that a node still exists before you write to it.

## Where to look next

- [Shell files](user-shell.md): where your `shell.lua` goes and what it can reach.
- [Build your shell](build-your-shell.md): changing the stock shell step by step.
- [Lua runtime API](shell-api/index.md): every class, property and function.
