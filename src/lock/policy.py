"""State and policy for the opt-in Gnoblin session-lock broker.

This deliberately has no dependency on D-Bus or Wayland so its security
transitions can be tested without treating a headless test as compositor proof.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
import secrets
import time


class State(str, Enum):
    UNLOCKED = "unlocked"
    REQUESTED = "requested"
    COMPOSITOR_COVERING = "compositor-covering"
    COMPOSITOR_LOCKED = "compositor-locked"
    FAILED = "failed"


@dataclass(frozen=True)
class LockRequest:
    token: str
    reason: str


@dataclass
class LockPolicy:
    """Tracks a single lock owner and fail-closed presentation lifecycle.

    A locker can provide diagnostic telemetry, but only an internal future
    compositor callback may enter ``COMPOSITOR_LOCKED``. Session-bus clients do
    not attest that pixels were presented or input was isolated.
    """

    state: State = State.UNLOCKED
    active_token: str | None = None
    active_since: float | None = None
    active_reason: str | None = None
    client_reported_presented: bool = False
    inhibitors: dict[int, tuple[str, str]] = field(default_factory=dict)
    _next_inhibitor: int = 1

    def request_lock(self, reason: str) -> LockRequest | None:
        if self.state not in (State.UNLOCKED, State.FAILED):
            return None
        self.active_token = secrets.token_urlsafe(32)
        self.active_reason = reason
        self.client_reported_presented = False
        self.state = State.REQUESTED
        return LockRequest(self.active_token, reason)

    def report_client_presented(self, token: str) -> bool:
        """Record diagnostic client telemetry; this cannot change lock state."""
        if self.state is not State.REQUESTED or token != self.active_token:
            return False
        self.client_reported_presented = True
        return True

    def compositor_locked(self, now: float | None = None) -> bool:
        """Internal-only boundary for a future trusted compositor callback."""
        if self.state is State.COMPOSITOR_LOCKED:
            return True
        self.state = State.COMPOSITOR_LOCKED
        self.active_since = time.monotonic() if now is None else now
        return True

    def compositor_covering(self) -> bool:
        """The compositor has hidden normal content but is not yet presented."""
        if self.state is State.COMPOSITOR_LOCKED:
            return False
        self.state = State.COMPOSITOR_COVERING
        return True

    def report_failed(self, token: str) -> bool:
        """Record launch failure without ever treating it as an unlock."""
        if self.state is not State.REQUESTED or token != self.active_token:
            return False
        self.active_token = None
        self.active_reason = None
        self.client_reported_presented = False
        self.state = State.FAILED
        return True

    def locker_disconnected(self) -> None:
        """Client death never proves the compositor restored the session."""
        if self.state is State.REQUESTED:
            self.active_token = None
            self.client_reported_presented = False
            self.state = State.FAILED
        # COMPOSITOR_COVERING and COMPOSITOR_LOCKED intentionally remain locked.
        # The compositor must keep
        # a black fallback until a privileged recovery path is implemented.

    def compositor_unlocked(self) -> None:
        """Called only after the owning Wayland client unlocks the compositor."""
        self.state = State.UNLOCKED
        self.active_token = None
        self.active_since = None
        self.active_reason = None
        self.client_reported_presented = False

    def inhibit(self, application: str, reason: str) -> int:
        cookie = self._next_inhibitor
        self._next_inhibitor += 1
        self.inhibitors[cookie] = (application, reason)
        return cookie

    def uninhibit(self, cookie: int) -> bool:
        return self.inhibitors.pop(cookie, None) is not None

    @property
    def idle_lock_allowed(self) -> bool:
        return not self.inhibitors and self.state in (State.UNLOCKED, State.FAILED)
