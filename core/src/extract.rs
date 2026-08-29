use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

use rayon::prelude::*;
use rustfft::num_complex::Complex32;
use rustfft::num_traits::Zero;
use rustfft::{Fft, FftPlanner};
use serde::{Deserialize, Serialize};

use crate::config::{BANDS_PER_OCTAVE, MAX_FREQUENCY_BIN, MAX_FREQUENCY_HZ, SAMPLE_RATE};
use crate::{Error, Fingerprint, Result};

const HOP: usize = 128;
const AUDIO_BLOCK: usize = 8192;
const BAND_COUNT: usize = 510;
const MAX_FILTER_DATA_LENGTH: usize = AUDIO_BLOCK / 2;
const FREQUENCY_FILTER: usize = 103;
const TIME_FILTER: usize = 25;
const MIN_TIME_DISTANCE: u32 = 2;
const MAX_TIME_DISTANCE: u32 = 33;
const MIN_FREQUENCY_DISTANCE: u16 = 1;
const MAX_FREQUENCY_DISTANCE: u16 = 128;
const GABORATOR_MAX_ERROR: f64 = 1e-5;
const GABORATOR_ANALYSIS_SUPPORT: usize = 12_469;
// Twelve seconds at 16 kHz is deliberately one chunk so the fixture-proven transform remains
// unchanged. Longer inputs pay a fixed memory cost and retain this much context on each side.
const STREAM_CORE_SAMPLES: usize = 24 * AUDIO_BLOCK;
const STREAM_FFT_SIZE: usize = 1 << 18;

struct AnalysisSpectrum {
    bins: Vec<Complex32>,
    fft_size: usize,
    inverse: Arc<dyn Fft<f32>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Extraction {
    pub prints: Vec<Fingerprint>,
    pub duration: f64,
}

#[derive(Clone, Copy, Debug)]
struct EventPoint {
    t: u32,
    f: u16,
    magnitude: f32,
}

pub fn extract_audio(path: &Path) -> Result<Extraction> {
    let mut prints = Vec::new();
    let duration = extract_audio_streaming(path, |print| prints.push(print))?;
    Ok(Extraction { prints, duration })
}

/// Decode with bounded RAM and emit fingerprints in canonical extraction order.
pub fn extract_audio_streaming(path: &Path, mut emit: impl FnMut(Fingerprint)) -> Result<f64> {
    let mut child = ffmpeg_command(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            Error::BadRequest(format!("start ffmpeg for {}: {error}", path.display()))
        })?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Internal("ffmpeg stdout was not piped".into()))?;
    let mut spool =
        tempfile::tempfile().map_err(|error| Error::io("temporary PCM spool", error))?;
    let byte_count = std::io::copy(&mut stdout, &mut spool)
        .map_err(|error| Error::io("temporary PCM spool", error))?;
    let output = child.wait_with_output().map_err(|error| {
        Error::BadRequest(format!(
            "wait for ffmpeg decoding {}: {error}",
            path.display()
        ))
    })?;
    validate_ffmpeg_output(path, output.status.success(), &output.stderr, byte_count)?;
    extract_pcm(&mut spool, byte_count / 2, &mut emit)
}

/// Whole-file decode used only as an independent validation seam for the streaming path.
pub fn extract_audio_whole(path: &Path) -> Result<Extraction> {
    let output = ffmpeg_command(path).output().map_err(|error| {
        Error::BadRequest(format!("start ffmpeg for {}: {error}", path.display()))
    })?;
    validate_ffmpeg_output(
        path,
        output.status.success(),
        &output.stderr,
        output.stdout.len() as u64,
    )?;
    let samples: Vec<f32> = output
        .stdout
        .chunks_exact(2)
        .map(|chunk| {
            f32::from(i16::from_le_bytes(
                chunk.try_into().expect("two-byte chunk"),
            )) / 32_768.0
        })
        .collect();
    Ok(extract_samples(&samples))
}

fn ffmpeg_command(path: &Path) -> Command {
    let mut command = Command::new("ffmpeg");
    command
        .arg("-nostdin")
        .arg("-v")
        .arg("error")
        .arg("-i")
        .arg(path)
        .arg("-vn")
        .arg("-ar")
        .arg(SAMPLE_RATE.to_string())
        .arg("-ac")
        .arg("1")
        .arg("-acodec")
        .arg("pcm_s16le")
        .arg("-f")
        .arg("s16le")
        .arg("pipe:1");
    command
}

