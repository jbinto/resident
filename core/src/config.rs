use sha2::{Digest, Sha256};

pub const SAMPLE_RATE: u32 = 16_000;
pub const TIME_RESOLUTION: u32 = 128;
pub const TIME_BIN_SECONDS: f64 = TIME_RESOLUTION as f64 / SAMPLE_RATE as f64;
pub const TRANSFORM_LATENCY_SAMPLES: u32 = 12_464;
pub const TRANSFORM_LATENCY_SECONDS: f64 = TRANSFORM_LATENCY_SAMPLES as f64 / SAMPLE_RATE as f64;
pub const MIN_FREQUENCY_HZ: f64 = 110.0;
pub const MAX_FREQUENCY_HZ: f64 = 7_040.0;
pub const REFERENCE_FREQUENCY_HZ: f64 = 440.0;
pub const BANDS_PER_OCTAVE: f64 = 85.0;
pub const QUERY_RANGE: u64 = 2;
pub const MIN_HITS_UNFILTERED: usize = 10;
pub const MIN_HITS_FILTERED_EXCLUSIVE: usize = 5;
pub const MIN_MATCH_DURATION_SECONDS: f64 = 5.0;
pub const MIN_SECONDS_WITH_MATCH: f64 = 0.2;
pub const MIN_TIME_FACTOR: f64 = 0.8;
pub const MAX_TIME_FACTOR: f64 = 1.2;
pub const MIN_FREQUENCY_FACTOR: f64 = 0.8;
pub const MAX_FREQUENCY_FACTOR: f64 = 1.2;
pub const HIT_PART_MAX_SIZE: usize = 250;
pub const HIT_PART_DIVIDER: usize = 5;
pub const MAX_HASH: u64 = (1_u64 << 34) - 1;
pub const MAX_FREQUENCY_BIN: u16 = 512;

const IDENTITY: &str = concat!(
    "panako-v0;sr=16000;step=128;latency=12464;freq=110:7040:440;bpo=85;",
    "filters=103:25;fp-time=2:33;fp-freq=1:128;query-range=2;",
    "hits=10:5;duration=5;coverage=.2;time-factor=.8:1.2;freq-factor=.8:1.2"
);

pub fn config_id() -> String {
    hex::encode(Sha256::digest(IDENTITY.as_bytes()))
}

pub fn bins_to_seconds(t: u32) -> f64 {
    f64::from(t) * TIME_BIN_SECONDS + TRANSFORM_LATENCY_SECONDS
}

pub fn seconds_to_bin(seconds: f64) -> Option<u32> {
    if !seconds.is_finite() || seconds < TRANSFORM_LATENCY_SECONDS {
        return None;
    }
    let bin = ((seconds - TRANSFORM_LATENCY_SECONDS) / TIME_BIN_SECONDS).ceil();
    (bin <= f64::from(u32::MAX)).then_some(bin as u32)
}

pub fn bin_to_hz(f: u16) -> f64 {
    MIN_FREQUENCY_HZ * 2_f64.powf(f64::from(f) / BANDS_PER_OCTAVE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_time_grid_has_measured_latency() {
        assert!((bins_to_seconds(12) - 0.875).abs() < 1e-12);
        assert_eq!(seconds_to_bin(0.875), Some(12));
    }

    #[test]
    fn reference_frequency_is_on_expected_bin() {
        assert!((bin_to_hz(170) - REFERENCE_FREQUENCY_HZ).abs() < 1e-9);
    }
}
