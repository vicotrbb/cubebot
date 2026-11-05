//! The embedded speech path on real models: `just speech-check [directory]`. Downloads them through
//! the production model store, hears a human recording through Silero and Zipformer, speaks with
//! both Piper voices and hears that back. Prints what docs/STATUS.md records, and fails on a wrong
//! transcript.
use cubebot_orchestrator::speech::{
    Audio, Heard, LESSAC, Listener, Models, Piper, RYAN, SAMPLE_RATE, SILERO, Silero, Tts,
    ZIPFORMER, Zipformer, piper, recognizer,
};
use sherpa_onnx::OnlineRecognizer;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

const SENTENCE: &str = "The weather in Paris is lovely this morning.";
// Not "weather": Piper varies between runs and Zipformer sometimes hears its homophone.
const HEARD_BACK: [&str; 2] = ["PARIS", "MORNING"];
/// What the bundled recording says, from its trans.txt.
const RECORDED: [&str; 2] = [
    "AFTER EARLY NIGHTFALL THE YELLOW LAMPS",
    "QUARTER OF THE BROTHELS",
];

fn rss_mb() -> u64 {
    let pid = std::process::id().to_string();
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .expect("ps runs");
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim().parse::<u64>().expect("ps prints kilobytes") / 1024
}

/// Hears `samples` in 100 ms frames, then silence enough to end the sentence. Returns the
/// transcripts, how many partials came, and the time spent hearing the speech alone.
fn hear(
    directory: &Path,
    recognizer: &Arc<OnlineRecognizer>,
    samples: &[f32],
) -> (String, usize, f64) {
    let vad = Silero::load(directory).expect("silero loads");
    let stt = Zipformer::new(recognizer.clone());
    let mut listener = Listener::new(Box::new(vad), Box::new(stt));
    let frame = SAMPLE_RATE as usize / 10;
    let (mut text, mut partials) = (String::new(), 0);
    let mut note = |heard: Vec<Heard>| {
        for item in heard {
            match item {
                Heard::Partial(_) => partials += 1,
                Heard::Utterance(said) => text = format!("{text} {said}"),
                Heard::Speech => {}
                Heard::Failed(error) => panic!("{error}"),
            }
        }
    };
    let began = Instant::now();
    for chunk in samples.chunks(frame) {
        note(listener.hear(chunk));
    }
    let spent = began.elapsed().as_secs_f64();
    for _ in 0..15 {
        note(listener.hear(&vec![0.0; frame]));
    }
    (text.trim().to_uppercase(), partials, spent)
}

fn resampled(samples: &[f32], rate: u32) -> Vec<f32> {
    let ratio = f64::from(rate) / f64::from(SAMPLE_RATE);
    let length = (samples.len() as f64 / ratio) as usize;
    (0..length)
        .map(|at| samples[(at as f64 * ratio) as usize])
        .collect()
}

/// Speaks, timing the first audio as a call would hear it, and returns the voice at 16 kHz.
fn speak(name: &str, tts: &Piper) -> Vec<f32> {
    let began = Instant::now();
    let mut first = None;
    let (mut pcm, mut rate) = (Vec::new(), 0);
    tts.speak(SENTENCE, &mut |audio: Audio| {
        first.get_or_insert_with(|| began.elapsed());
        rate = audio.rate;
        pcm.extend(audio.pcm);
        true
    })
    .expect("piper speaks");
    let spent = began.elapsed().as_secs_f64();
    let samples: Vec<f32> = pcm
        .chunks_exact(2)
        .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
        .collect();
    let seconds = samples.len() as f64 / f64::from(rate);
    println!(
        "TTS {name}: {seconds:.2} s of audio at {rate} Hz, first audio {:.0} ms, RTF {:.3}",
        first.expect("piper produced audio").as_secs_f64() * 1000.0,
        spent / seconds,
    );
    resampled(&samples, rate)
}

/// Calls share one recognizer and voice, each stream on its own thread.
fn together(directory: &Path, recognizer: &Arc<OnlineRecognizer>, samples: &[f32], voice: &Piper) {
    std::thread::scope(|scope| {
        for _ in 0..3 {
            scope.spawn(|| {
                let (text, _, _) = hear(directory, recognizer, samples);
                assert!(
                    text.contains(RECORDED[0]),
                    "three streams at once: {text:?}"
                );
                let mut produced = 0;
                voice
                    .speak(SENTENCE, &mut |audio: Audio| {
                        produced += audio.pcm.len();
                        true
                    })
                    .expect("piper speaks");
                assert!(produced > 16_000, "three voices at once: {produced} bytes");
            });
        }
    });
    println!("three calls at once heard and spoke correctly");
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let root = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("target/speech-models"), PathBuf::from);
    let models = Models::new(root);
    let report = |name: &'static str, received: u64, total: u64| {
        eprintln!("{name}: {} of {} MB", received >> 20, total >> 20);
    };
    let began = Instant::now();
    let silero = models
        .ensure(&SILERO, &report)
        .await
        .expect("silero downloads");
    let zipformer = models
        .ensure(&ZIPFORMER, &report)
        .await
        .expect("zipformer downloads");
    let lessac = models
        .ensure(&LESSAC, &report)
        .await
        .expect("lessac downloads");
    let ryan = models.ensure(&RYAN, &report).await.expect("ryan downloads");
    println!("models ready in {:.1} s", began.elapsed().as_secs_f64());
    println!("RSS before loading: {} MB", rss_mb());

    let loading = Instant::now();
    let recognizer = recognizer(&zipformer).expect("zipformer loads");
    println!(
        "STT loaded in {:.2} s, RSS {} MB",
        loading.elapsed().as_secs_f64(),
        rss_mb()
    );

    let recording = sherpa_onnx::Wave::read(zipformer.join("test_wavs/0.wav").to_str().unwrap())
        .expect("the bundled recording reads");
    assert_eq!(recording.sample_rate(), SAMPLE_RATE as i32);
    let seconds = recording.samples().len() as f64 / f64::from(SAMPLE_RATE);
    let (text, partials, spent) = hear(&silero, &recognizer, recording.samples());
    println!(
        "STT+VAD on a human recording: {seconds:.2} s of audio, RTF {:.3} (about {:.1}% of a core), {partials} partials",
        spent / seconds,
        100.0 * spent / seconds,
    );
    println!("heard: {text}");
    for words in RECORDED {
        assert!(text.contains(words), "expected {words:?} in {text:?}");
    }
    println!("RSS with STT and one stream: {} MB", rss_mb());

    let loading = Instant::now();
    let female = Piper(piper(&lessac, LESSAC.file).expect("lessac loads"));
    println!(
        "Female voice loaded in {:.2} s, RSS {} MB",
        loading.elapsed().as_secs_f64(),
        rss_mb()
    );
    let male = Piper(piper(&ryan, RYAN.file).expect("ryan loads"));
    println!("Male voice loaded, RSS {} MB", rss_mb());
    for (name, voice) in [("Female", &female), ("Male", &male)] {
        let spoken = speak(name, voice);
        let (back, _, _) = hear(&silero, &recognizer, &spoken);
        println!("{name} voice heard back: {back}");
        for word in HEARD_BACK {
            assert!(back.contains(word), "expected {word:?} in {back:?}");
        }
    }
    together(&silero, &recognizer, recording.samples(), &female);
    println!("RSS after everything: {} MB", rss_mb());
    println!("embedded speech passed");
}
