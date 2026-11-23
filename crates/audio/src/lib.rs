//! L4: sound for play mode.
//!
//! A [`Clip`] is a sound file decoded once (craft-audio: WAV, AIFF, MP3, Opus, AAC and others, and
//! Ogg Vorbis) and turned into stereo at the output's rate; a long sound may instead be played
//! straight from its file ([`stream`]). The [`Mixer`] plays them as voices:
//! each with a volume, looping or not, and, when spatial, quieter with distance and panned by
//! where it is from the listener. [`Audio`] feeds the mixer to the default output device on a
//! thread of its own (cpal), or, where there is no device, keeps the voices going silently so play
//! behaves the same in tests and on servers.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
#[cfg(not(target_arch = "wasm32"))]
use std::thread::JoinHandle;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use bayleaf_core::StableId;
use bayleaf_core::math::DVec3;

pub mod stream;

pub use stream::{Streamed, can_stream};

/// A sound to play: decoded whole, or read from its file as it plays.
#[derive(Clone, Debug)]
pub enum Sound {
    Loaded(Arc<Clip>),
    Streamed(Arc<Streamed>),
}

/// Longest sound one clip may hold, in seconds.
pub const MAX_CLIP_SECONDS: f64 = 600.0;
/// Most voices playing at once; asking for more is refused.
pub const MAX_VOICES: usize = 64;
/// The rate the mixer runs at when there is no output device.
pub const SILENT_RATE: u32 = 48_000;
/// Largest sound file read, in bytes.
pub const MAX_FILE_BYTES: u64 = 256 << 20;
/// The file extensions clips are read from.
pub const EXTENSIONS: &[&str] = &["wav", "ogg", "flac", "mp3", "aif", "aiff", "opus", "m4a", "aac"];

/// A decoded sound, stereo at the mixer's rate.
#[derive(Debug)]
pub struct Clip {
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Clip {
    /// Decodes a sound file's `bytes` (`extension` says which kind) to stereo at `rate`: mono
    /// plays on both sides, more than two channels keep the first two.
    pub fn decode(bytes: &[u8], extension: &str, rate: u32) -> Result<Self, String> {
        let (source_rate, mut channels) = if extension.eq_ignore_ascii_case("ogg") {
            let decoded = craft_audio::vorbis::decoder::decode_ogg(bytes).map_err(|e| e.to_string())?;
            (decoded.sample_rate, decoded.channels)
        } else if extension.eq_ignore_ascii_case("flac") {
            let decoded = craft_audio::flac::decode(bytes).map_err(|e| e.to_string())?;
            // Whole numbers of the stream's bit depth, to -1 to 1.
            let scale = 1.0 / (1u64 << (decoded.info.bits_per_sample.clamp(1, 32) - 1)) as f32;
            (decoded.info.sample_rate, decoded.channels.into_iter().map(|c| c.into_iter().map(|v| v as f32 * scale).collect()).collect())
        } else {
            let (_, buffer) = craft_audio::decode(bytes, Some(extension)).map_err(|e| e.to_string())?;
            (buffer.sample_rate, buffer.channels)
        };
        let frames = channels.first().map_or(0, Vec::len);
        if frames == 0 || source_rate == 0 {
            return Err("the file holds no sound".to_owned());
        }
        if frames as f64 / f64::from(source_rate) > MAX_CLIP_SECONDS {
            return Err(format!("the sound is longer than {MAX_CLIP_SECONDS} seconds"));
        }
        channels.truncate(2);
        let left = channels.remove(0);
        let right = if channels.is_empty() { left.clone() } else { channels.remove(0) };
        if source_rate == rate {
            return Ok(Self { left, right });
        }
        let mut out =
            craft_audio_dsp::offline::resample(&[&left, &right], source_rate, rate).ok_or_else(|| format!("cannot play a sound at {source_rate} Hz"))?;
        if out.len() != 2 {
            return Err("cannot resample the sound".to_owned());
        }
        let right = out.remove(1);
        let left = out.remove(0);
        Ok(Self { left, right })
    }

    /// How many frames (stereo samples) it has.
    pub fn frames(&self) -> usize {
        self.left.len()
    }
}

/// Where a spatial voice is, and how far it is heard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub at: DVec3,
    /// Beyond this distance it is silent; it fades out toward it.
    pub range: f64,
}

/// Where sounds are heard from: a position and the direction to its right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub at: DVec3,
    pub right: DVec3,
}

impl Default for Listener {
    fn default() -> Self {
        Self { at: DVec3::ZERO, right: DVec3::X }
    }
}

