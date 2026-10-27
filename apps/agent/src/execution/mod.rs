//! Tool-runner implementation inside the agent application.
//!
//! The gated terminal supervisor owns process lifetime and private invocation evidence.
//! It never owns model credentials or agent turns.

pub mod network;

pub mod terminal;
