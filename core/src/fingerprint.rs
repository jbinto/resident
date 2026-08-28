use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct Fingerprint {
    pub hash: u64,
    pub t: u32,
    pub f: u16,
}

impl Fingerprint {
    pub const fn new(hash: u64, t: u32, f: u16) -> Self {
        Self { hash, t, f }
    }
}
