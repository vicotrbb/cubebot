use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "cubebot-journal-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn retains_records_until_acknowledged_and_never_reuses_committed_sequence() {
    let fixture = Fixture::new();
    let mut journal = Journal::reopen(&fixture.0).unwrap();
    assert_eq!(journal.append(b"intent").unwrap(), 1);
    assert_eq!(journal.append(b"outcome").unwrap(), 2);
    assert!(journal.acknowledge(3).is_err());
    drop(journal);
    let mut journal = Journal::reopen(&fixture.0).unwrap();
    assert_eq!(journal.pending()[0].payload, b"intent");
    assert_eq!(journal.pending()[1].payload, b"outcome");
    journal.acknowledge(2).unwrap();
    journal.acknowledge(1).unwrap();
    drop(journal);
    let mut journal = Journal::reopen(&fixture.0).unwrap();
    assert!(journal.pending().is_empty());
    assert_eq!(journal.append(b"next").unwrap(), 3);
}

#[test]
fn one_writer_holds_a_stable_lock_across_checkpoint_replacement() {
    let fixture = Fixture::new();
    let mut journal = Journal::reopen(&fixture.0).unwrap();
    journal.append(b"first").unwrap();
    assert!(Journal::open(&fixture.0).is_err());
    journal.acknowledge(1).unwrap();
    assert!(Journal::open(&fixture.0).is_err());
    drop(journal);
    assert!(Journal::reopen(&fixture.0).is_ok());
}

#[test]
fn corrupt_or_missing_committed_evidence_is_never_discarded() {
    let fixture = Fixture::new();
    let mut journal = Journal::reopen(&fixture.0).unwrap();
    journal.append(b"retained").unwrap();
    drop(journal);
    let path = fixture.0.join(record_name(1));
    fs::write(&path, b"damaged").unwrap();
    assert!(Journal::reopen(&fixture.0).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"damaged");
    fs::remove_file(&path).unwrap();
    assert!(Journal::reopen(&fixture.0).is_err());
}

#[test]
fn rejects_untrusted_paths_and_oversized_records() {
    let fixture = Fixture::new();
    let mut journal = Journal::reopen(&fixture.0).unwrap();
    assert!(journal.append(&vec![0; MAX_PAYLOAD + 1]).is_err());
    assert!(journal.pending().is_empty());
    drop(journal);
    fs::remove_file(fixture.0.join("lock")).unwrap();
    std::os::unix::fs::symlink("state", fixture.0.join("lock")).unwrap();
    assert!(Journal::reopen(&fixture.0).is_err());
}

#[test]
fn failed_checkpoint_poisoning_requires_recovery_before_another_action() {
    let fixture = Fixture::new();
    let mut journal = Journal::reopen(&fixture.0).unwrap();
    fs::create_dir(fixture.0.join(".state.pending")).unwrap();
    assert!(journal.append(b"unacknowledged append").is_err());
    assert!(journal.append(b"must not run").is_err());
    drop(journal);
    fs::remove_dir(fixture.0.join(".state.pending")).unwrap();
    let mut journal = Journal::reopen(&fixture.0).unwrap();
    assert!(journal.pending().is_empty());
    assert_eq!(journal.append(b"recovered").unwrap(), 1);
}

#[test]
fn recovers_after_process_exit_without_destructors() {
    let fixture = Fixture::new();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "journal::tests::journal_child", "--nocapture"])
        .env("CUBEBOT_JOURNAL_TEST_ROOT", &fixture.0)
        .status()
        .unwrap();
    assert!(status.success());
    let journal = Journal::reopen(&fixture.0).unwrap();
    assert_eq!(journal.pending().len(), 1);
    assert_eq!(journal.pending()[0].payload, b"child committed");
}

#[test]
fn journal_child() {
    let Some(root) = std::env::var_os("CUBEBOT_JOURNAL_TEST_ROOT") else {
        return;
    };
    let mut journal = Journal::open(Path::new(&root)).unwrap();
    journal.append(b"child committed").unwrap();
    std::process::exit(0);
}

