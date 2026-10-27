use cubebot_orchestrator::connections::Credentials;

#[test]
fn credentials_require_the_correct_key_and_record_context() {
    let keys = Credentials::new([7; 32]);
    let encrypted = keys.encrypt(b"provider-key", b"provider:1").unwrap();
    assert_ne!(encrypted, b"provider-key");
    assert_eq!(
        keys.decrypt(&encrypted, b"provider:1").unwrap(),
        b"provider-key"
    );
    assert!(keys.decrypt(&encrypted, b"provider:2").is_err());
    assert!(
        Credentials::new([8; 32])
            .decrypt(&encrypted, b"provider:1")
            .is_err()
    );
}

#[test]
fn encryption_is_randomized_and_tampering_fails_closed() {
    let keys = Credentials::new([7; 32]);
    let first = keys.encrypt(b"provider-key", b"provider:1").unwrap();
    let mut second = keys.encrypt(b"provider-key", b"provider:1").unwrap();
    assert_ne!(first, second);
    *second.last_mut().unwrap() ^= 1;
    assert!(keys.decrypt(&second, b"provider:1").is_err());
    assert!(keys.decrypt(&[], b"provider:1").is_err());
}

#[test]
fn projected_private_keys_are_read_without_accepting_missing_or_public_keys() {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};
    let root = std::env::temp_dir().join(format!("cubebot-key-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let private = root.join("key");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&private)
        .unwrap();
    file.write_all(&[7; 32]).unwrap();
    symlink(&private, root.join("projected")).unwrap();
    let encrypted = Credentials::new([7; 32])
        .encrypt(b"secret", b"context")
        .unwrap();
    assert_eq!(
        Credentials::from_file(&root.join("projected"))
            .unwrap()
            .decrypt(&encrypted, b"context")
            .unwrap(),
        b"secret"
    );
    assert!(Credentials::from_file(&root.join("missing")).is_err());
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Credentials::from_file(&private).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
