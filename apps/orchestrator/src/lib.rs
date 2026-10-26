//! Installation-wide behavior, exposed through the orchestrator application.
//!
//! Each domain owns its rules and persistence. PostgreSQL identity is the first implemented domain slice.

pub mod agents;
pub mod api;
pub mod connections;
pub mod identity;
pub mod knowledge;
pub mod operations;
pub mod storage;
pub mod work;
