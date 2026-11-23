use super::Mark;
use super::plan::{Focus, Look, Piece, Run, Timeline, Visual, plan};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};

const FPS: u64 = 24;
const RATE: u64 = 48_000;
const BLOCK: u64 = RATE * 10;
const SIZE: &str = "1280x720";
const GROUND: &str = "0x201a16";
const DESKTOP: (usize, usize) = (1280, 800);
const DESKTOP_FPS: u64 = 4;
const OUTPUT: &str = "recording.mp4";
const LOG: &str = "ffmpeg.log";
const FONT: &str = "/usr/share/fonts/cubebot/rubik.woff2";
const WRAP: &str =
    "{ echo 1000 > /proc/self/oom_score_adj; } 2>/dev/null; exec nice -n 10 ffmpeg \"$@\"";
const ENCODE: [&str; 20] = [
    "-c:v",
    "libx264",
    "-preset",
    "veryfast",
    "-crf",
    "23",
    "-pix_fmt",
    "yuv420p",
    "-r",
    "24",
    "-g",
    "96",
    "-threads",
    "2",
    "-video_track_timescale",
    "24000",
    "-an",
    "-movflags",
    "+faststart",
    "-y",
];

#[derive(Clone, Debug)]
pub struct Tools {
    font: Option<PathBuf>,
}

impl Tools {
    pub fn detect() -> Option<Self> {
        let filters = Command::new("ffmpeg")
            .args(["-hide_banner", "-filters"])
            .stderr(Stdio::null())
            .output()
            .inspect_err(
                |error| tracing::warn!(%error, "ffmpeg is missing; meetings cannot record"),
            )
            .ok()?;
        let listed = String::from_utf8_lossy(&filters.stdout);
        let font = Path::new(FONT);
        let labels = listed.contains(" drawtext ") && font.exists();
        if !labels {
            tracing::warn!(
                "recordings carry no name labels: ffmpeg drawtext or the font is missing"
            );
        }
        Some(Self {
            font: labels.then(|| font.to_path_buf()),
        })
    }
}

fn frame(at: u64) -> u64 {
    (at * FPS + 500) / 1000
}

fn marks(directory: &Path) -> io::Result<Vec<Mark>> {
    let file = BufReader::new(File::open(directory.join(super::MARKS))?);
    Ok(file
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect())
}

pub(super) struct Job<'a> {
    pub directory: &'a Path,
    pub title: &'a str,
    pub tools: &'a Tools,
    pub progress: &'a AtomicU8,
}

