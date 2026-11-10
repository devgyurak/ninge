// SPDX-License-Identifier: Apache-2.0
//! Tests of `diacritics`.

use super::*;

#[test]
fn given_table_when_reading_first_entries_then_they_are_0305_030d_030e() {
    let first = [diacritic(0), diacritic(1), diacritic(2)];
    assert_eq!(first, [Some('\u{305}'), Some('\u{30D}'), Some('\u{30E}')]);
}

#[test]
fn given_table_when_counting_entries_then_there_are_297() {
    assert_eq!(DIACRITICS.len(), 297);
}

#[test]
fn given_index_past_table_when_reading_diacritic_then_none() {
    assert_eq!(diacritic(297), None);
}

#[test]
fn given_table_when_checking_every_entry_then_all_are_distinct_valid_chars() {
    let mut sorted = DIACRITICS.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let valid = DIACRITICS
        .iter()
        .filter(|&&c| char::from_u32(c).is_some())
        .count();
    assert_eq!((sorted.len(), valid), (297, 297));
}