fn validate_ffmpeg_output(path: &Path, success: bool, stderr: &[u8], bytes: u64) -> Result<()> {
    if !success {
        return Err(Error::BadRequest(format!(
            "ffmpeg could not decode {}: {}",
            path.display(),
            String::from_utf8_lossy(stderr).trim()
        )));
    }
    if !bytes.is_multiple_of(2) {
        return Err(Error::Internal(
            "ffmpeg returned a partial s16 sample".into(),
        ));
    }
    Ok(())
}

fn extract_pcm(
    file: &mut File,
    sample_count: u64,
    emit: &mut impl FnMut(Fingerprint),
) -> Result<f64> {
    let sample_count = usize::try_from(sample_count)
        .map_err(|_| Error::Unsupported("decoded audio exceeds addressable sample count".into()))?;
    extract_chunks(
        sample_count,
        |start, stop| {
            let byte_start = u64::try_from(start)
                .ok()
                .and_then(|value| value.checked_mul(2))
                .ok_or_else(|| Error::Internal("PCM byte offset overflow".into()))?;
            file.seek(SeekFrom::Start(byte_start))
                .map_err(|error| Error::io("temporary PCM spool", error))?;
            let mut bytes = vec![0_u8; (stop - start) * 2];
            file.read_exact(&mut bytes)
                .map_err(|error| Error::io("temporary PCM spool", error))?;
            Ok(bytes
                .chunks_exact(2)
                .map(|chunk| {
                    f32::from(i16::from_le_bytes(
                        chunk.try_into().expect("two-byte chunk"),
                    )) / 32_768.0
                })
                .collect())
        },
        emit,
    )
}

pub fn extract_samples(samples: &[f32]) -> Extraction {
    let mut prints = Vec::new();
    let duration = extract_chunks(
        samples.len(),
        |start, stop| Ok(samples[start..stop].to_vec()),
        &mut |print| prints.push(print),
    )
    .expect("slice ranges are valid");
    Extraction { prints, duration }
}

fn extract_chunks(
    sample_count: usize,
    mut read_samples: impl FnMut(usize, usize) -> Result<Vec<f32>>,
    emit: &mut impl FnMut(Fingerprint),
) -> Result<f64> {
    let duration = sample_count as f64 / f64::from(SAMPLE_RATE);
    if sample_count < HOP * TIME_FILTER {
        return Ok(duration);
    }
    let padded_samples = sample_count.div_ceil(AUDIO_BLOCK) * AUDIO_BLOCK;
    let ring_frames = (GABORATOR_ANALYSIS_SUPPORT + 2 * AUDIO_BLOCK) / HOP;
    let available_frames = padded_samples.saturating_sub(GABORATOR_ANALYSIS_SUPPORT + 1) / HOP;
    let frames = available_frames.saturating_sub(ring_frames);
    if frames < TIME_FILTER {
        return Ok(duration);
    }
    let mut stream = EventStream::default();
    for core_start in (0..sample_count).step_by(STREAM_CORE_SAMPLES) {
        let core_stop = (core_start + STREAM_CORE_SAMPLES).min(sample_count);
        let input_start = core_start.saturating_sub(GABORATOR_ANALYSIS_SUPPORT);
        let input_stop = (core_stop + GABORATOR_ANALYSIS_SUPPORT).min(sample_count);
        let samples = read_samples(input_start, input_stop)?;
        if samples.len() != input_stop - input_start {
            return Err(Error::Internal(
                "sample source returned a short range".into(),
            ));
        }
        let spectrum = analysis_spectrum(&samples);
        let output_start = if core_start == 0 {
            0
        } else {
            core_start / HOP - 1
        };
        let output_stop = if core_stop == sample_count {
            frames
        } else {
            (core_stop / HOP - 1).min(frames)
        };
        if output_start >= output_stop {
            continue;
        }
        let by_band: Vec<Vec<f32>> = (0..BAND_COUNT)
            .into_par_iter()
            .map(|band| {
                band_magnitudes(
                    &spectrum,
                    samples.len(),
                    input_start,
                    output_start,
                    output_stop,
                    band,
                )
            })
            .collect();
        stream.push(&by_band, emit);
    }
    let event_count = stream.event_count;
    stream.finish(emit);
    if std::env::var_os("RESIDENT_EXTRACT_DEBUG").is_some() {
        eprintln!("extract frames={frames} events={event_count}");
    }
    Ok(duration)
}

