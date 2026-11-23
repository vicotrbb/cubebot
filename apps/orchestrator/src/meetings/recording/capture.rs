use super::Mark;
use crate::meetings::live::wire::Stream;
use axum::body::Bytes;
use base64::{Engine, engine::general_purpose::STANDARD};
use cubebot_protocol::desktop::Tile;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::sync::mpsc;

const GAP_MS: f64 = 300.0;
const BUCKET_MS: u64 = 500;
const LOUD_RMS: f64 = 600.0;
const VIDEO_HEADER: usize = 5;

pub(crate) enum Capture {
    Name {
        slot: u16,
        name: String,
    },
    Audio {
        slot: u16,
        rate: u32,
        pcm: Bytes,
    },
    Track {
        slot: u16,
        stream: Stream,
        codec: Option<(String, u32, u32)>,
    },
    Frame {
        slot: u16,
        stream: Stream,
        key: bool,
        payload: Bytes,
    },
    Tiles {
        slot: u16,
        tiles: Vec<Tile>,
    },
    DesktopEnded {
        slot: u16,
    },
    Gone {
        slot: u16,
    },
}

struct Voice {
    file: BufWriter<File>,
    written: u64,
    rate: u32,
    cursor: f64,
    loud: Option<u64>,
}

struct Ivf {
    file: BufWriter<File>,
    name: String,
    began: u64,
}

#[derive(Default)]
struct Video {
    codec: Option<(String, u32, u32)>,
    writer: Option<Ivf>,
}

struct Desktop {
    file: BufWriter<File>,
    name: String,
}

pub(super) struct Recorder {
    directory: PathBuf,
    began: Instant,
    marks: BufWriter<File>,
    voices: HashMap<u16, Voice>,
    videos: HashMap<(u16, Stream), Video>,
    desktops: HashMap<u16, Desktop>,
    files: u32,
}

pub(super) fn fourcc(codec: &str) -> Option<&'static [u8; 4]> {
    if codec.starts_with("vp8") {
        Some(b"VP80")
    } else if codec.starts_with("vp09") {
        Some(b"VP90")
    } else if codec.starts_with("avc1") {
        Some(b"H264")
    } else {
        None
    }
}

fn voice_file(slot: u16) -> String {
    format!("a{slot}.pcm")
}

fn rms(pcm: &[u8]) -> f64 {
    let samples = pcm.len() / 2;
    if samples == 0 {
        return 0.0;
    }
    let sum: f64 = pcm
        .chunks_exact(2)
        .map(|pair| f64::from(i16::from_le_bytes([pair[0], pair[1]])).powi(2))
        .sum();
    (sum / samples as f64).sqrt()
}

impl Recorder {
    pub(super) fn create(directory: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        let marks = File::create(directory.join(super::MARKS))?;
        Ok(Self {
            directory: directory.to_path_buf(),
            began: Instant::now(),
            marks: BufWriter::new(marks),
            voices: HashMap::new(),
            videos: HashMap::new(),
            desktops: HashMap::new(),
            files: 0,
        })
    }

    pub(super) fn run(mut self, mut captures: mpsc::Receiver<Capture>) -> io::Result<u64> {
        while let Some(capture) = captures.blocking_recv() {
            self.take(capture)?;
        }
        self.finish()
    }

