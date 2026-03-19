use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

const AMPLITUDE: f32 = 0.10;
const FADE_SECS: f32 = 0.015; // 15ms fade in/out
const TONE_MS: u64 = 60; // each note duration
const GAP_MS: u64 = 20; // silence between notes

/// Ascending two-tone (C5→E5) — played BEFORE recording starts (blocking).
pub fn play_start_sound() {
    if let Err(e) = play_two_tone(523.0, 659.0) {
        log::warn!("Start sound failed: {}", e);
    }
}

/// Descending two-tone (E5→C5) — played AFTER recording stops (async).
pub fn play_stop_sound() {
    std::thread::spawn(|| {
        if let Err(e) = play_two_tone(659.0, 523.0) {
            log::warn!("Stop sound failed: {}", e);
        }
    });
}

fn play_two_tone(freq1: f32, freq2: f32) -> anyhow::Result<()> {
    play_tone(freq1, TONE_MS)?;
    std::thread::sleep(Duration::from_millis(GAP_MS));
    play_tone(freq2, TONE_MS)?;
    Ok(())
}

fn play_tone(freq: f32, duration_ms: u64) -> anyhow::Result<()> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("No audio output device"))?;
    let supported = device.default_output_config()?;
    let sample_rate = supported.sample_rate().0 as f32;
    let channels = supported.channels() as usize;
    let total_samples = (sample_rate * duration_ms as f32 / 1000.0) as u32;
    let idx = Arc::new(AtomicU32::new(0));

    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => {
            build_tone::<f32>(&device, &supported.into(), freq, total_samples, channels, sample_rate, idx)?
        }
        cpal::SampleFormat::I16 => {
            build_tone::<i16>(&device, &supported.into(), freq, total_samples, channels, sample_rate, idx)?
        }
        cpal::SampleFormat::I32 => {
            build_tone::<i32>(&device, &supported.into(), freq, total_samples, channels, sample_rate, idx)?
        }
        cpal::SampleFormat::U16 => {
            build_tone::<u16>(&device, &supported.into(), freq, total_samples, channels, sample_rate, idx)?
        }
        _ => anyhow::bail!("Unsupported output sample format"),
    };

    stream.play()?;
    std::thread::sleep(Duration::from_millis(duration_ms + 50));
    drop(stream);
    Ok(())
}

fn build_tone<T: SizedSample + FromSample<f32> + Send + 'static>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    freq: f32,
    total_samples: u32,
    channels: usize,
    sample_rate: f32,
    idx: Arc<AtomicU32>,
) -> anyhow::Result<cpal::Stream> {
    let fade_len = (sample_rate * FADE_SECS) as u32;

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            for frame in data.chunks_mut(channels) {
                let i = idx.fetch_add(1, Ordering::Relaxed);
                let value = if i < total_samples {
                    let t = i as f32 / sample_rate;
                    let raw = (t * freq * std::f32::consts::TAU).sin() * AMPLITUDE;
                    let fade = if i < fade_len {
                        i as f32 / fade_len as f32
                    } else if i > total_samples - fade_len {
                        (total_samples - i) as f32 / fade_len as f32
                    } else {
                        1.0
                    };
                    T::from_sample(raw * fade)
                } else {
                    T::from_sample(0.0f32)
                };
                for sample in frame.iter_mut() {
                    *sample = value;
                }
            }
        },
        |err| log::error!("Audio output error: {}", err),
        None,
    )?;
    Ok(stream)
}
