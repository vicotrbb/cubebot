use super::{Record, State, corrupt};
use sha2::{Digest, Sha256};
use std::io;

const MAGIC: &[u8; 8] = b"CBJNL001";

pub(super) fn encode_state(state: State) -> Vec<u8> {
    let mut bytes = MAGIC.to_vec();
    bytes.extend(state.committed.to_be_bytes());
    bytes.extend(state.acknowledged.to_be_bytes());
    seal(bytes)
}

pub(super) fn decode_state(bytes: &[u8]) -> io::Result<State> {
    if bytes.len() != 56 || &bytes[..8] != MAGIC {
        return Err(corrupt());
    }
    verify(bytes)?;
    let committed = u64::from_be_bytes(bytes[8..16].try_into().map_err(|_| corrupt())?);
    let acknowledged = u64::from_be_bytes(bytes[16..24].try_into().map_err(|_| corrupt())?);
    if acknowledged > committed {
        return Err(corrupt());
    }
    Ok(State {
        committed,
        acknowledged,
    })
}

pub(super) fn encode_record(sequence: u64, payload: &[u8]) -> Vec<u8> {
    let mut bytes = sequence.to_be_bytes().to_vec();
    bytes.extend((payload.len() as u32).to_be_bytes());
    bytes.extend(payload);
    seal(bytes)
}

pub(super) fn decode_record(sequence: u64, bytes: &[u8]) -> io::Result<Record> {
    if bytes.len() < super::RECORD_HEADER {
        return Err(corrupt());
    }
    verify(bytes)?;
    let recorded = u64::from_be_bytes(bytes[..8].try_into().map_err(|_| corrupt())?);
    let length = u32::from_be_bytes(bytes[8..12].try_into().map_err(|_| corrupt())?) as usize;
    if recorded != sequence
        || length > super::MAX_PAYLOAD
        || length + super::RECORD_HEADER != bytes.len()
    {
        return Err(corrupt());
    }
    Ok(Record {
        sequence,
        payload: bytes[12..12 + length].to_vec(),
    })
}

fn seal(mut bytes: Vec<u8>) -> Vec<u8> {
    let checksum = Sha256::digest(&bytes);
    bytes.extend(checksum);
    bytes
}

fn verify(bytes: &[u8]) -> io::Result<()> {
    let split = bytes.len().checked_sub(32).ok_or_else(corrupt)?;
    if Sha256::digest(&bytes[..split])[..] != bytes[split..] {
        return Err(corrupt());
    }
    Ok(())
}
