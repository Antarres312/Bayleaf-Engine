use std::collections::HashMap;
use std::sync::Arc;

use bayleaf_core::StableId;
use bayleaf_core::math::DVec3;

use super::{Audio, Clip, Listener, MAX_VOICES, Mixer, Place};

/// A mono 16-bit WAV file of `seconds` at `rate`, every sample at half of full scale.
fn wav(rate: u32, seconds: f64) -> Vec<u8> {
    let frames = (f64::from(rate) * seconds) as u32;
    let data = frames * 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for _ in 0..frames {
        bytes.extend_from_slice(&16_384i16.to_le_bytes());
    }
    bytes
}

fn clip(rate: u32, seconds: f64) -> Arc<Clip> {
    Arc::new(Clip::decode(&wav(8_000, seconds), "wav", rate).unwrap())
}

fn mixed(mixer: &mut Mixer, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    mixer.mix(&mut left, &mut right);
    (left, right)
}

#[test]
fn a_clip_decodes_to_stereo_at_the_mixers_rate() {
    let at_twice = clip(16_000, 0.5);
    let frames = at_twice.frames() as f64;
    assert!((frames - 8_000.0).abs() < 50.0, "half a second at 16 kHz: {frames} frames");
    assert!(Clip::decode(b"not a sound", "wav", 16_000).is_err(), "a broken file is an error, not a panic");
}

#[test]
fn voices_play_to_their_end_or_loop_and_stop_on_request() {
    let mut mixer = Mixer::default();
    let short = clip(8_000, 0.1);
    let once = mixer.play(Arc::clone(&short), "once.wav", None, 1.0, false, None).unwrap();
    let (left, right) = mixed(&mut mixer, 400);
    assert!(left[100] > 0.4 && right[100] > 0.4, "a centred voice is heard on both sides");
    mixed(&mut mixer, 1_000);
    assert!(mixer.voices().iter().all(|v| v.id != once), "it ended after its 800 frames");

    let owner = StableId::from_u128(7);
    mixer.play(Arc::clone(&short), "loop.wav", Some(owner), 1.0, true, None).unwrap();
    let (left, _) = mixed(&mut mixer, 2_000);
    assert!(left[1_900] > 0.4, "a looping voice starts over");
    assert_eq!(mixer.stop(Some(owner), None), 1, "stopping its object stops it");
    assert!(mixer.voices().is_empty());

    for _ in 0..MAX_VOICES {
        mixer.play(Arc::clone(&short), "many.wav", None, 0.1, true, None).unwrap();
    }
    assert!(mixer.play(short, "too many.wav", None, 0.1, true, None).is_none(), "at most {MAX_VOICES} voices");
}

#[test]
fn spatial_voices_pan_to_their_side_and_fade_out_at_their_range() {
    let mut mixer = Mixer::default();
    let tone = clip(8_000, 1.0);
    let owner = StableId::from_u128(7);
    mixer.play(tone, "near.wav", Some(owner), 1.0, true, Some(Place { at: DVec3::new(1.0, 0.0, 0.0), range: 10.0 })).unwrap();
    let (left, right) = mixed(&mut mixer, 100);
    assert!(right[50] > left[50] * 2.0, "a sound to the right is louder on the right: {} and {}", left[50], right[50]);
    // The object walks away, past its range.
    let places: HashMap<StableId, DVec3> = [(owner, DVec3::new(0.0, 0.0, 20.0))].into();
    mixer.place(Listener::default(), &places, |_| false);
    let (left, right) = mixed(&mut mixer, 100);
    assert!(left[50].abs() < 1e-6 && right[50].abs() < 1e-6, "beyond its range it is silent");
    // And then it is gone.
    mixer.place(Listener::default(), &places, |id| id == owner);
    assert!(mixer.voices().is_empty(), "a gone object's voice stops");
}

#[test]
fn silent_audio_reads_clips_once_and_keeps_time() {
    let dir = std::env::temp_dir().join(format!("bayleaf-audio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("beep.wav");
    std::fs::write(&file, wav(8_000, 0.25)).unwrap();
    let mut audio = Audio::silent(8_000);
    let beep = audio.clip(&file).unwrap();
    assert!(Arc::ptr_eq(&beep, &audio.clip(&file).unwrap()), "read once");
    assert!(audio.clip(&dir.join("notes.txt")).is_err(), "only sound files");
    audio.with(|mixer| mixer.play(beep, "beep.wav", None, 1.0, false, None));
    audio.advance(0.1);
    assert_eq!(audio.with(|m| m.voices().len()), 1, "still playing after 0.1 s");
    audio.advance(0.2);
    assert_eq!(audio.with(|m| m.voices().len()), 0, "done after its quarter second");
    std::fs::remove_dir_all(&dir).ok();
}
