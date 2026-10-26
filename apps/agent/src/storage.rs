//! Trusted init-container operation. Tool processes never mount this parent directory.
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::{fs, io, path::Path};

pub fn initialize(role: &str) -> io::Result<()> {
    let uid = match role {
        "runtime" => 10001,
        "tools" => 10002,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid storage role",
            ));
        }
    };
    create(Path::new("/storage"), role, uid)
}

fn create(root: &Path, name: &str, uid: u32) -> io::Result<()> {
    let path = root.join(name);
    match fs::DirBuilder::new().mode(0o700).create(&path) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_dir() || metadata.uid() != uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "storage owner or type mismatch",
        ));
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    fs::File::open(&path)?.sync_all()?;
    fs::File::open(root)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn initializes_idempotently_without_erasing_existing_files() {
        let root = temporary();
        let uid = fs::metadata(&root).unwrap().uid();
        create(&root, "runtime", uid).unwrap();
        fs::write(root.join("runtime/journal"), b"retained evidence").unwrap();
        create(&root, "runtime", uid).unwrap();
        assert_eq!(
            fs::read(root.join("runtime/journal")).unwrap(),
            b"retained evidence"
        );
        assert_eq!(
            fs::metadata(root.join("runtime")).unwrap().mode() & 0o777,
            0o700
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_symlinks_foreign_ownership_and_unknown_roles() {
        let root = temporary();
        let uid = fs::metadata(&root).unwrap().uid();
        fs::create_dir(root.join("outside")).unwrap();
        symlink(root.join("outside"), root.join("runtime")).unwrap();
        assert!(create(&root, "runtime", uid).is_err());
        assert!(create(&root, "tools", uid.wrapping_add(1)).is_err());
        assert!(initialize("../outside").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parallel_fixtures_have_independent_directories() {
        let threads: Vec<_> = (0..64).map(|_| std::thread::spawn(temporary)).collect();
        let paths: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(unique.len(), paths.len());
        for path in paths {
            fs::remove_dir(path).unwrap();
        }
    }

    fn temporary() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let name = format!(
            "cubebot-storage-{}-{sequence}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(name);
        fs::create_dir(&root).unwrap();
        root
    }
}
