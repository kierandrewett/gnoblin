//! Test support: the private bus one test starts for itself.
//!
//! The agent serves a system-bus interface in production. A test must never register on
//! the real system bus — that slot is the running session's — so every D-Bus test starts
//! its own `dbus-daemon` and passes only that address to zbus.

#![allow(dead_code)]

pub mod bus;