#[test]
fn capacity_exhaustion_retains_prior_evidence_and_acknowledgement_frees_space() {
    let fixture = Fixture::new();
    let mut journal = Journal::reopen(&fixture.0).unwrap();
    let payload = vec![42; MAX_PAYLOAD];
    let capacity = MAX_BYTES / (MAX_PAYLOAD + RECORD_HEADER);
    for _ in 0..capacity {
        journal.append(&payload).unwrap();
    }
    assert!(journal.append(&payload).is_err());
    drop(journal);
    let mut journal = Journal::reopen(&fixture.0).unwrap();
    assert_eq!(journal.pending().len(), capacity);
    assert_eq!(journal.pending()[0].payload, payload);
    journal.acknowledge(1).unwrap();
    assert_eq!(journal.append(&payload).unwrap(), capacity as u64 + 1);
}

#[test]
fn checkpoint_corruption_and_hard_links_are_rejected_without_resetting() {
    let fixture = Fixture::new();
    drop(Journal::reopen(&fixture.0).unwrap());
    let state = fixture.0.join("state");
    let mut bytes = fs::read(&state).unwrap();
    bytes[12] ^= 1;
    fs::write(&state, &bytes).unwrap();
    assert!(Journal::reopen(&fixture.0).is_err());
    assert_eq!(fs::read(&state).unwrap(), bytes);
    fs::hard_link(&state, fixture.0.join("linked")).unwrap();
    assert!(private_file(&state, false).is_err());
}

#[test]
fn cleans_only_uncommitted_or_acknowledged_files_after_restart() {
    let fixture = Fixture::new();
    let mut journal = Journal::reopen(&fixture.0).unwrap();
    journal.append(b"committed").unwrap();
    drop(journal);
    files::publish(
        &fixture.0,
        ".entry.pending",
        &record_name(2),
        b"incomplete tail",
    )
    .unwrap();
    files::publish(
        &fixture.0,
        ".unused",
        ".state.pending",
        b"incomplete checkpoint",
    )
    .unwrap();
    let journal = Journal::reopen(&fixture.0).unwrap();
    assert_eq!(journal.pending().len(), 1);
    assert!(!fixture.0.join(record_name(2)).exists());
    assert!(!fixture.0.join(".state.pending").exists());
    drop(journal);
    files::publish(
        &fixture.0,
        ".unused",
        "state",
        &encode_state(State {
            committed: 1,
            acknowledged: 1,
        }),
    )
    .unwrap();
    let journal = Journal::reopen(&fixture.0).unwrap();
    assert!(journal.pending().is_empty());
    assert!(!fixture.0.join(record_name(1)).exists());
}

#[test]
fn missing_checkpoint_cannot_reset_acknowledged_high_water_mark() {
    let fixture = Fixture::new();
    let mut journal = Journal::reopen(&fixture.0).unwrap();
    journal.append(b"acknowledged").unwrap();
    journal.acknowledge(1).unwrap();
    drop(journal);
    fs::remove_file(fixture.0.join("state")).unwrap();
    assert!(Journal::reopen(&fixture.0).is_err());
    assert!(!fixture.0.join("state").exists());
}

#[test]
fn detects_same_length_payload_tampering_and_nonprivate_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let mut journal = Journal::reopen(&fixture.0).unwrap();
    journal.append(b"original").unwrap();
    drop(journal);
    let path = fixture.0.join(record_name(1));
    let original = fs::read(&path).unwrap();
    let mut altered = original.clone();
    altered[12] ^= 1;
    fs::write(&path, altered).unwrap();
    assert!(Journal::reopen(&fixture.0).is_err());
    fs::write(&path, original).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Journal::reopen(&fixture.0).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(Journal::reopen(&fixture.0).is_ok());
    fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Journal::reopen(&fixture.0).is_err());
}
