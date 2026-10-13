//! Borrowed, typed keys for grouping, joining and sorting.
//!
//! A [`KeyPart`] reads one cell of a column without boxing it as a `Scalar`,
//! and string keys borrow from the batch. Equality and hashing match the
//! grouping rules (`NaN` groups with `NaN`, `-0.0` with `0.0`), and ordering
//! matches the sort rules (NULL first, `NaN` last), so one key type serves all
//! three operators.

use std::cmp::Ordering;

use crate::array::ArrayRef;

/// One cell of a column, borrowed where possible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyPart<'a> {
    Null,
    Bool(bool),
    I32(i32),
    I64(i64),
    /// `f64` bits after normalisation: all NaNs share one pattern and `-0.0`
    /// becomes `0.0`, so equal keys are bit-equal.
    F64(u64),
    Str(&'a str),
}

impl KeyPart<'_> {
    /// Position in the sort order of variants: NULL sorts before every value.
    fn rank(&self) -> u8 {
        match self {
            KeyPart::Null => 0,
            KeyPart::Bool(_) => 1,
            KeyPart::I32(_) => 2,
            KeyPart::I64(_) => 3,
            KeyPart::F64(_) => 4,
            KeyPart::Str(_) => 5,
        }
    }
}

impl Ord for KeyPart<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (KeyPart::Null, KeyPart::Null) => Ordering::Equal,
            (KeyPart::Bool(a), KeyPart::Bool(b)) => a.cmp(b),
            (KeyPart::I32(a), KeyPart::I32(b)) => a.cmp(b),
            (KeyPart::I64(a), KeyPart::I64(b)) => a.cmp(b),
            (KeyPart::F64(a), KeyPart::F64(b)) => f64::from_bits(*a).total_cmp(&f64::from_bits(*b)),
            (KeyPart::Str(a), KeyPart::Str(b)) => a.cmp(b),
            // Different variants only occur when comparing across columns of
            // different types, which the planner rules out. Keep the order total.
            _ => self.rank().cmp(&other.rank()),
        }
    }
}

impl PartialOrd for KeyPart<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Normalise `f64` bits so that keys which should group together compare equal.
pub fn f64_key_bits(v: f64) -> u64 {
    if v.is_nan() {
        f64::NAN.to_bits()
    } else if v == 0.0 {
        0
    } else {
        v.to_bits()
    }
}

/// Read the key for row `row` of `col`.
pub fn key_at(col: &ArrayRef, row: usize) -> KeyPart<'_> {
    match col {
        ArrayRef::Boolean(a) => match a.0.get(row).copied().flatten() {
            Some(v) => KeyPart::Bool(v),
            None => KeyPart::Null,
        },
        ArrayRef::Int32(a) => match a.0.get(row).copied().flatten() {
            Some(v) => KeyPart::I32(v),
            None => KeyPart::Null,
        },
        ArrayRef::Int64(a) => match a.0.get(row).copied().flatten() {
            Some(v) => KeyPart::I64(v),
            None => KeyPart::Null,
        },
        ArrayRef::Float64(a) => match a.0.get(row).copied().flatten() {
            Some(v) => KeyPart::F64(f64_key_bits(v)),
            None => KeyPart::Null,
        },
        ArrayRef::Date(a) => match a.0.get(row).copied().flatten() {
            Some(v) => KeyPart::I32(v),
            None => KeyPart::Null,
        },
        ArrayRef::Timestamp(a) => match a.0.get(row).copied().flatten() {
            Some(v) => KeyPart::I64(v),
            None => KeyPart::Null,
        },
        ArrayRef::Utf8(a) => match a.0.get(row).and_then(|v| v.as_deref()) {
            Some(s) => KeyPart::Str(s),
            None => KeyPart::Null,
        },
    }
}

/// A fast, non-cryptographic hasher for the engine's internal indexes.
///
/// The standard library's default hasher resists hash-flooding, which costs
/// time on every short key. Keys here come from the engine's own input, not
/// from an attacker, so this simple multiply-and-rotate hash is enough.
#[derive(Default, Clone, Copy)]
pub struct FastHasher {
    hash: u64,
}

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FastHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl std::hash::Hasher for FastHasher {
    fn finish(&self) -> u64 {
        self.hash
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            let mut word = [0u8; 8];
            word.copy_from_slice(chunk);
            self.add(u64::from_le_bytes(word));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut word = [0u8; 8];
            word[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(word));
        }
    }

    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }

    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    fn write_i64(&mut self, i: i64) {
        self.add(i as u64);
    }

    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
}

/// A `HashMap` that uses [`FastHasher`].
pub type FastMap<K, V> = std::collections::HashMap<K, V, std::hash::BuildHasherDefault<FastHasher>>;
