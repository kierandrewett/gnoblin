# Touchpad gestures

A swipe or a pinch of several fingers is a binding: `input.bind` takes a gesture trigger and a function, and the
function runs once, when the fingers lift.

```ts
const input = gnoblin.input.primary;

input.bind("Swipe4Up", () => {
    input.focusedClient?.close();
});
input.bind("Pinch3In", () => {
    gnoblin.spawn(["kgx"]);
});
```

A trigger is `Swipe` or `Pinch`, a finger count, and a direction: `Swipe3Left`, `Swipe4Up`, `Pinch3In`, `Pinch4Out`.
The stock setup binds a three-finger swipe to `WorkspaceSlideBinding`, which follows the fingers while they move:

```ts
import { WorkspaceSlideBinding } from "gnoblin-std";

gnoblin.input.primary.bind("Swipe3", new WorkspaceSlideBinding());
```

A trigger that is already bound at the same priority is refused, so delete the stock line before you bind a three-finger
swipe of your own. See [window policy](/js-window-policy#gesture-actions) for the event a function receives.

A gesture drawn as a path of points (`touchpadGestures`) has no TypeScript home yet: see
[settings with no TypeScript home yet](/js-settings#dropped-settings).

## Status

None of this has been run against a touchpad. The routing, cancel, lock and finger-count rules are covered by tests
that feed synthetic samples, and no hardware proof is claimed.