impl Job<'_> {
    pub(super) fn run(&self) -> io::Result<(PathBuf, u64)> {
        let marks = marks(self.directory)
            .inspect_err(|error| tracing::warn!(%error, "recording marks unreadable"))
            .map_err(|error| io::Error::new(error.kind(), "the captured media is missing"))?;
        let timeline = Timeline::read(marks);
        let frames = frame(timeline.duration);
        if frames == 0 {
            return Err(io::Error::other("nothing was recorded"));
        }
        mix(self.directory, &timeline.runs, frames * (RATE / FPS))?;
        let pieces = plan(&timeline);
        let mut list = String::new();
        for (index, piece) in pieces.iter().enumerate() {
            if frame(piece.to) == frame(piece.from) {
                continue;
            }
            let name = format!("piece-{index:05}.mp4");
            self.piece(&timeline, index, piece, &name)?;
            list.push_str(&format!("file '{name}'\n"));
            let done = (index + 1) * 95 / pieces.len();
            self.progress
                .store(u8::try_from(done).unwrap_or(95), Ordering::Relaxed);
        }
        std::fs::write(self.directory.join("pieces.txt"), list)?;
        self.ffmpeg(&[
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            "pieces.txt",
            "-f",
            "s16le",
            "-ar",
            "48000",
            "-ac",
            "1",
            "-i",
            "mix.pcm",
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-movflags",
            "+faststart",
            "-shortest",
            "-y",
            OUTPUT,
        ])?;
        Ok((self.directory.join(OUTPUT), timeline.duration))
    }

    fn text(&self, name: &str, text: &str) -> io::Result<String> {
        let path = self.directory.join(name);
        std::fs::write(&path, text)?;
        Ok(path.to_string_lossy().into_owned())
    }

    fn label(&self, piece: usize, text: Option<String>) -> io::Result<String> {
        let (Some(font), Some(text)) = (&self.tools.font, text) else {
            return Ok(String::new());
        };
        let file = self.text(&format!("label-{piece}.txt"), &text)?;
        Ok(format!(
            ",drawtext=textfile={file}:expansion=none:fontfile={}:fontsize=24:fontcolor=white:x=28:y=h-th-28:box=1:boxcolor=black@0.55:boxborderw=12",
            font.display()
        ))
    }

    fn slate(&self, piece: usize, name: Option<String>) -> io::Result<String> {
        let Some(font) = &self.tools.font else {
            return Ok(String::new());
        };
        let font = font.display();
        let title = self.text(&format!("title-{piece}.txt"), self.title)?;
        let mut filter = format!(
            ",drawtext=textfile={title}:expansion=none:fontfile={font}:fontsize=26:fontcolor=0xa3968b:x=(w-tw)/2:y=h/2+48"
        );
        if let Some(name) = name {
            let name = self.text(&format!("name-{piece}.txt"), &name)?;
            filter.push_str(&format!(
                ",drawtext=textfile={name}:expansion=none:fontfile={font}:fontsize=64:fontcolor=0xf3ece2:x=(w-tw)/2:y=h/2-th"
            ));
        }
        Ok(filter)
    }

    fn piece(&self, timeline: &Timeline, at: usize, piece: &Piece, name: &str) -> io::Result<()> {
        let ground = format!("color=c={GROUND}:s={SIZE}:r={FPS}");
        let mut args: Vec<String> = ["-f", "lavfi", "-i", &ground].map(String::from).to_vec();
        let filter = match piece.focus {
            Focus::Slate(slot) => {
                let speaker = slot.and_then(|slot| timeline.names.get(&slot).cloned());
                format!("[0:v]null{}[v]", self.slate(at, speaker)?)
            }
            Focus::Visual(index) => {
                args.extend(self.input(&timeline.visuals[index], piece));
                let label = self.label(at, timeline.label(piece.focus))?;
                format!(
                    "[1:v]fps={FPS},scale={SIZE}:force_original_aspect_ratio=decrease,setsar=1[fg];[0:v][fg]overlay=(W-w)/2:(H-h)/2:eof_action=repeat{label}[v]"
                )
            }
        };
        let frames = (frame(piece.to) - frame(piece.from)).to_string();
        args.extend(
            [
                "-filter_complex",
                &filter,
                "-map",
                "[v]",
                "-frames:v",
                &frames,
            ]
            .map(String::from),
        );
        args.extend(ENCODE.map(String::from));
        args.push(name.into());
        let Focus::Visual(index) = piece.focus else {
            return self.ffmpeg(&args);
        };
        let visual = &timeline.visuals[index];
        if visual.look != Look::Desktop {
            return self.ffmpeg(&args);
        }
        let mut child = self.spawn(&args, Stdio::piped())?;
        if let Some(stdin) = child.stdin.take() {
            let replay = Replay::open(&self.directory.join(&visual.file))?;
            let _ = replay.play(BufWriter::new(stdin), piece.from, piece.to);
        }
        self.finish(child)
    }

    fn input(&self, visual: &Visual, piece: &Piece) -> Vec<String> {
        if visual.look == Look::Desktop {
            let size = format!("{}x{}", DESKTOP.0, DESKTOP.1);
            let rate = DESKTOP_FPS.to_string();
            return [
                "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", &size, "-r", &rate, "-i", "pipe:0",
            ]
            .map(String::from)
            .to_vec();
        }
        let offset = piece.from.saturating_sub(visual.from) as f64 / 1000.0;
        let offset = format!("{offset:.3}");
        ["-ss", &offset, "-i", &visual.file]
            .map(String::from)
            .to_vec()
    }

    fn spawn<S: AsRef<str>>(&self, args: &[S], stdin: Stdio) -> io::Result<Child> {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.directory.join(LOG))?;
        Command::new("sh")
            .arg("-c")
            .arg(WRAP)
            .arg("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error"])
            .args(args.iter().map(AsRef::as_ref))
            .current_dir(self.directory)
            .stdin(stdin)
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
    }

    fn finish(&self, mut child: Child) -> io::Result<()> {
        if child.wait()?.success() {
            return Ok(());
        }
        let log = std::fs::read_to_string(self.directory.join(LOG)).unwrap_or_default();
        let last = log.lines().last().unwrap_or("ffmpeg failed").trim();
        Err(io::Error::other(last.chars().take(300).collect::<String>()))
    }

    fn ffmpeg<S: AsRef<str>>(&self, args: &[S]) -> io::Result<()> {
        let child = self.spawn(args, Stdio::null())?;
        self.finish(child)
    }
}

