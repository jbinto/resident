#![deny(unsafe_code)]

pub mod config;
pub mod dump;
pub mod error;
pub mod extract;
pub mod fingerprint;
pub mod matcher;
mod mmap_view;
pub mod passage;
pub mod span;
pub mod store;

pub use dump::{DumpResource, ResourceMeta, load_dump_dir, load_metadata, load_prints};
pub use error::{Error, Result};
pub use extract::{
    Extraction, extract_audio, extract_audio_streaming, extract_audio_whole, extract_samples,
};
pub use fingerprint::Fingerprint;
pub use matcher::{DensityBin, Evidence, EvidenceHit, HistogramBin, MatchRow, Matcher};
pub use passage::{
    PASSAGE_PROFILE, PairPassages, Passage, PassageDiscovery, PassageMatch, PassageQuality,
    PassageSnapshot, SupportSpan, discover_passages_between, passages_between,
};
pub use span::{
    CrosscheckMatch, Segment, crosscheck, crosscheck_between, crosscheck_between_multiline,
    crosscheck_multiline, span, span_between, span_between_multiline, span_multiline,
};
pub use store::{IngestStats, ResourceInfo, RetireStats, Store, StoreStats};
