//! The agent application. The agent runs its own tools inside its sandbox (ADRs 0083, 0084).

pub mod collaboration;
pub mod context;
pub mod control;
pub mod engine;
#[cfg(target_os = "linux")]
pub mod guest;
pub mod memory;
pub mod skills;
pub mod tools;

pub mod journal;

/// Any error carried as the cause of another.
pub type Source = Box<dyn std::error::Error + Send + Sync>;

/// A fixed message that keeps the error that caused it.
#[derive(Debug)]
pub struct Caused {
    message: &'static str,
    source: Option<Source>,
}
impl Caused {
    pub fn new(message: &'static str, source: impl Into<Source>) -> Self {
        Self {
            message,
            source: Some(source.into()),
        }
    }
}
impl From<&'static str> for Caused {
    fn from(message: &'static str) -> Self {
        Self {
            message,
            source: None,
        }
    }
}
impl std::fmt::Display for Caused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}
impl std::error::Error for Caused {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}
