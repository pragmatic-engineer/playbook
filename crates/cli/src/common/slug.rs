// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Slugs for file names and keys.

/// Every character outside `[A-Za-z0-9_.-]` becomes `_`, matching the retired
/// shell `slugify` and the old marker file names.
pub fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn slug_replaces_every_unsafe_character() {
        assert_eq!(slugify("a/b c"), "a_b_c");
        assert_eq!(slugify("keep_me.v-1"), "keep_me.v-1");
    }
}
