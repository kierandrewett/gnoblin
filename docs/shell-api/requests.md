# Requests

A `Request` is something your config answers: a keyring or GPG prompt, a polkit
authentication, a location authorization or a session-lock request. The event
that offers it carries it as `event.request`. Follow-up events of the same
request carry the same object.

```lua
gnoblin.on("gnoblin.prompt.requested", function(event)
    local request = event.request
    request:finished():on_complete(function(outcome)
        print(request.kind, request.id, outcome)
    end)
    request:cancel()
end)
```

This handler cancels every keyring and GPG prompt and prints the outcome when
each one ends.

## Fields

Every request has these fields. All fields are read-only.

| Field    | Type    | Meaning                                                             |
| -------- | ------- | ------------------------------------------------------------------- |
| `id`     | integer | Identifies the request among requests of the same kind.             |
| `kind`   | string  | What the request asks for. See [Kinds](#kinds).                     |
| `status` | string  | `"pending"`, `"claimed"`, `"answered"` or `"finished"`.             |
| `result` | string  | The outcome after `status` becomes `"finished"`. `nil` before that. |

A request also has the detail fields of the event that offered it, such as
`title` for a prompt or `app_id` for location. The page for each kind lists
them.

`status` starts as `"pending"`. It becomes `"claimed"` after `claim()`,
`"answered"` after `answer()` or `cancel()`, and `"finished"` when the request
ends for any reason.

## Kinds

| `kind`             | Offered by                                 | Outcomes                            |
| ------------------ | ------------------------------------------ | ----------------------------------- |
| `"password"`       | `gnoblin.prompt.requested`                 | `answered`, `cancelled`, `timeout`  |
| `"confirm"`        | `gnoblin.prompt.requested`                 | `answered`, `cancelled`, `timeout`  |
| `"authentication"` | `gnoblin.auth.requested`                   | `authorized`, `denied`, `cancelled` |
| `"location"`       | `gnoblin.location.authorization-requested` | `allowed`, `denied`, `timeout`      |
| `"lock"`           | `gnoblin.session.lock-requested`           | `locked`, `timeout`                 |

- `"password"` asks for a password and `"confirm"` asks for a yes or no answer.
  See [Keyring and GPG prompts](/shell-api/passphrase-prompts).
- `"authentication"` asks for a polkit password. See
  [Authentication prompts](/shell-api/authentication).
- `"location"` asks whether an application may use location. See
  [Privacy](/shell-api/privacy#location-requests).
- `"lock"` asks the shell to start its locker. It takes no answer. It finishes
  with `locked` when the session locks, or `timeout` after 30 seconds. See
  [Session lock](/session-lock).

## Methods

A method exists only on the kinds that use it. Calling it on another kind
raises an error that names the kind and the method.

| Method                   | Kinds             | Effect                                                |
| ------------------------ | ----------------- | ----------------------------------------------------- |
| `answer(value, option?)` | all except `lock` | Answer the request. Arguments depend on the kind.     |
| `cancel()`               | all except `lock` | Refuse the request.                                   |
| `claim(identity?)`       | `authentication`  | Start the check for one identity, counted from 1.     |
| `on(signal, fn)`         | `authentication`  | Call `fn(request)` on each `"prompt"` or `"message"`. |
| `finished()`             | all               | An Operation that succeeds once with the outcome.     |

`answer` takes these arguments:

| Kind             | Arguments                  | Example                    |
| ---------------- | -------------------------- | -------------------------- |
| `password`       | password string, choice?   | `request:answer(password)` |
| `confirm`        | confirmed boolean, choice? | `request:answer(true)`     |
| `authentication` | response string            | `request:answer(password)` |
| `location`       | allow boolean, accuracy?   | `request:answer(true, 4)`  |

`choice` is the state of a prompt's optional checkbox. For location, `accuracy`
defaults to the requested accuracy when `allow` is `true`. `cancel()` on a
location request denies it.

`answer`, `cancel` and `claim` return an Operation that succeeds with
`{request_id = ..., submitted = true}` after Gnoblin accepts the answer. An
answer with wrong values fails that Operation with a message that names the
problem. A request takes one answer; a second answer fails.

## Answer from a shell process

A socket client does not get `Request` objects. It receives the same events
with `request_id` and answers with the wire methods that each kind's page
lists. Only the client that received the event can answer that request.
