//! One agent application with separate runtime and tool-runner entry points.
//!
//! The tool runner belongs to this app but runs in a separate container under ADR 0033.
//! Its process must not inherit provider credentials or the runtime's platform identity.

pub mod context;
pub mod control;
pub mod engine;
pub mod execution;
pub mod memory;
pub mod skills;
pub mod tools;

pub mod storage;

pub mod journal;
