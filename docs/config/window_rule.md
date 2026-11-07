# gnoblin.window_rule

Add an ordered window rule. Every key in `match` must hold for the rule to
apply. Matching rules run in list order, and a later rule overrides an earlier
one only for the fields it sets.

See the [window rules guide](/guides/window_rules) for usage patterns.

```lua
gnoblin.window_rule {
    match = {type = "window", app_id = "^org%.example%.Editor$"},
    workspace = {id = "code"},
    maximise = true,
}
```

## Match keys

| Key                | Values                                                   |
| ------------------ | -------------------------------------------------------- |
| `type`             | `"window"`                                               |
| `focused`          | Boolean; matches the window's keyboard focus             |
| `app_id`           | Lua pattern against GTK app ID, falling back to WM class |
| `title`            | Lua pattern against the current window title             |
| `workspace_id`     | Exact workspace ID                                       |
| `workspace_number` | Current one-based workspace position, 1–1024             |
| `client_type`      | `"wayland"` or `"x11"`; `"x11"` is an Xwayland client    |

All supplied match keys must match. An omitted or empty `match` selects every
window.

Text patterns use Lua 5.4 `string.find` syntax. They are case-sensitive and
search anywhere by default. Use `^` and `$` to match the whole value.

- `.` matches any character, and `%.` matches a literal dot.
- `^org%.example%.App$` matches the complete ID `org.example.App`.
- JavaScript and PCRE regex syntax is not supported.
- A malformed pattern is a load error.

See the [window rules guide](/guides/window_rules#find-the-values) for ways to
find a live app ID or title.

Declare persistent workspaces in `gnoblin.configure`'s `workspaces` field and
match them by ID here.

## Rule fields

| Field       | Values                                                             |
| ----------- | ------------------------------------------------------------------ |
| `workspace` | `{id = "code"}` or `{number = 1..1024}`, exactly one of the two    |
| `maximise`  | Boolean; `true` maximises, `false` restores, when the rule applies |

`workspace` places the window on that workspace. A `{id = ...}` target must
exist; otherwise the window stays where it is and Gnoblin logs a warning.

`maximise` takes effect at the window's first match and again when a re-match
changes the set of matching rules. A fullscreen window is not maximised.

## When rules apply

Rules are matched when a window opens. They are matched again when its title or
application ID changes. The fields of the matching rules apply again only when
the set of matching rules differs from the set the window last had. The whole
current set is merged, so the last matching rule still wins.

A re-match can therefore move a window or change its maximised state, for
example when a title change makes a rule with `workspace` start to match.

A window that loses its match keeps what the rule already did. A configuration
reload does not re-run rules on windows that are already open; new rules apply
only to windows that open afterwards.

## Limits

A `window-rules` list holds at most 256 rules. A longer list is a load error.

## Refused keys

A configuration that sets one of these keys is rejected at load, and the last
accepted configuration stays in force. Each refusal names the key and ends with
"not built in this compositor". Either the dashed or underscored spelling
of a key is refused.

| Key                         | Reason given                           |
| --------------------------- | -------------------------------------- |
| `blur`                      | Needs the blur pass                    |
| `blur-ignore-shadows`       | Needs the blur pass                    |
| `opacity`                   | Needs the alpha pass                   |
| `corners`                   | Needs the rounded-corner pass          |
| `shader`, `shader-uniforms` | Needs the WASM shader host             |
| `animation`                 | Needs the animation system             |
| `frame`                     | Needs the layer-shell surface renderer |
| `match.layer`               | Needs the layer-shell surface selector |

## Migrating from the legacy effect keys

The older docs describe flat effect keys, which do not apply in this build:

- `blur` and `opacity`
- `corners`, `shader`, `animation` and `frame`

None of them takes effect here. Remove them from `window_rule` tables, or the
load fails. Keep the rules that set `workspace`, `maximise` or match keys.

The planned replacement is an ordered `effects` list on each rule. Each entry
is one pass, added in stack order, so the author decides the order of blur,
shadow and shader. The design is D26 in `design/rust-compositor.md`. The flat
`blur` and `shader` keys are removed in the same cutover that replaces
`window:set_effect`, so no config should depend on either until that lands.
