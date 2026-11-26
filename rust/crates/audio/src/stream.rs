//! Sounds played straight from their file: a thread decodes them a block at a time (craft-audio's
//! decoder, which reads WAV, AIFF, MP3, Opus and AAC that way), turns them into stereo at the
//! output's rate (craft-audio-dsp's streaming resampler, with no jump between blocks), and keeps a
//! few blocks ready for the mixer. Long music then takes neither the time to decode it all nor the
//! memory to hold it. Ogg Vorbis and FLAC have no such decoder here and are loaded whole.

#[cfg(not(target_arch = "wasm32"))]
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{SyncSender, sync_channel};

#[cfg(not(target_arch = "wasm32"))]
use craft_audio_dsp::resample::StreamResampler;

/// Frames in each block the thread hands the mixer.
#[cfg(not(target_arch = "wasm32"))]
const BLOCK_FRAMES: usize = 4096;
/// Blocks kept ready: about two thirds of a second at 48 kHz.
#[cfg(not(target_arch = "wasm32"))]
const READY_BLOCKS: usize = 8;

/// A sound file to play as it is read.
#[derive(Debug)]
pub struct Streamed {
    pub path: PathBuf,
    extension: String,
}

/// Whether the file at `path` can be played as it is read; why not when it cannot.
pub fn can_stream(path: &Path) -> Result<Streamed, String> {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    open(path, &extension).map(|_| Streamed { path: path.to_path_buf(), extension }).map_err(|e| {
        format!("{} cannot be played as it is read ({e}); streaming reads WAV, AIFF, MP3, Opus and AAC, and other sounds are loaded whole", path.display())
    })
}

/// The file on the disk is read as it plays; one in an exported game's pack is decoded from memory.
fn open(path: &Path, extension: &str) -> Result<craft_audio::Decoder<'static>, String> {
    let options = craft_audio::DecodeOptions::default();
    if path.is_file() {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let source = craft_audio::ReadSeekSource::new(file).map_err(|e| e.to_string())?;
        return craft_audio::Decoder::open(source, Some(extension), &options).map_err(|e| e.to_string());
    }
    let bytes = bayleaf_core::files::read(path).map_err(|e| e.to_string())?;
    let source = craft_audio::ReadSeekSource::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    craft_audio::Decoder::open(source, Some(extension), &options).map_err(|e| e.to_string())
}

/// A stereo block for the mixer.
type Block = (Vec<f32>, Vec<f32>);

/// A streamed sound playing: blocks arrive from its thread, which stops when this is dropped.
#[derive(Debug)]
pub(crate) struct StreamVoice {
    blocks: Receiver<Block>,
    current: Block,
    at: usize,
    /// The thread has sent its last block.
    ended: bool,
    stop: Arc<AtomicBool>,
}

impl StreamVoice {
    /// Starts reading `sound` on a thread of its own, for output at `rate`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn start(sound: &Streamed, rate: u32, looping: bool) -> Result<Self, String> {
        let decoder = open(&sound.path, &sound.extension).map_err(|e| format!("{}: {e}", sound.path.display()))?;
        let (send, blocks) = sync_channel(READY_BLOCKS);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("bayleaf sound stream".into())
            .spawn(move || feed(decoder, rate, looping, &send, &stopped))
            .map_err(|e| format!("cannot start reading {}: {e}", sound.path.display()))?;
        Ok(Self { blocks, current: (Vec::new(), Vec::new()), at: 0, ended: false, stop })
    }

    /// A browser has no threads to read a sound on; [`crate::Audio::sound`] loads it whole there.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn start(sound: &Streamed, _rate: u32, _looping: bool) -> Result<Self, String> {
        Err(format!("{} cannot be read as it plays in a browser; a browser loads {} sounds whole", sound.path.display(), sound.extension))
    }

    /// Adds up to `left.len()` frames into `left` and `right`, times the gains; returns how many
    /// it had. Fewer than asked means the thread is behind (a gap of silence) or the sound ended.
    pub(crate) fn mix(&mut self, left: &mut [f32], right: &mut [f32], gains: [f32; 2]) -> usize {
        let frames = left.len().min(right.len());
        let mut written = 0;
        while written < frames {
            if self.at >= self.current.0.len() {
                match self.blocks.try_recv() {
                    Ok(block) => {
                        self.current = block;
                        self.at = 0;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.ended = true;
                        break;
                    }
                }
                continue;
            }
            let take = (self.current.0.len() - self.at).min(frames - written);
            for i in 0..take {
                left[written + i] += self.current.0[self.at + i] * gains[0];
                right[written + i] += self.current.1[self.at + i] * gains[1];
            }
            self.at += take;
            written += take;
        }
        written
    }

    /// Whether it has played everything its thread sent and the thread is done.
    pub(crate) fn is_over(&self) -> bool {
        self.ended && self.at >= self.current.0.len()
    }
}

impl Drop for StreamVoice {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// The thread's work: decode, resample and send blocks until the sound ends (or, looping, until
/// stopped), the voice is dropped, or the file cannot be read further.
#[cfg(not(target_arch = "wasm32"))]
fn feed(mut decoder: craft_audio::Decoder<'static>, rate: u32, looping: bool, send: &SyncSender<Block>, stop: &AtomicBool) {
    let from = decoder.info().sample_rate.max(1);
    let mut resampler = StreamResampler::new(from, rate);
    let (mut left, mut right) = (VecDeque::<f32>::new(), VecDeque::<f32>::new());
    // Source frames decoded in all, and whether the file is done.
    let (mut decoded, mut done) = (0u64, false);
    let mut out = 0i64;
    while !stop.load(Ordering::Relaxed) {
        let pull = |_start: i64, frames: usize| {
            while left.len() < frames && !done {
                match decoder.next_block() {
                    Ok(Some(block)) => {
                        let mut channels = block.channels.into_iter();
                        let first = channels.next().unwrap_or_default();
                        let second = channels.next().unwrap_or_else(|| first.clone());
                        decoded += first.len() as u64;
                        left.extend(first);
                        right.extend(second);
                    }
                    // Looping starts the file over; a file that cannot go back ends.
                    Ok(None) if looping && decoded > 0 => done = decoder.seek(0).is_err(),
                    Ok(None) | Err(_) => done = true,
                }
            }
            let take = frames.min(left.len());
            let mut l: Vec<f32> = left.drain(..take).collect();
            let mut r: Vec<f32> = right.drain(..take).collect();
            l.resize(frames, 0.0);
            r.resize(frames, 0.0);
            vec![l, r]
        };
        let mut block = resampler.process(out, BLOCK_FRAMES, pull);
        let (Some(r), Some(l)) = (block.pop(), block.pop()) else { return };
        let mut block = (l, r);
        out += BLOCK_FRAMES as i64;
        if done && left.is_empty() {
            // The last block: only as long as the sound, at the output's rate.
            let length = i64::try_from(decoded.saturating_mul(u64::from(rate)).div_ceil(u64::from(from))).unwrap_or(i64::MAX);
            let keep = usize::try_from((length - (out - BLOCK_FRAMES as i64)).clamp(0, BLOCK_FRAMES as i64)).unwrap_or(0);
            block.0.truncate(keep);
            block.1.truncate(keep);
            let _ = send.send(block);
            return;
        }
        if send.send(block).is_err() {
            return;
        }
    }
}
