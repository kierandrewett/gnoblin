# Requests

A request is how the compositor asks a provider to decide something: a
passphrase or secret prompt, a polkit authentication, a portal consent dialog,
a location request. The `request` object in the `gnoblin` schema defines the
exchange. Every request follows one lifecycle and ends with one `finished`
outcome.

## Lifecycle

1. **Offered.** The compositor offers the request to the connections that hold
   its slot. The `request.offered` signal names it by identity and carries what
   a provider needs to draw its prompt:
    - `request`: the identity the later calls name.
    - `deadline`: when the offer fails if nobody claims and answers it.
    - `slot`: the provider slot the offer is routed to.
    - `title` and `message`: the prompt's heading and body, chosen by the
      offering client.
    - `subject`: opaque bytes chosen by the offering client. The compositor
      does not interpret them.
2. **Claimed.** One connection claims it with `request.claim`. The first claim
   wins. A second claimant is refused with `request-taken`, which names the
   holder. A connection the request was not offered to is refused with
   `request-unknown`.
3. **Ended.** The claimant settles the request one of three ways:
    - `request.answer` sends the provider's answer. Only the claimant may answer,
      and an empty answer is still an answer.
    - `request.decline` refuses it, with a reason. Declining is its own member so
      that "refused" and "answered with nothing" stay distinct.
    - No settlement before the deadline ends it as timed out.

Whatever ends a request, the compositor emits `request.finished` once. Its
completion is one of four kinds: answered, ended, refused (carrying the decline
reason) or timed out. The answer value is never part of `finished`; only the
connection that offered the request receives it.

## Ownership

A request belongs to the provider it was offered to and to the connection that
offered it.

- If the provider disconnects, its unclaimed and claimed requests end.
- If the offering connection disconnects, the requests it offered are abandoned.
- A late claim, answer or decline on a request that already finished is refused
  with `request-finished`.

## Secrets

`answer` carries a field marked `secret` in the schema. The compositor flags and
redacts every frame that carries it, so `gnoblinctl ipc dump`, the parity
recorder and logs never show the bytes.

## Status

The IPC lifecycle is implemented in `crates/gnoblin-compositor/src/ipc/requests.rs`.
The schema now declares the full `offered` signal above. The Lua surface is
being wired to it; until that lands, Lua does not receive `request.offered` or
`request.finished`, and the `request` class has no way to claim, answer or
decline without an object identity.

With no provider holding a request's slot, the request is refused at once. A
provider that claims it but never answers ends it as timed out.