fn analysis_spectrum(samples: &[f32]) -> AnalysisSpectrum {
    let fft_size = (samples.len() + 2 * crate::config::TRANSFORM_LATENCY_SAMPLES as usize)
        .next_power_of_two()
        .max(STREAM_FFT_SIZE);
    let mut bins = vec![Complex32::zero(); fft_size];
    for (output, &sample) in bins.iter_mut().zip(samples) {
        output.re = sample;
    }
    let mut planner = FftPlanner::new();
    let forward = planner.plan_fft_forward(fft_size);
    let inverse = planner.plan_fft_inverse(fft_size);
    forward.process(&mut bins);
    AnalysisSpectrum {
        bins,
        fft_size,
        inverse,
    }
}

fn band_magnitudes(
    spectrum: &AnalysisSpectrum,
    sample_count: usize,
    input_start: usize,
    output_start: usize,
    output_stop: usize,
    band: usize,
) -> Vec<f32> {
    // Gaborator numbers bands from high to low; JGaborator subtracts the first retained
    // band without reversing that order.
    let frequency = MAX_FREQUENCY_HZ * 2_f64.powf(-((band + 1) as f64) / BANDS_PER_OCTAVE);
    let frequency_sd = frequency * (2_f64.powf(1.0 / BANDS_PER_OCTAVE) - 1.0);
    let support_sd = (-2.0 * (GABORATOR_MAX_ERROR * 0.5).ln()).sqrt();
    let mut filtered = spectrum.bins.clone();
    for (index, value) in filtered.iter_mut().enumerate() {
        let bin_hz = if index <= spectrum.fft_size / 2 {
            index as f64 * f64::from(SAMPLE_RATE) / spectrum.fft_size as f64
        } else {
            -((spectrum.fft_size - index) as f64) * f64::from(SAMPLE_RATE)
                / spectrum.fft_size as f64
        };
        let distance_sd = (bin_hz - frequency) / frequency_sd;
        // Gaborator sizes the short spectrum from the requested support, then evaluates
        // Gaussian values across twice that support instead of hard-truncating at the budget.
        if distance_sd.abs() > 2.0 * support_sd {
            *value = Complex32::zero();
        } else {
            *value *= (-0.5 * distance_sd * distance_sd).exp() as f32;
        }
    }
    spectrum.inverse.process(&mut filtered);

    let two_sided_support = 2.0 * support_sd * frequency_sd / f64::from(SAMPLE_RATE);
    let mut native_step = 1;
    let mut scaled_support = two_sided_support;
    while scaled_support <= 0.5 {
        scaled_support *= 2.0;
        native_step *= 2;
    }
    let mut magnitudes = vec![0.0_f32; output_stop - output_start];
    let input_stop = input_start + sample_count;
    let first_sample = input_start
        .div_ceil(native_step)
        .max(1)
        .saturating_mul(native_step);
    for sample in (first_sample..input_stop).step_by(native_step) {
        let frame = sample / HOP;
        if frame > 0 {
            let output = frame - 1;
            if (output_start..output_stop).contains(&output) {
                let magnitude = filtered[sample - input_start].norm();
                let slot = &mut magnitudes[output - output_start];
                *slot = slot.max(magnitude);
            }
        }
    }
    magnitudes
}

#[derive(Default)]
struct EventStream {
    magnitudes: VecDeque<Vec<f32>>,
    vertical: VecDeque<Vec<f32>>,
    packer: EventPacker,
    frame_count: u32,
    event_count: usize,
}

