//! Deterministic hashing of a [`Canon`](crate::canonical::Canon) value.
//!
//! The fingerprint is a SHA-256 over a canonical textual encoding of the normal
//! form. The encoding uses explicit field tags and length prefixes so that two
//! structurally different forms can never serialise to the same bytes (no
//! delimiter-collision ambiguity). Sorted containers (`BTreeMap`/`BTreeSet`)
//! make the encoding order-independent where the form is order-independent.

use sha2::{Digest, Sha256};

/// Builds the canonical byte encoding incrementally. Every piece is written with
/// an unambiguous, length-prefixed framing.
#[derive(Default)]
pub(crate) struct Encoder {
    buf: Vec<u8>,
}

impl Encoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Write a tagged, length-prefixed string field.
    pub fn field(&mut self, tag: &str, value: &str) {
        self.raw(tag);
        self.raw_len(value.as_bytes());
    }

    /// Write a tag with no value (e.g. a boolean flag's presence).
    pub fn tag(&mut self, tag: &str) {
        self.raw(tag);
    }

    fn raw(&mut self, s: &str) {
        self.buf.extend_from_slice(b"|");
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.extend_from_slice(b":");
    }

    fn raw_len(&mut self, bytes: &[u8]) {
        self.buf
            .extend_from_slice(format!("{}=", bytes.len()).as_bytes());
        self.buf.extend_from_slice(bytes);
    }

    pub fn finish(self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(&self.buf);
        hasher.finalize().into()
    }
}
