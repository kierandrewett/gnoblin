# Window rules

A window rule changes what the shell does to the windows it matches. Rules are the `rules` list of your
`defineConfig`, in `~/.config/gnoblin/init.ts`. They load with the config, and a save reloads them.

## Add a rule

This places every editor window on workspace `dev` and maximises it:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    workspaces: [{ id: "web" }, { id: "dev" }],
    rules: [{ appId: "org.example.Editor", workspace: "dev", maximised: true }],
});
```

A rule runs when a window opens, so it does not move windows that are already open when the config loads.

The [window rule reference](/config/window_rule) lists every query key and field.

## Choose what to match

Every query key you give must match. If a key is omitted, the rule matches any value of it.

| Key           | Matches                                         |
| ------------- | ----------------------------------------------- |
| `appId`       | The application ID, as a glob or a `RegExp`     |
| `title`       | The current title, as a glob or a `RegExp`      |
| `focused`     | `true` or `false`                               |
| `onWorkspace` | The id of the window's workspace, such as `"2"` |

Choose the key based on what identifies the window:

- Use `appId` to match an application across its windows.
- Use `title` to target a particular document or window. Titles often change as the document or page changes.

Layer-shell surfaces are not windows, so a rule never matches them.

## Find the values

List the live windows with:

```sh
gnoblinctl window list
```

Use the `app_id` and `title` of the window you want as the values to match.

## Write a matcher

`appId` and `title` take a glob string or a `RegExp`:

- A glob matches the whole text. `*` is any run of characters and `?` is one character. A dot is a dot.
- A `RegExp` is tested anywhere in the text. Use `^` and `$` to match the whole value.
- Both are case-sensitive.

To match `org.gnome.Nautilus` only, write the full id:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    rules: [{ appId: "org.gnome.Nautilus", maximised: true }],
});
```

To match a title that contains `Notes` anywhere, put a `*` on both sides, or use a `RegExp`:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    rules: [{ title: /Notes/, maximised: false }],
});
```

This also matches `Notes — Work`. To match only the title `Notes`, write `title: "Notes"`.

This rule targets only unfocused windows from one app:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    rules: [{ appId: "org.gnome.Nautilus", focused: false, style: { opacity: 0.9 } }],
});
```

## Workspaces {#workspaces}

The stock config has four workspaces, named `"1"` to `"4"`. When your config declares `workspaces`, the shell builds
those instead, and each workspace is named by its `id`. Name the workspace by that id in a rule:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    workspaces: [{ id: "code" }, { id: "web" }],
    rules: [
        { appId: "org.example.Editor", workspace: "code" },
        { onWorkspace: "web", maximised: true },
    ],
});
```

A `workspace` that names a workspace the config does not declare fails the load.

## When rules run again

A rule runs when a window opens. It runs again when the window's title, app ID or focus changes whether the rule
matches.

- A field that no matching rule sets falls back to its default.
- A window whose title stops matching a `secure` rule is not secure.
- A window that stops matching a `maximised` rule is unmaximised.
- Give a rule `reapply: false` to apply it once, when the window opens, and leave the window alone afterwards.

## Rule order

Matching does not stop at the first rule. Later rules override only the fields they set, so a rule that maximises every
window can be followed by a rule for one application that sets `maximised: false`. A rule with `first: true` ends the
search where it matches.

The `rules` list replaces the stock list as a whole. A list with a mistake is refused as a whole at load, with one
error that names the rule, and the rules already in force stay.
