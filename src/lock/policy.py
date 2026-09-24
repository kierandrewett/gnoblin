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
    PRESENTED = "presented"
    FAILED = "failed"


@dataclass(frozen=True)
class LockRequest:
    token: str
    reason: str


@dataclass
class LockPolicy:
    """Tracks a single lock owner and fail-closed presentation lifecycle.

    ``presented`` means only that the configured locker reported its received
    ``ext_session_lock_v1.locked`` event.  It is *not* a compositor attestation;
    callers must keep the feature opt-in until the compositor implementation is
    validated on hardware.
    """

    state: State = State.UNLOCKED
    active_token: str | None = None
    active_since: float | None = None
    inhibitors: dict[int, tuple[str, str]] = field(default_factory=dict)
    _next_inhibitor: int = 1

    def request_lock(self, reason: str) -> LockRequest | None:
        if self.state not in (State.UNLOCKED, State.FAILED):
            return None
        self.active_token = secrets.token_urlsafe(32)
        self.state = State.REQUESTED
        return LockRequest(self.active_token, reason)

    def report_presented(self, token: str, now: float | None = None) -> bool:
        if self.state is not State.REQUESTED or token != self.active_token:
            return False
        self.state = State.PRESENTED
        self.active_since = time.monotonic() if now is None else now
        return True

    def report_failed(self, token: str) -> bool:
        """Record launch failure without ever treating it as an unlock."""
        if self.state is not State.REQUESTED or token != self.active_token:
            return False
        self.active_token = None
        self.state = State.FAILED
        return True

    def locker_disconnected(self) -> None:
        """A presented lock may not transition to unlocked on client death."""
        if self.state is State.REQUESTED:
            self.active_token = None
            self.state = State.UNLOCKED
        # PRESENTED intentionally remains PRESENTED.  The compositor must keep
        # a black fallback until a privileged recovery path is implemented.

    def compositor_unlocked(self) -> None:
        """Called only after the owning Wayland client unlocks the compositor."""
        self.state = State.UNLOCKED
        self.active_token = None
        self.active_since = None

    def inhibit(self, application: str, reason: str) -> int:
        cookie = self._next_inhibitor
        self._next_inhibitor += 1
        self.inhibitors[cookie] = (application, reason)
        return cookie

    def uninhibit(self, cookie: int) -> bool:
        return self.inhibitors.pop(cookie, None) is not None

    @property
    def idle_lock_allowed(self) -> bool:
        return not self.inhibitors and self.state is State.UNLOCKED