/// One clip playing.
#[derive(Debug)]
pub struct Voice {
    pub id: u64,
    /// The object whose sound it is, if any.
    pub owner: Option<StableId>,
    /// Where it came from: a file path, for listings.
    pub source: String,
    playing: Playing,
    /// Frames played, counted from its start (or its last loop for a loaded clip).
    position: usize,
    pub looping: bool,
    pub volume: f32,
    /// `None` plays the same on both sides at any distance.
    pub place: Option<Place>,
    gains: [f32; 2],
}

/// What a voice plays from.
#[derive(Debug)]
enum Playing {
    Clip(Arc<Clip>),
    Stream(Box<stream::StreamVoice>),
}

impl Voice {
    /// Seconds into the clip, at `rate`.
    pub fn seconds(&self, rate: u32) -> f64 {
        self.position as f64 / f64::from(rate.max(1))
    }
}

/// The voices playing, mixed into stereo.
#[derive(Debug)]
pub struct Mixer {
    voices: Vec<Voice>,
    next: u64,
    listener: Listener,
    /// The output's rate, which streamed sounds are read at; [`SILENT_RATE`] until told.
    rate: u32,
}

impl Default for Mixer {
    fn default() -> Self {
        Self::at_rate(SILENT_RATE)
    }
}

impl Mixer {
    /// A mixer for an output at `rate`.
    pub fn at_rate(rate: u32) -> Self {
        Self { voices: Vec::new(), next: 0, listener: Listener::default(), rate: rate.max(1) }
    }

    /// Starts a clip; returns its voice id, or `None` when [`MAX_VOICES`] are already playing.
    pub fn play(&mut self, clip: Arc<Clip>, source: &str, owner: Option<StableId>, volume: f32, looping: bool, place: Option<Place>) -> Option<u64> {
        self.start(Playing::Clip(clip), source, owner, volume, looping, place)
    }

    /// Starts a sound, loaded or read from its file; its voice id, `Ok(None)` when
    /// [`MAX_VOICES`] are already playing, or why a streamed file cannot start.
    pub fn play_sound(
        &mut self,
        sound: &Sound,
        source: &str,
        owner: Option<StableId>,
        volume: f32,
        looping: bool,
        place: Option<Place>,
    ) -> Result<Option<u64>, String> {
        if self.voices.len() >= MAX_VOICES {
            return Ok(None);
        }
        let playing = match sound {
            Sound::Loaded(clip) => Playing::Clip(Arc::clone(clip)),
            Sound::Streamed(file) => Playing::Stream(Box::new(stream::StreamVoice::start(file, self.rate, looping)?)),
        };
        Ok(self.start(playing, source, owner, volume, looping, place))
    }

    fn start(&mut self, playing: Playing, source: &str, owner: Option<StableId>, volume: f32, looping: bool, place: Option<Place>) -> Option<u64> {
        if self.voices.len() >= MAX_VOICES {
            return None;
        }
        self.next += 1;
        let mut voice =
            Voice { id: self.next, owner, source: source.to_owned(), playing, position: 0, looping, volume: volume.clamp(0.0, 4.0), place, gains: [0.0; 2] };
        voice.gains = gains(&voice, &self.listener);
        self.voices.push(voice);
        Some(self.next)
    }

    /// Stops the voices of `owner`, or one voice by id; returns how many stopped.
    pub fn stop(&mut self, owner: Option<StableId>, id: Option<u64>) -> usize {
        let before = self.voices.len();
        self.voices.retain(|v| !(owner.is_some() && v.owner == owner || id == Some(v.id)));
        before - self.voices.len()
    }

    pub fn stop_all(&mut self) {
        self.voices.clear();
    }

    /// Moves the listener and the voices of objects (`places` by owner), and drops the voices of
    /// owners `gone` says are gone.
    pub fn place(&mut self, listener: Listener, places: &HashMap<StableId, DVec3>, gone: impl Fn(StableId) -> bool) {
        self.listener = listener;
        self.voices.retain(|v| v.owner.is_none_or(|o| !gone(o)));
        for voice in &mut self.voices {
            if let (Some(owner), Some(place)) = (voice.owner, voice.place.as_mut())
                && let Some(at) = places.get(&owner)
            {
                place.at = *at;
            }
            voice.gains = gains(voice, &self.listener);
        }
    }

    pub fn voices(&self) -> &[Voice] {
        &self.voices
    }