impl EventStream {
    fn push(&mut self, by_band: &[Vec<f32>], emit: &mut impl FnMut(Fingerprint)) {
        let frames = by_band.first().map_or(0, Vec::len);
        debug_assert!(by_band.iter().all(|band| band.len() == frames));
        for time in 0..frames {
            let row: Vec<f32> = by_band.iter().map(|band| band[time]).collect();
            self.vertical.push_back(lemire_vertical_max_row(&row));
            self.magnitudes.push_back(row);
            let newest = self.frame_count;
            self.frame_count += 1;
            if self.magnitudes.len() < TIME_FILTER {
                continue;
            }
            let center = TIME_FILTER / 2;
            let event_time = newest - center as u32;
            for frequency in 2..BAND_COUNT - 1 {
                let value = self.magnitudes[center][frequency];
                if value == 0.0 || self.vertical[center][frequency] != value {
                    continue;
                }
                // Panako excludes the upper endpoint despite retaining a 25-frame cache.
                if self
                    .vertical
                    .iter()
                    .take(TIME_FILTER - 1)
                    .any(|row| row[frequency] > value)
                {
                    continue;
                }
                let mut magnitude = 0.0;
                for row in self.magnitudes.iter().skip(center - 1).take(3) {
                    magnitude += row[frequency - 1..=frequency + 1].iter().sum::<f32>();
                }
                self.packer.push(EventPoint {
                    t: event_time,
                    f: frequency as u16,
                    magnitude,
                });
                self.event_count += 1;
            }
            self.packer.advance(event_time, emit);
            self.vertical.pop_front();
            self.magnitudes.pop_front();
        }
    }

    fn finish(self, emit: &mut impl FnMut(Fingerprint)) {
        self.packer.finish(emit);
    }
}

#[derive(Default)]
struct EventPacker {
    events: VecDeque<EventPoint>,
}

impl EventPacker {
    fn push(&mut self, event: EventPoint) {
        self.events.push_back(event);
    }

    fn advance(&mut self, time: u32, emit: &mut impl FnMut(Fingerprint)) {
        while self
            .events
            .front()
            .is_some_and(|first| time > first.t + 2 * MAX_TIME_DISTANCE)
        {
            self.pack_first(emit);
        }
    }

    fn pack_first(&mut self, emit: &mut impl FnMut(Fingerprint)) {
        let first = self.events.pop_front().expect("nonempty event window");
        for (second_index, second) in self.events.iter().enumerate() {
            let dt = second.t - first.t;
            if dt > MAX_TIME_DISTANCE {
                break;
            }
            if dt < MIN_TIME_DISTANCE || !frequency_distance(first.f, second.f) {
                continue;
            }
            for third in self.events.iter().skip(second_index + 1) {
                let dt = third.t - second.t;
                if dt > MAX_TIME_DISTANCE {
                    break;
                }
                if dt < MIN_TIME_DISTANCE || !frequency_distance(second.f, third.f) {
                    continue;
                }
                emit(Fingerprint::new(
                    landmark_hash(first, *second, *third),
                    first.t,
                    first.f,
                ));
            }
        }
    }

    fn finish(mut self, emit: &mut impl FnMut(Fingerprint)) {
        while !self.events.is_empty() {
            self.pack_first(emit);
        }
    }
}

#[cfg(test)]
fn event_points(by_band: &[Vec<f32>], frames: usize) -> Vec<EventPoint> {
    let time_radius = TIME_FILTER / 2;
    let vertical: Vec<Vec<f32>> = (0..frames)
        .map(|time| lemire_vertical_max(by_band, time))
        .collect();
    let mut events = Vec::new();
    for t in time_radius..frames.saturating_sub(time_radius) {
        for f in 2..BAND_COUNT - 1 {
            let value = by_band[f][t];
            if value == 0.0 {
                continue;
            }
            if vertical[t][f] != value {
                continue;
            }
            let t_start = t - time_radius;
            // Panako's horizontal loop excludes the upper endpoint despite a 25-frame cache.
            let t_stop = t + time_radius;
            if (t_start..t_stop).any(|other| vertical[other][f] > value) {
                continue;
            }
            let mut magnitude = 0.0;
            for band in &by_band[f - 1..=f + 1] {
                magnitude += band[t - 1..=t + 1].iter().sum::<f32>();
            }
            events.push(EventPoint {
                t: t as u32,
                f: f as u16,
                magnitude,
            });
        }
    }
    events
}

#[cfg(test)]
fn lemire_vertical_max(by_band: &[Vec<f32>], time: usize) -> Vec<f32> {
    let row: Vec<f32> = by_band.iter().map(|band| band[time]).collect();
    lemire_vertical_max_row(&row)
}