fn lengths(directory: &Path, runs: &[Run]) -> io::Result<Vec<u64>> {
    let mut ends: HashMap<&str, u64> = HashMap::new();
    for run in runs {
        if !ends.contains_key(run.file.as_str()) {
            ends.insert(
                &run.file,
                std::fs::metadata(directory.join(&run.file))?.len(),
            );
        }
    }
    Ok(runs
        .iter()
        .map(|run| {
            let next = runs
                .iter()
                .filter(|other| other.file == run.file && other.offset > run.offset)
                .map(|other| other.offset)
                .min();
            next.unwrap_or(ends[run.file.as_str()])
                .saturating_sub(run.offset)
        })
        .collect())
}

struct Source<'a> {
    run: &'a Run,
    file: File,
    samples: u64,
    start: u64,
    end: u64,
}

impl Source<'_> {
    fn add(&mut self, block: u64, mixed: &mut [i32]) -> io::Result<()> {
        let from = self.start.max(block);
        let to = self.end.min(block + mixed.len() as u64);
        if from >= to {
            return Ok(());
        }
        let step = f64::from(self.run.rate) / RATE as f64;
        let first = ((from - self.start) as f64 * step) as u64;
        let last = (((to - self.start) as f64 * step) as u64 + 1).min(self.samples - 1);
        let mut bytes = vec![0_u8; ((last - first + 1) * 2) as usize];
        self.file
            .seek(SeekFrom::Start(self.run.offset + first * 2))?;
        self.file.read_exact(&mut bytes)?;
        let sample = |index: usize| {
            let at = index * 2;
            bytes.get(at..at + 2).map_or(0.0, |pair| {
                f64::from(i16::from_le_bytes([pair[0], pair[1]]))
            })
        };
        for out in from..to {
            let position = (out - self.start) as f64 * step - first as f64;
            let below = position.floor();
            let fraction = position - below;
            let below = below as usize;
            let value = sample(below) * (1.0 - fraction) + sample(below + 1) * fraction;
            mixed[(out - block) as usize] += value as i32;
        }
        Ok(())
    }
}

fn mix(directory: &Path, runs: &[Run], total: u64) -> io::Result<()> {
    let lengths = lengths(directory, runs)?;
    let mut sources = Vec::new();
    for (run, bytes) in runs.iter().zip(lengths) {
        let samples = bytes / 2;
        if samples < 2 || run.rate == 0 {
            continue;
        }
        let start = run.at * RATE / 1000;
        let end = start + samples * RATE / u64::from(run.rate);
        let file = File::open(directory.join(&run.file))?;
        sources.push(Source {
            run,
            file,
            samples,
            start,
            end,
        });
    }
    let mut out = BufWriter::new(File::create(directory.join("mix.pcm"))?);
    let mut block = 0;
    while block < total {
        let mut mixed = vec![0_i32; (BLOCK.min(total - block)) as usize];
        for source in &mut sources {
            source.add(block, &mut mixed)?;
        }
        for value in mixed {
            let clipped = value.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
            out.write_all(&clipped.to_le_bytes())?;
        }
        block += BLOCK;
    }
    out.flush()
}

struct Tile {
    at: u64,
    x: usize,
    y: usize,
    png: Vec<u8>,
}

struct Replay {
    reader: BufReader<File>,
    canvas: Vec<u8>,
    pending: Option<Tile>,
}