    /// Adds the voices into `left` and `right` (one sample per frame each) and moves them on;
    /// voices that end are dropped, looping ones start over.
    pub fn mix(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        for voice in &mut self.voices {
            let clip = match &mut voice.playing {
                Playing::Clip(clip) => Arc::clone(clip),
                Playing::Stream(stream) => {
                    voice.position += stream.mix(&mut left[..frames], &mut right[..frames], voice.gains);
                    continue;
                }
            };
            let length = clip.frames();
            let [gl, gr] = voice.gains;
            let mut written = 0;
            while written < frames && length > 0 {
                if voice.position >= length {
                    if !voice.looping {
                        break;
                    }
                    voice.position = 0;
                }
                let take = (length - voice.position).min(frames - written);
                for i in 0..take {
                    left[written + i] += clip.left[voice.position + i] * gl;
                    right[written + i] += clip.right[voice.position + i] * gr;
                }
                voice.position += take;
                written += take;
            }
        }
        self.voices.retain(|v| match &v.playing {
            Playing::Clip(clip) => v.looping || v.position < clip.frames(),
            Playing::Stream(stream) => !stream.is_over(),
        });
    }
}

/// A voice's left and right gains: its volume; for a spatial voice also fading to silence at its
/// range and panned by where it is from the listener (equal power).
fn gains(voice: &Voice, listener: &Listener) -> [f32; 2] {
    let Some(place) = voice.place else { return [voice.volume; 2] };
    let offset = place.at - listener.at;
    let distance = offset.length();
    let fade = (1.0 - distance / place.range.max(1e-6)).clamp(0.0, 1.0);
    let pan = if distance > 1e-9 { offset.normalize().dot(listener.right.normalize_or(DVec3::X)).clamp(-1.0, 1.0) } else { 0.0 };
    let angle = (pan + 1.0) * std::f64::consts::FRAC_PI_4;
    let level = f64::from(voice.volume) * fade * fade * std::f64::consts::SQRT_2;
    [(level * angle.cos()) as f32, (level * angle.sin()) as f32]
}

/// The output thread: it owns the device's stream until told to stop.
#[cfg(not(target_arch = "wasm32"))]
struct Output {
    stop: Sender<()>,
    thread: Option<JoinHandle<()>>,
}

/// In a browser the page's audio runs the stream, with no thread of ours.
#[cfg(target_arch = "wasm32")]
struct Output {
    stream: cpal::Stream,
}

/// Sound for one play session: clips read once, the mixer, and the output.
pub struct Audio {
    mixer: Arc<Mutex<Mixer>>,
    rate: u32,
    clips: HashMap<PathBuf, Arc<Clip>>,
    streams: HashMap<PathBuf, Arc<Streamed>>,
    output: Option<Output>,
    /// Why there is no output, when there is none.
    pub silent_because: Option<String>,
}

