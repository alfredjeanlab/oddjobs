// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Slugify strings for use as job name components.

const STOP_WORDS: &[&str] = &[
    "the",
    "a",
    "an",
    "is",
    "are",
    "was",
    "were",
    "be",
    "been",
    "being",
    "have",
    "has",
    "had",
    "do",
    "does",
    "did",
    "will",
    "would",
    "shall",
    "should",
    "may",
    "might",
    "must",
    "can",
    "could",
    "to",
    "of",
    "in",
    "for",
    "on",
    "with",
    "at",
    "by",
    "from",
    "as",
    "into",
    "through",
    "during",
    "before",
    "after",
    "above",
    "below",
    "between",
    "out",
    "off",
    "over",
    "under",
    "again",
    "further",
    "then",
    "once",
    "that",
    "this",
    "these",
    "those",
    "and",
    "but",
    "or",
    "nor",
    "not",
    "so",
    "yet",
    "both",
    "each",
    "every",
    "all",
    "any",
    "few",
    "more",
    "most",
    "other",
    "some",
    "such",
    "no",
    "only",
    "own",
    "same",
    "than",
    "too",
    "very",
    "just",
    "about",
    "also",
    "its",
    "it",
    "we",
    "our",
    "currently",
    "when",
    "which",
    "what",
    // Pronouns commonly found in contractions
    "i",
    "he",
    "she",
    "they",
    "you",
    // Contraction fragments (apostrophe replaced by hyphen, e.g. "don't" → "don-t")
    "t",
    "s",
    "d",
    "m",
    "re",
    "ve",
    "ll",
    // Left stems of common n't contractions
    "don",
    "doesn",
    "didn",
    "hasn",
    "hadn",
    "isn",
    "aren",
    "wasn",
    "weren",
    "won",
    "wouldn",
    "shouldn",
    "couldn",
    "mustn",
    "needn",
];

/// Slugify a string for use as a job name component.
///
/// Lowercases, replaces non-alphanumeric with hyphens, removes stop words,
/// collapses hyphens, and truncates to `max_len` characters (trimming trailing hyphens).
pub fn slugify(input: &str, max_len: usize) -> String {
    // 1. Lowercase
    let lower = input.to_lowercase();

    // 2. Replace any run of non-[a-z0-9] characters with a single hyphen
    let mut slug = String::with_capacity(lower.len());
    let mut last_was_hyphen = false;
    for ch in lower.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_was_hyphen = false;
        } else if !last_was_hyphen {
            slug.push('-');
            last_was_hyphen = true;
        }
    }

    // 3. Split on hyphens, filter out stop words, deduplicate consecutive repeats, rejoin
    let mut filtered: Vec<&str> = Vec::new();
    for word in slug.split('-') {
        if word.is_empty() || STOP_WORDS.contains(&word) {
            continue;
        }
        if filtered.last() != Some(&word) {
            filtered.push(word);
        }
    }
    let mut result = filtered.join("-");

    // 4. Trim leading/trailing hyphens
    let trimmed = result.trim_matches('-');
    if trimmed.len() != result.len() {
        result = trimmed.to_string();
    }

    // 5. Truncate to max_len at word boundary
    if result.len() > max_len {
        if let Some(pos) = result[..max_len].rfind('-') {
            result.truncate(pos);
        } else {
            result.truncate(max_len);
        }
    }

    // 6. Trim trailing hyphens (safety net)
    let trimmed = result.trim_end_matches('-');
    if trimmed.len() != result.len() {
        result = trimmed.to_string();
    }

    result
}

/// Build a job name from a template result and nonce.
///
/// Slugifies the input, truncates to 28 chars, strips the project prefix
/// (if present), and appends `-{nonce}`.
pub fn job_display_name(raw: &str, nonce: &str, project: &str) -> String {
    let slug = slugify(raw, 28);
    let slug = if !project.is_empty() {
        let prefix = format!("{}-", project);
        slug.strip_prefix(&prefix).unwrap_or(&slug)
    } else {
        &slug
    };
    if slug.is_empty() {
        nonce.to_string()
    } else {
        format!("{}-{}", slug, nonce)
    }
}

#[cfg(test)]
#[path = "slug_tests.rs"]
mod tests;
