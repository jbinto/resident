#![deny(unsafe_code)]

pub mod config;
pub mod dump;
pub mod error;
pub mod extract;
pub mod fingerprint;
pub mod matcher;
mod mmap_view;
pub mod span;
pub mod store;

pub use dump::{DumpResource, ResourceMeta, load_dump_dir, load_metadata, load_prints};
pub use error::{Error, Result};
pub use extract::{
    Extraction, extract_audio, extract_audio_streaming, extract_audio_whole, extract_samples,
};
pub use fingerprint::Fingerprint;
pub use matcher::{Evidence, MatchRow, Matcher};
pub use span::{CrosscheckMatch, Segment, crosscheck, crosscheck_between, span, span_between};
pub use store::{IngestStats, ResourceInfo, RetireStats, Store, StoreStats};