fn lock(mixer: &Mutex<Mixer>) -> MutexGuard<'_, Mixer> {
    mixer.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Audio {
    /// Sound on the default output device; silent (the voices still play on in time) when there
    /// is none or it cannot start.
    pub fn open() -> Self {
        let mixer = Arc::new(Mutex::new(Mixer::default()));
        match start_output(Arc::clone(&mixer)) {
            Ok((output, rate)) => {
                lock(&mixer).rate = rate.max(1);
                Self { mixer, rate, clips: HashMap::new(), streams: HashMap::new(), output: Some(output), silent_because: None }
            }
            Err(why) => Self { mixer, rate: SILENT_RATE, clips: HashMap::new(), streams: HashMap::new(), output: None, silent_because: Some(why) },
        }
    }

    /// Sound that is never heard, at `rate`: for tests and servers.
    pub fn silent(rate: u32) -> Self {
        Self {
            mixer: Arc::new(Mutex::new(Mixer::at_rate(rate))),
            rate,
            clips: HashMap::new(),
            streams: HashMap::new(),
            output: None,
            silent_because: Some("silent by request".to_owned()),
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn is_heard(&self) -> bool {
        self.output.is_some()
    }

    /// The clip in the file at `path`, read and decoded the first time.
    pub fn clip(&mut self, path: &Path) -> Result<Arc<Clip>, String> {
        if let Some(clip) = self.clips.get(path) {
            return Ok(Arc::clone(clip));
        }
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
        if !EXTENSIONS.contains(&extension.as_str()) {
            return Err(format!("{} is not a sound file ({})", path.display(), EXTENSIONS.join(", ")));
        }
        let size = bayleaf_core::files::len(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if size > MAX_FILE_BYTES {
            return Err(format!("{} is larger than {} MiB", path.display(), MAX_FILE_BYTES >> 20));
        }
        let bytes = bayleaf_core::files::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let clip = Arc::new(Clip::decode(&bytes, &extension, self.rate).map_err(|e| format!("{}: {e}", path.display()))?);
        self.clips.insert(path.to_path_buf(), Arc::clone(&clip));
        Ok(clip)
    }

    /// The sound in the file at `path`: decoded whole the first time, or, with `stream`, checked
    /// once and read from the file each time it plays. A browser has no threads to read a file as it
    /// plays, so there every sound is loaded whole.
    pub fn sound(&mut self, path: &Path, stream: bool) -> Result<Sound, String> {
        if !stream || cfg!(target_arch = "wasm32") {
            return self.clip(path).map(Sound::Loaded);
        }
        if let Some(file) = self.streams.get(path) {
            return Ok(Sound::Streamed(Arc::clone(file)));
        }
        let file = Arc::new(can_stream(path)?);
        self.streams.insert(path.to_path_buf(), Arc::clone(&file));
        Ok(Sound::Streamed(file))
    }

    /// Works on the mixer.
    pub fn with<T>(&self, f: impl FnOnce(&mut Mixer) -> T) -> T {
        f(&mut lock(&self.mixer))
    }

    /// Without an output, moves the voices on by `seconds`, as the device would have played them.
    pub fn advance(&self, seconds: f64) {
        if self.output.is_some() {
            return;
        }
        let frames = (seconds.max(0.0) * f64::from(self.rate)).round() as usize;
        let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
        lock(&self.mixer).mix(&mut left, &mut right);
    }

    /// Starts the sound again where the system held it back: a browser lets a page play sound only
    /// after a click or a key, so the web player asks on each until it plays.
    pub fn resume(&self) {
        #[cfg(target_arch = "wasm32")]
        if let Some(output) = &self.output {
            use cpal::traits::StreamTrait;
            let _ = output.stream.play();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Audio {
    fn drop(&mut self) {
        if let Some(mut output) = self.output.take() {
            let _ = output.stop.send(());
            if let Some(thread) = output.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

/// Starts the default output device on a thread of its own; returns it and the device's rate.
#[cfg(not(target_arch = "wasm32"))]
fn start_output(mixer: Arc<Mutex<Mixer>>) -> Result<(Output, u32), String> {
    let (ready_send, ready) = channel::<Result<u32, String>>();
    let (stop, stopped) = channel::<()>();
    let thread = std::thread::Builder::new()
        .name("bayleaf audio".into())
        .spawn(move || match device_stream(mixer) {
            Ok((stream, rate)) => {
                let _ = ready_send.send(Ok(rate));
                let _ = stopped.recv();
                drop(stream);
            }
            Err(why) => {
                let _ = ready_send.send(Err(why));
            }
        })
        .map_err(|e| e.to_string())?;
    let rate = ready.recv_timeout(Duration::from_secs(5)).map_err(|_| "the sound device did not answer".to_owned())??;
    Ok((Output { stop, thread: Some(thread) }, rate))
}

/// Starts the page's sound; it stays quiet until the browser allows it ([`Audio::resume`]).
#[cfg(target_arch = "wasm32")]
fn start_output(mixer: Arc<Mutex<Mixer>>) -> Result<(Output, u32), String> {
    let (stream, rate) = device_stream(mixer)?;
    Ok((Output { stream }, rate))
}

fn device_stream(mixer: Arc<Mutex<Mixer>>) -> Result<(cpal::Stream, u32), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let device = cpal::default_host().default_output_device().ok_or("there is no sound output device")?;
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let rate = config.sample_rate().0;
    let channels = usize::from(config.channels()).max(1);
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config.into(), channels, mixer),
        cpal::SampleFormat::I16 => build::<i16>(&device, &config.into(), channels, mixer),
        cpal::SampleFormat::U16 => build::<u16>(&device, &config.into(), channels, mixer),
        other => return Err(format!("the sound device wants {other} samples, which are not supported")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, rate))
}

fn build<T>(device: &cpal::Device, config: &cpal::StreamConfig, channels: usize, mixer: Arc<Mutex<Mixer>>) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    use cpal::traits::DeviceTrait;
    let (mut left, mut right) = (Vec::new(), Vec::new());
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let frames = data.len() / channels;
                left.clear();
                left.resize(frames, 0.0);
                right.clear();
                right.resize(frames, 0.0);
                lock(&mixer).mix(&mut left, &mut right);
                for (frame, out) in data.chunks_mut(channels).enumerate() {
                    for (channel, sample) in out.iter_mut().enumerate() {
                        let value = match channel {
                            0 => left[frame],
                            1 => right[frame],
                            _ => 0.0,
                        };
                        *sample = T::from_sample(value.clamp(-1.0, 1.0));
                    }
                }
            },
            |_| {},
            None,
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
