//! Unit tests for the `shift_text_range` helper that survives after Phase 2.
//!
//! After the refactor, the only surviving range helper is a pure integer-
//! arithmetic function:
//!
//! ```text
//! fn shift_text_range(range: TextRange, offset: TextSize) -> TextRange
//! ```
//!
//! All the old `shifted_range(text, range, usize)` / `shift_diagnostic_ranges`
//! / `remap_body_range` / `shifted_body_text_range` helpers are deleted;
//! call sites use `range + offset` directly (since `rowan::TextRange` implements
//! `Add<TextSize>`).
//!
//! This file tests the arithmetic contract directly.

use rowan::{TextRange, TextSize};

/// Identity: shifting by zero is a no-op.
#[test]
fn shift_by_zero_is_identity() {
    let range = TextRange::new(TextSize::from(10u32), TextSize::from(20u32));
    let shifted = range + TextSize::from(0u32);
    assert_eq!(shifted, range, "shifting by zero should be identity");
}

/// Non-zero offset: start and end both advance by the offset.
#[test]
fn shift_non_zero_offset_advances_both_endpoints() {
    let range = TextRange::new(TextSize::from(5u32), TextSize::from(15u32));
    let offset = TextSize::from(100u32);
    let shifted = range + offset;
    assert_eq!(
        u32::from(shifted.start()),
        105,
        "start should be 5 + 100 = 105"
    );
    assert_eq!(
        u32::from(shifted.end()),
        115,
        "end should be 15 + 100 = 115"
    );
}

/// Start at zero, non-zero offset.
#[test]
fn shift_from_zero_start() {
    let range = TextRange::new(TextSize::from(0u32), TextSize::from(7u32));
    let offset = TextSize::from(42u32);
    let shifted = range + offset;
    assert_eq!(u32::from(shifted.start()), 42);
    assert_eq!(u32::from(shifted.end()), 49);
}

/// Empty range (start == end): shift preserves the zero-width.
#[test]
fn shift_empty_range() {
    let range = TextRange::new(TextSize::from(3u32), TextSize::from(3u32));
    let shifted = range + TextSize::from(10u32);
    assert_eq!(
        shifted.start(),
        shifted.end(),
        "shifted empty range should still have start == end"
    );
    assert_eq!(u32::from(shifted.start()), 13);
}

/// The `len()` of a range is invariant under shifting.
#[test]
fn len_is_invariant_under_shift() {
    let range = TextRange::new(TextSize::from(4u32), TextSize::from(11u32));
    let original_len = range.len();
    let shifted = range + TextSize::from(999u32);
    assert_eq!(
        shifted.len(),
        original_len,
        "length should be invariant under shifting"
    );
}
