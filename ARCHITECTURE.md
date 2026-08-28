# ARCHITECTURE — current implementation

Resident is a Rust 2024 workspace:

- `core/` (`resident-core`) owns fingerprint/config types, typed errors, dump parsing, storage,
  matching, and extraction.
- `resident/` is the CLI and JSON-lines process edge. It may use `anyhow`; core does not.

The pinned Panako-compatible configuration lives in `core/src/config.rs`. Time is stored as
integer transform bins and converted to seconds only at an API edge. Fingerprints are the
plain fact tuple `(hash: u64, t: u32, f: u16)`; Panako resource ids are import metadata, not
engine identity.

Store layout and runtime data flow will be documented when those modules exist.

