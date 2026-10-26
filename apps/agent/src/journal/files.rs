use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
};

pub(super) fn directory(path: &Path) -> io::Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {
            super::files::sync_directory(path.parent().ok_or_else(super::corrupt)?)?;
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.mode() & 0o077 != 0 {
        return Err(super::corrupt());
    }
    Ok(())
}

pub(super) fn private_file(path: &Path, create: bool) -> io::Result<File> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() => return Err(super::corrupt()),
        Ok(_) => (),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => return Err(error),
    }
    let file = if create {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                OpenOptions::new().read(true).write(true).open(path)?
            }
            Err(error) => return Err(error),
        }
    } else {
        OpenOptions::new().read(true).open(path)?
    };
    validate(path, &file)?;
    Ok(file)
}

fn validate(path: &Path, file: &File) -> io::Result<()> {
    let actual = file.metadata()?;
    let named = fs::symlink_metadata(path)?;
    let parent = fs::symlink_metadata(path.parent().ok_or_else(super::corrupt)?)?;
    if !named.is_file()
        || actual.dev() != named.dev()
        || actual.ino() != named.ino()
        || actual.nlink() != 1
        || actual.uid() != parent.uid()
        || actual.mode() & 0o077 != 0
    {
        return Err(super::corrupt());
    }
    Ok(())
}

pub(super) fn read(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let file = private_file(path, false)?;
    let mut data = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut data)?;
    if data.len() > limit {
        return Err(super::corrupt());
    }
    Ok(data)
}

pub(super) fn publish(root: &Path, temporary: &str, name: &str, data: &[u8]) -> io::Result<()> {
    let path = root.join(temporary);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(data)?;
    file.sync_all()?;
    fs::rename(path, root.join(name))?;
    sync_directory(root)
}

pub(super) fn remove(path: &Path) -> io::Result<()> {
    let _file = private_file(path, false)?;
    fs::remove_file(path)
}

pub(super) fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}
