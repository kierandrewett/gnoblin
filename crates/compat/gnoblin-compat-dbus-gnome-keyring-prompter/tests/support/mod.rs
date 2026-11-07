//! Test support: the private bus, and a subscriber that captures what the adapter logs.
//!
//! Both exist for one reason: the tests must not touch the real session or system bus,
//! and "the passphrase never reaches a log" is only a claim until something reads the
//! log.

#![allow(dead_code)]

pub mod bus;
pub mod log;