fn lemire_vertical_max_row(row: &[f32]) -> Vec<f32> {
    debug_assert_eq!(row.len(), BAND_COUNT);
    let radius = FREQUENCY_FILTER / 2;
    // Panako constructs this filter for AUDIO_BLOCK/2 values, then passes a 510-value
    // Gaborator row. The untouched middle is zero and the right edge clamp lands far past
    // the values Panako reads. Preserve that observable behavior for fixture compatibility.
    let relevant_length = radius + BAND_COUNT + radius;
    debug_assert!(relevant_length < MAX_FILTER_DATA_LENGTH + FREQUENCY_FILTER - 1);
    let mut padded = vec![0.0_f32; relevant_length];
    padded[..radius].fill(row[0]);
    for (frequency, value) in padded[radius..radius + BAND_COUNT].iter_mut().enumerate() {
        *value = row[frequency];
    }

    let mut maximum = vec![0.0_f32; BAND_COUNT];
    let mut fifo = VecDeque::with_capacity(FREQUENCY_FILTER);
    fifo.push_back(0);
    for index in 1..FREQUENCY_FILTER {
        if padded[index] > padded[index - 1] {
            fifo.pop_back();
            while fifo
                .back()
                .is_some_and(|&previous| padded[index] > padded[previous])
            {
                fifo.pop_back();
            }
        }
        fifo.push_back(index);
    }
    for index in FREQUENCY_FILTER..relevant_length {
        maximum[index - FREQUENCY_FILTER] = padded[*fifo.front().expect("nonempty maximum deque")];
        if padded[index] > padded[index - 1] {
            fifo.pop_back();
            while fifo
                .back()
                .is_some_and(|&previous| padded[index] > padded[previous])
            {
                fifo.pop_back();
            }
        }
        fifo.push_back(index);
        if index == FREQUENCY_FILTER + *fifo.front().expect("nonempty maximum deque") {
            fifo.pop_front();
        }
    }
    maximum[BAND_COUNT - 1] = padded[*fifo.front().expect("nonempty maximum deque")];
    maximum
}

fn frequency_distance(a: u16, b: u16) -> bool {
    let distance = a.abs_diff(b);
    (MIN_FREQUENCY_DISTANCE..=MAX_FREQUENCY_DISTANCE).contains(&distance)
}

fn landmark_hash(first: EventPoint, second: EventPoint, third: EventPoint) -> u64 {
    debug_assert!(first.f <= MAX_FREQUENCY_BIN);
    let bit = |value: bool| u64::from(value);
    let ratio_t =
        u64::from((((second.t - first.t) as f32 / (third.t - first.t) as f32) * 64.0) as u32);
    let f1_range = u64::from(first.f >> 5);
    let df2f1 = u64::from(second.f.abs_diff(first.f) >> 2);
    let df3f2 = u64::from(third.f.abs_diff(second.f) >> 2);
    (ratio_t & 0x3f)
        | (bit(first.f > second.f) << 6)
        | (bit(second.f > third.f) << 7)
        | (bit(third.f > first.f) << 8)
        | (bit(first.magnitude > second.magnitude) << 9)
        | (bit(second.magnitude > third.magnitude) << 10)
        | (bit(third.magnitude > first.magnitude) << 11)
        | (bit(second.t - first.t > third.t - second.t) << 12)
        | (bit(second.f.abs_diff(first.f) > third.f.abs_diff(second.f)) << 13)
        | ((f1_range & 0xff) << 14)
        | ((df2f1 & 0x3f) << 22)
        | ((df3f2 & 0x3f) << 28)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_hash_packing_uses_all_34_bits() {
        let first = EventPoint {
            t: 1,
            f: 511,
            magnitude: 3.0,
        };
        let second = EventPoint {
            t: 3,
            f: 255,
            magnitude: 2.0,
        };
        let third = EventPoint {
            t: 6,
            f: 0,
            magnitude: 1.0,
        };
        let hash = landmark_hash(first, second, third);
        assert!(hash > u64::from(u32::MAX));
        assert!(hash < (1_u64 << 34));
        assert_eq!(hash & 0x3f, 25);
    }

    #[test]
    fn short_audio_has_duration_but_no_prints() {
        let extraction = extract_samples(&vec![0.0; HOP * 2]);
        assert!(extraction.prints.is_empty());
        assert_eq!(extraction.duration, 0.016);
    }

    #[test]
    fn horizontal_filter_consumes_neighboring_vertical_maxima() {
        let mut magnitudes = vec![vec![0.0; 30]; BAND_COUNT];
        magnitudes[100][12] = 5.0;
        magnitudes[101][11] = 10.0;
        let events = event_points(&magnitudes, 30);
        assert!(!events.iter().any(|event| event.t == 12 && event.f == 100));
    }
}
