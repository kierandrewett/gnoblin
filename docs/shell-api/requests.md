# Requests and dialogs

A request is how the compositor asks someone to decide: a portal consent question, a keyring passphrase, a polkit
authentication. The compositor draws the dialog itself, so it works with no shell and a script never sees a password.
`gnoblin.dialogs` lets your config watch these requests and refuse one before its dialog opens.

```ts
import { DialogListener, defineConfig } from "gnoblin";
import type { ConsentRequest } from "gnoblin";

class NoScreenCasts extends DialogListener {
    onConsentRequest(request: ConsentRequest) {
        if (request.subject === "ScreenCast") {
            request.decline("screen casts are off");
        }
    }
}

defineConfig({ setup: (gnoblin) => gnoblin.dialogs.addListener(new NoScreenCasts()) });
```

Put the file in `~/.config/gnoblin/init.ts`. A program that asks for a screen cast is refused, and the dialog never opens.

## Watch and refuse

A listener is shown each request for a short window before the dialog opens, and may refuse it. `decline(reason)` is the
only call that decides anything: the requester is told it was refused and the dialog never opens. `reason` is for the log.

There is no call that allows a request, and none that reads or gives a secret. A listener that does nothing, throws or runs
past its time budget leaves the request to the dialog. `decline` after the window has closed does nothing.

`gnoblin.dialogs` has these members:

| Member                     | Meaning                      |
| -------------------------- | ---------------------------- |
| `addListener(listener)`    | Attaches a `DialogListener`. |
| `removeListener(listener)` | Detaches it.                 |
| `listeners`                | The attached listeners.      |

A `DialogListener` can define these methods. Both are optional.

| Method                      | Called for                       |
| --------------------------- | -------------------------------- |
| `onConsentRequest(request)` | A portal consent dialog.         |
| `onSecretRequest(request)`  | The keyring's passphrase prompt. |

A polkit authentication is watched through [`gnoblin.authentication`](authentication), which serves the same observation.

### What a request carries

| Field      | Type   | Meaning                                                                                         |
| ---------- | ------ | ----------------------------------------------------------------------------------------------- |
| `kind`     | string | `consent` for a portal question, `secret` for the passphrase prompt, `auth` for polkit.         |
| `appId`    | string | The application id the compositor verified for whoever asks. Empty when the request names none. |
| `subject`  | string | What is asked about. See below.                                                                 |
| `deadline` | number | When the compositor takes the request back, in milliseconds on its own clock.                   |

`subject` depends on the kind:

- For `consent`, the portal interface that asked, such as `Screenshot` or `ScreenCast`.
- For `secret`, the program's own prompt text. It is the program's words, not an identity to trust.
- For `auth`, the program asking, such as `pkexec`.

Show `appId`, not text the program chose, when you tell the user who is asking.

## The dialogs

The compositor draws all three above every window and layer, and a screen capture leaves them out. A script cannot draw
over them or hide them. While one is open it takes every key and click, so no client or script sees what you type.

### Consent

The portal backend asks for consent before an application can capture the screen, control input, read its secret or use
the accessibility dialog. The dialog opens with Deny focused, ignores input for the first 500 ms, and names the caller by
the application id the portal backend verified. The look of the dialogs is the [`dialogs`](/config/configure/dialogs) key.

### Keyring passphrase

The built-in keyring is locked until it receives its login password, and it asks through the compositor's passphrase
dialog. There is no config switch for it: the keyring asks whenever it needs the password. Turn the keyring off with the
[keyring configuration](/config/configure/keyring) if you use another one.

The dialog:

- shows a lock, the request's title and question, and who asks;
- masks the entry with one mark per character. Return confirms a non-empty entry, and Escape or Cancel declines, which ends
  the request with no answer;
- queues requests that arrive while one is open, and declines them when the session locks;
- wipes the passphrase from memory when it is answered, and logs no more than the request number.

A request expires after 120 seconds. An answer after that is discarded and the keyring stays locked.

::: details GnuPG passphrases
A passphrase that GnuPG asks for through `pinentry-gnome3` takes another route. It hands the request to `gcr-prompter` over
D-Bus, which opens its own window: an ordinary toplevel that Gnoblin maps and focuses like any other. The compositor draws
nothing for it. `gcr-prompter` has to be running on the session bus, or `pinentry-gnome3` falls back to a terminal prompt.
:::

## Request lifecycle

Every request follows one lifecycle and ends with one `finished` outcome, which you can hear with
[`gnoblin.request.offered` and `gnoblin.request.finished`](/events/lifecycle#requests-behind-dialogs).

1. **Offered.** The compositor offers the request to the program that holds its slot. The offer names the request and
   carries the prompt's `title`, `message` and `subject`, which the asking program chose.
2. **Claimed.** One connection claims it. The first claim wins.
3. **Ended.** The claimant answers or refuses, or the deadline passes and the request ends as timed out.

`finished` carries the outcome, which is `answered`, `ended`, `refused` with the reason, or `timed out`. It never carries the
answer: only the program that offered the request receives that.

A request ends when its provider disconnects, and a request whose offering program disconnects is abandoned. A late claim,
answer or refusal is refused. A consent or passphrase request with no program on its slot is served by the compositor's own
dialog.

A program of your own can hold a slot and draw the dialog: see [Replace the built-in dialogs](/guides/replace-built-in-dialogs).
