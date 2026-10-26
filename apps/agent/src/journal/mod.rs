//! Private recovery evidence. A retained record grants no execution authority.
//! Callers must stop starting actions when a journal operation fails.
mod files;
mod format;

use files::{private_file, publish, sync_directory};
use format::{decode_record, decode_state, encode_record, encode_state};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub const MAX_PAYLOAD: usize = 64 * 1024;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_RECORDS: usize = 4096;
const RECORD_HEADER: usize = 44;

pub struct Record {
    pub sequence: u64,
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Default)]
struct State {
    committed: u64,
    acknowledged: u64,
}

pub struct Journal {
    root: PathBuf,
    _lock: fs::File,
    state: State,
    records: Vec<Record>,
    bytes: usize,
    poisoned: bool,
}

impl Journal {
    /// Opens a dedicated private directory and exclusively locks its stable lock file.
    pub fn open(root: &Path) -> io::Result<Self> {
        files::directory(root)?;
        let lock_path = root.join("lock");
        let initialized = fs::symlink_metadata(&lock_path).is_ok();
        let lock = private_file(&lock_path, true)?;
        lock.try_lock()
            .map_err(|_| io::Error::other("journal already locked or locking unavailable"))?;
        let state = load_state(root, initialized)?;
        let mut journal = Self {
            root: root.into(),
            _lock: lock,
            state,
            records: Vec::new(),
            bytes: 0,
            poisoned: false,
        };
        journal.recover()?;
        Ok(journal)
    }

    pub fn pending(&self) -> &[Record] {
        &self.records
    }

    /// Returns only after the record and checkpoint have been synchronized.
    pub fn append(&mut self, payload: &[u8]) -> io::Result<u64> {
        self.healthy()?;
        if payload.len() > MAX_PAYLOAD
            || self.records.len() >= MAX_RECORDS
            || self.bytes + payload.len() + RECORD_HEADER > MAX_BYTES
        {
            return Err(io::Error::other("journal capacity exhausted"));
        }
        let sequence = self
            .state
            .committed
            .checked_add(1)
            .ok_or_else(|| io::Error::other("journal sequence exhausted"))?;
        let next = State {
            committed: sequence,
            ..self.state
        };
        let result = self.commit_record(next, payload);
        self.poisoned = result.is_err();
        result?;
        self.state = next;
        self.bytes += payload.len() + RECORD_HEADER;
        self.records.push(Record {
            sequence,
            payload: payload.to_vec(),
        });
        Ok(sequence)
    }

    /// Only the caller's explicit central acknowledgement permits deletion.
    pub fn acknowledge(&mut self, through: u64) -> io::Result<()> {
        self.healthy()?;
        if through <= self.state.acknowledged {
            return Ok(());
        }
        if through > self.state.committed {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "acknowledgement exceeds committed sequence",
            ));
        }
        let next = State {
            acknowledged: through,
            ..self.state
        };
        let result = self.commit_acknowledgement(next);
        self.poisoned = result.is_err();
        result?;
        self.state = next;
        self.records.retain(|record| record.sequence > through);
        self.bytes = self
            .records
            .iter()
            .map(|record| record.payload.len() + RECORD_HEADER)
            .sum();
        Ok(())
    }

    fn healthy(&self) -> io::Result<()> {
        if self.poisoned {
            return Err(io::Error::other(
                "journal requires reopening after failed I/O",
            ));
        }
        Ok(())
    }

    fn commit_record(&self, next: State, payload: &[u8]) -> io::Result<()> {
        let name = record_name(next.committed);
        if self.root.join(&name).symlink_metadata().is_ok() {
            return Err(corrupt());
        }
        publish(
            &self.root,
            ".entry.pending",
            &name,
            &encode_record(next.committed, payload),
        )?;
        publish(&self.root, ".state.pending", "state", &encode_state(next))
    }

    fn commit_acknowledgement(&self, next: State) -> io::Result<()> {
        publish(&self.root, ".state.pending", "state", &encode_state(next))?;
        for record in self
            .records
            .iter()
            .filter(|record| record.sequence <= next.acknowledged)
        {
            files::remove(&self.root.join(record_name(record.sequence)))?;
        }
        sync_directory(&self.root)
    }

    fn recover(&mut self) -> io::Result<()> {
        // Validate all committed evidence before removing any disposable files.
        let count = self.state.committed - self.state.acknowledged;
        if count > MAX_RECORDS as u64 {
            return Err(corrupt());
        }
        for offset in 0..count {
            let sequence = self.state.acknowledged + offset + 1;
            let data = files::read(
                &self.root.join(record_name(sequence)),
                MAX_PAYLOAD + RECORD_HEADER,
            )?;
            let record = decode_record(sequence, &data)?;
            self.bytes += data.len();
            if self.bytes > MAX_BYTES {
                return Err(corrupt());
            }
            self.records.push(record);
        }
        self.cleanup()?;
        sync_directory(&self.root)
    }

    fn cleanup(&self) -> io::Result<()> {
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(corrupt)?;
            match name {
                "state" | "lock" => (),
                ".entry.pending" | ".state.pending" => files::remove(&entry.path())?,
                _ => self.cleanup_record(name, &entry.path())?,
            }
        }
        Ok(())
    }

    fn cleanup_record(&self, name: &str, path: &Path) -> io::Result<()> {
        let sequence = name
            .strip_suffix(".entry")
            .and_then(|value| u64::from_str_radix(value, 16).ok())
            .ok_or_else(corrupt)?;
        if sequence == 0 || record_name(sequence) != name {
            return Err(corrupt());
        }
        if sequence <= self.state.acknowledged || sequence > self.state.committed {
            files::remove(path)?;
        }
        Ok(())
    }
}

fn load_state(root: &Path, initialized: bool) -> io::Result<State> {
    match files::read(&root.join("state"), 56) {
        Ok(data) => decode_state(&data),
        Err(error) if error.kind() == io::ErrorKind::NotFound && !initialized => {
            initialize_state(root)
        }
        Err(error) => Err(error),
    }
}

fn initialize_state(root: &Path) -> io::Result<State> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        match entry.file_name().to_str() {
            Some("lock") => (),
            Some(".state.pending") => files::remove(&entry.path())?,
            _ => return Err(corrupt()),
        }
    }
    let state = State::default();
    publish(root, ".state.pending", "state", &encode_state(state))?;
    Ok(state)
}

fn record_name(sequence: u64) -> String {
    format!("{sequence:016x}.entry")
}
fn corrupt() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid or missing journal evidence",
    )
}

#[cfg(test)]
mod tests;