impl Replay {
    fn open(path: &Path) -> io::Result<Self> {
        let mut replay = Self {
            reader: BufReader::new(File::open(path)?),
            canvas: vec![0; DESKTOP.0 * DESKTOP.1 * 3],
            pending: None,
        };
        replay.pending = replay.next();
        Ok(replay)
    }

    fn next(&mut self) -> Option<Tile> {
        let mut head = [0_u8; 20];
        self.reader.read_exact(&mut head).ok()?;
        let word =
            |at: usize| u32::from_le_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]);
        let at = u64::from_le_bytes(head[..8].try_into().ok()?);
        let mut png = vec![0; word(16) as usize];
        self.reader.read_exact(&mut png).ok()?;
        Some(Tile {
            at,
            x: word(8) as usize,
            y: word(12) as usize,
            png,
        })
    }

    fn advance(&mut self, until: u64) {
        while let Some(tile) = self.pending.take_if(|tile| tile.at <= until) {
            self.paint(&tile);
            self.pending = self.next();
        }
    }

    fn paint(&mut self, tile: &Tile) {
        let mut decoder = png::Decoder::new(io::Cursor::new(&tile.png));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let Ok(mut reader) = decoder.read_info() else {
            return;
        };
        let Some(size) = reader.output_buffer_size() else {
            return;
        };
        let mut pixels = vec![0; size];
        let Ok(info) = reader.next_frame(&mut pixels) else {
            return;
        };
        let channels = info.color_type.samples();
        let (width, height) = (info.width as usize, info.height as usize);
        for row in 0..height.min(DESKTOP.1.saturating_sub(tile.y)) {
            for column in 0..width.min(DESKTOP.0.saturating_sub(tile.x)) {
                let from = (row * width + column) * channels;
                let to = ((tile.y + row) * DESKTOP.0 + tile.x + column) * 3;
                let gray = channels < 3;
                for channel in 0..3 {
                    let source = if gray { from } else { from + channel };
                    self.canvas[to + channel] = pixels[source];
                }
            }
        }
    }

    fn play(mut self, mut out: impl Write, from: u64, to: u64) -> io::Result<()> {
        let spacing = 1000 / DESKTOP_FPS;
        let mut at = from;
        while at <= to + spacing {
            self.advance(at);
            out.write_all(&self.canvas)?;
            at += spacing;
        }
        out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_land_on_a_fixed_grid_so_pieces_add_up_to_the_whole() {
        let cuts = [0, 1234, 5000, 5021, 9999];
        let total: u64 = cuts
            .windows(2)
            .map(|pair| frame(pair[1]) - frame(pair[0]))
            .sum();
        assert_eq!(total, frame(9999));
    }

    #[test]
    fn voices_at_different_rates_are_placed_on_one_timeline_and_summed() {
        let directory = tempdir();
        let tone = |value: i16, samples: usize| -> Vec<u8> {
            std::iter::repeat_n(value.to_le_bytes(), samples)
                .flatten()
                .collect()
        };
        std::fs::write(directory.join("a1.pcm"), tone(1000, 16_000)).unwrap();
        std::fs::write(directory.join("a2.pcm"), tone(500, 24_000)).unwrap();
        let runs = [
            Run {
                file: "a1.pcm".into(),
                offset: 0,
                rate: 16_000,
                at: 0,
            },
            Run {
                file: "a2.pcm".into(),
                offset: 0,
                rate: 24_000,
                at: 500,
            },
        ];
        mix(&directory, &runs, RATE * 2).unwrap();
        let mixed = std::fs::read(directory.join("mix.pcm")).unwrap();
        let at = |second: f64| {
            let index = (second * RATE as f64) as usize * 2;
            i16::from_le_bytes([mixed[index], mixed[index + 1]])
        };
        assert_eq!(mixed.len() as u64, RATE * 2 * 2);
        assert_eq!(at(0.25), 1000);
        assert_eq!(at(0.75), 1500);
        assert_eq!(at(1.25), 500);
        assert_eq!(at(1.75), 0);
    }

    fn tempdir() -> PathBuf {
        let directory = std::env::temp_dir().join(format!("cubebot-mix-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }
}