    fn now(&self) -> u64 {
        u64::try_from(self.began.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn mark(&mut self, mark: &Mark) -> io::Result<()> {
        serde_json::to_writer(&mut self.marks, mark)?;
        self.marks.write_all(b"\n")
    }

    fn take(&mut self, capture: Capture) -> io::Result<()> {
        let at = self.now();
        match capture {
            Capture::Name { slot, name } => self.mark(&Mark::Name { at, slot, name }),
            Capture::Audio { slot, rate, pcm } => self.audio(at, slot, rate, &pcm),
            Capture::Track {
                slot,
                stream,
                codec,
            } => self.track(at, slot, stream, codec),
            Capture::Frame {
                slot,
                stream,
                key,
                payload,
            } => self.frame(at, slot, stream, key, &payload),
            Capture::Tiles { slot, tiles } => self.tiles(at, slot, &tiles),
            Capture::DesktopEnded { slot } => self.end_desktop(at, slot),
            Capture::Gone { slot } => self.gone(at, slot),
        }
    }

    fn next_file(
        &mut self,
        prefix: &str,
        extension: &str,
    ) -> io::Result<(BufWriter<File>, String)> {
        self.files += 1;
        let name = format!("{prefix}-{}.{extension}", self.files);
        let file = File::create(self.directory.join(&name))?;
        Ok((BufWriter::new(file), name))
    }

    fn audio(&mut self, at: u64, slot: u16, rate: u32, pcm: &[u8]) -> io::Result<()> {
        if rate == 0 || pcm.is_empty() {
            return Ok(());
        }
        if !self.voices.contains_key(&slot) {
            let file = File::create(self.directory.join(voice_file(slot)))?;
            let voice = Voice {
                file: BufWriter::new(file),
                written: 0,
                rate,
                cursor: f64::NEG_INFINITY,
                loud: None,
            };
            self.voices.insert(slot, voice);
        }
        let Some(voice) = self.voices.get_mut(&slot) else {
            return Ok(());
        };
        let mut marks = Vec::new();
        if voice.rate != rate || at as f64 > voice.cursor + GAP_MS {
            (voice.rate, voice.cursor) = (rate, at as f64);
            marks.push(Mark::Voice {
                at,
                slot,
                rate,
                file: voice_file(slot),
                offset: voice.written,
            });
        }
        let bucket = voice.cursor as u64 / BUCKET_MS * BUCKET_MS;
        voice.file.write_all(pcm)?;
        voice.written += pcm.len() as u64;
        voice.cursor += (pcm.len() / 2) as f64 * 1000.0 / f64::from(rate);
        if rms(pcm) >= LOUD_RMS && voice.loud != Some(bucket) {
            voice.loud = Some(bucket);
            marks.push(Mark::Loud { at: bucket, slot });
        }
        for mark in &marks {
            self.mark(mark)?;
        }
        Ok(())
    }

    fn track(
        &mut self,
        at: u64,
        slot: u16,
        stream: Stream,
        codec: Option<(String, u32, u32)>,
    ) -> io::Result<()> {
        let video = self.videos.entry((slot, stream)).or_default();
        if video.codec == codec {
            return Ok(());
        }
        video.codec = codec;
        let writer = video.writer.take();
        self.close_video(at, writer)
    }

    fn close_video(&mut self, at: u64, writer: Option<Ivf>) -> io::Result<()> {
        let Some(mut ivf) = writer else {
            return Ok(());
        };
        ivf.file.flush()?;
        let file = ivf.file.get_mut();
        file.seek(SeekFrom::Start(24))?;
        let length = u32::try_from(at.saturating_sub(ivf.began)).unwrap_or(u32::MAX);
        file.write_all(&length.to_le_bytes())?;
        self.mark(&Mark::Stop { at, file: ivf.name })
    }

    fn frame(
        &mut self,
        at: u64,
        slot: u16,
        stream: Stream,
        key: bool,
        payload: &[u8],
    ) -> io::Result<()> {
        let Some(chunk) = payload
            .get(VIDEO_HEADER..)
            .filter(|chunk| !chunk.is_empty())
        else {
            return Ok(());
        };
        let open = self
            .videos
            .get(&(slot, stream))
            .is_some_and(|video| video.writer.is_some());
        if !open && key {
            self.open_video(at, slot, stream)?;
        }
        let Some(ivf) = self
            .videos
            .get_mut(&(slot, stream))
            .and_then(|video| video.writer.as_mut())
        else {
            return Ok(());
        };
        let length = u32::try_from(chunk.len()).unwrap_or(u32::MAX);
        ivf.file.write_all(&length.to_le_bytes())?;
        ivf.file.write_all(&(at - ivf.began).to_le_bytes())?;
        ivf.file.write_all(chunk)?;
        Ok(())
    }

    fn open_video(&mut self, at: u64, slot: u16, stream: Stream) -> io::Result<()> {
        let Some((codec, width, height)) = self
            .videos
            .get(&(slot, stream))
            .and_then(|video| video.codec.clone())
        else {
            return Ok(());
        };
        let Some(fourcc) = fourcc(&codec) else {
            return Ok(());
        };
        let prefix = match stream {
            Stream::Camera => format!("c{slot}"),
            Stream::Screen => format!("s{slot}"),
        };
        let (mut file, name) = self.next_file(&prefix, "ivf")?;
        file.write_all(&ivf_header(fourcc, width, height))?;
        self.mark(&Mark::Video {
            at,
            slot,
            stream,
            file: name.clone(),
        })?;
        let ivf = Ivf {
            file,
            name,
            began: at,
        };
        if let Some(video) = self.videos.get_mut(&(slot, stream)) {
            video.writer = Some(ivf);
        }
        Ok(())
    }

    fn tiles(&mut self, at: u64, slot: u16, tiles: &[Tile]) -> io::Result<()> {
        if !self.desktops.contains_key(&slot) {
            let (file, name) = self.next_file(&format!("d{slot}"), "tiles")?;
            self.mark(&Mark::Desktop {
                at,
                slot,
                file: name.clone(),
            })?;
            self.desktops.insert(slot, Desktop { file, name });
        }
        let Some(desktop) = self.desktops.get_mut(&slot) else {
            return Ok(());
        };
        for tile in tiles {
            let Ok(png) = STANDARD.decode(&tile.png) else {
                continue;
            };
            desktop.file.write_all(&at.to_le_bytes())?;
            desktop.file.write_all(&tile.x.to_le_bytes())?;
            desktop.file.write_all(&tile.y.to_le_bytes())?;
            let length = u32::try_from(png.len()).unwrap_or(u32::MAX);
            desktop.file.write_all(&length.to_le_bytes())?;
            desktop.file.write_all(&png)?;
        }
        Ok(())
    }

    fn end_desktop(&mut self, at: u64, slot: u16) -> io::Result<()> {
        let Some(mut desktop) = self.desktops.remove(&slot) else {
            return Ok(());
        };
        desktop.file.flush()?;
        self.mark(&Mark::Stop {
            at,
            file: desktop.name,
        })
    }

    fn gone(&mut self, at: u64, slot: u16) -> io::Result<()> {
        for stream in [Stream::Camera, Stream::Screen] {
            let writer = self
                .videos
                .remove(&(slot, stream))
                .and_then(|video| video.writer);
            self.close_video(at, writer)?;
        }
        self.end_desktop(at, slot)
    }

    fn finish(mut self) -> io::Result<u64> {
        let at = self.now();
        let writers: Vec<Ivf> = self
            .videos
            .drain()
            .filter_map(|(_, video)| video.writer)
            .collect();
        for writer in writers {
            self.close_video(at, Some(writer))?;
        }
        let slots: Vec<u16> = self.desktops.keys().copied().collect();
        for slot in slots {
            self.end_desktop(at, slot)?;
        }
        for voice in self.voices.values_mut() {
            voice.file.flush()?;
        }
        self.mark(&Mark::End { at })?;
        self.marks.flush()?;
        Ok(at)
    }
}

fn ivf_header(fourcc: &[u8; 4], width: u32, height: u32) -> [u8; 32] {
    let mut header = [0_u8; 32];
    header[..4].copy_from_slice(b"DKIF");
    header[6..8].copy_from_slice(&32_u16.to_le_bytes());
    header[8..12].copy_from_slice(fourcc);
    header[12..14].copy_from_slice(&u16::try_from(width).unwrap_or(u16::MAX).to_le_bytes());
    header[14..16].copy_from_slice(&u16::try_from(height).unwrap_or(u16::MAX).to_le_bytes());
    header[16..20].copy_from_slice(&1000_u32.to_le_bytes());
    header[20..24].copy_from_slice(&1_u32.to_le_bytes());
    header
}
