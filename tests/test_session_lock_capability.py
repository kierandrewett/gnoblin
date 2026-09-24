"""The Shell cutover may observe only the compositor's real protocol gate."""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PATCH = (ROOT / "patches/mutter/70-session-lock-capability/"
         "0001-backend-expose-Gnoblin-lock-capability.patch").read_text()
HEADER = (ROOT / "src/protocols/session-lock/"
          "meta-wayland-session-lock.h").read_text()


def test_backend_reads_manager_capability_not_policy_or_launcher_state():
    assert "meta_backend_get_gnoblin_session_lock_capability" in PATCH
    assert "meta_wayland_session_lock_get_capability (compositor)" in PATCH
    assert "meta_backend_get_gnoblin_session_lock_active" in PATCH
    assert "meta_wayland_session_lock_is_active (compositor)" in PATCH
    assert "GSubprocess" not in PATCH
    assert "SessionLockCoordinator" not in PATCH


def test_manager_contract_stays_zero_until_secure_global_exists():
    assert "meta_wayland_session_lock_get_capability" in HEADER
