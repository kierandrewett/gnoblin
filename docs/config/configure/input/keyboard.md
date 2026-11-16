# Keyboard settings

A `Keyboard` has two settings, the key repeat rate and the delay before a key repeats. Both are applied by the
compositor on a reload. This example repeats a held key 40 times a second after 250 ms:

```ts
import { Keyboard } from "gnoblin-core";

for (const device of gnoblin.input.devices) {
    if (device instanceof Keyboard) {
        device.repeatRate = 40;
        device.repeatDelay = 250;
    }
}
```

| Property      | Values                   | Default      | Effect                                        |
| ------------- | ------------------------ | ------------ | --------------------------------------------- |
| `repeatRate`  | Number, 0.1 to 1000      | The system's | Key repeats per second.                       |
| `repeatDelay` | Whole number, 1 to 10000 | The system's | Milliseconds a key is held before it repeats. |

`undefined` takes a property back to the system's value. The shell writes `repeatRate` as the reciprocal, in whole
milliseconds, to the document's `input.keyboard.repeat-interval`, and `repeatDelay` to `input.keyboard.delay`.

The XKB options (`caps:escape`, `compose:ralt`), Num Lock state and the keyboard accessibility features (slow keys,
sticky keys, bounce keys, mouse keys) have no TypeScript home yet: see
[settings with no TypeScript home yet](/js-settings#dropped-settings).
