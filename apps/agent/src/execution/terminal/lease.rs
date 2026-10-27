//! Renewable connection authority, independent of command duration.
use cubebot_protocol::terminal::LEASE_MILLIS;
use std::io;
use tokio::time::{Duration, Instant};

pub(super) struct Lease {
    pub deadline: Instant,
    pub bytes: [u8; 8],
    pub received: usize,
}
impl Lease {
    pub fn new(not_after: u64) -> io::Result<Option<Self>> {
        Ok(deadline(not_after)?.map(|deadline| Self {
            deadline,
            bytes: [0; 8],
            received: 0,
        }))
    }
    pub fn receive(&mut self, size: usize) -> io::Result<bool> {
        if size == 0 || Instant::now() >= self.deadline {
            return Ok(false);
        }
        self.received += size;
        if self.received == self.bytes.len() {
            let Some(deadline) = deadline(u64::from_be_bytes(self.bytes))? else {
                return Ok(false);
            };
            self.deadline = deadline;
            self.received = 0;
        }
        Ok(true)
    }
}
fn deadline(not_after: u64) -> io::Result<Option<Instant>> {
    let start = Instant::now();
    let remaining = not_after.saturating_sub(super::monotonic_millis()?);
    Ok((remaining > 0 && remaining <= LEASE_MILLIS)
        .then(|| start + Duration::from_millis(remaining)))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renewable_authority_handles_fragmented_frames_without_extending_expiry() {
        let now = super::super::monotonic_millis().unwrap();
        assert!(Lease::new(now.saturating_sub(1)).unwrap().is_none());
        assert!(Lease::new(now + LEASE_MILLIS + 1000).unwrap().is_none());
        let mut lease = Lease::new(now + 1000).unwrap().unwrap();
        let first = lease.deadline;
        lease.bytes = (now + 5000).to_be_bytes();
        assert!(lease.receive(3).unwrap());
        assert_eq!(lease.deadline, first);
        assert!(lease.receive(5).unwrap());
        assert!(lease.deadline > first);
        lease.deadline = Instant::now() - Duration::from_millis(1);
        lease.bytes = (now + 5000).to_be_bytes();
        assert!(!lease.receive(8).unwrap());
        assert!(!lease.receive(0).unwrap());
    }
}
