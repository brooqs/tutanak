//! Pure text-windowing for map-reduce LLM steps.
//!
//! A 2h transcript exceeds any single LLM context window, so translate/summarize
//! run map-reduce: split into word windows, process each (map), then combine
//! (reduce). `split_into_windows` is the pure splitter.

/// PURE: split `text` into windows of at most `max_words` words each.
/// Returns an empty vec for empty/whitespace-only input.
pub fn split_into_windows(text: &str, max_words: usize) -> Vec<String> {
    let max_words = max_words.max(1);
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    words
        .chunks(max_words)
        .map(|w| w.join(" "))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_no_windows() {
        assert!(split_into_windows("", 10).is_empty());
        assert!(split_into_windows("   \n  ", 10).is_empty());
    }

    #[test]
    fn fits_in_one_window() {
        assert_eq!(split_into_windows("a b c", 10), vec!["a b c"]);
    }

    #[test]
    fn splits_on_word_boundary() {
        let w = split_into_windows("one two three four five", 2);
        assert_eq!(w, vec!["one two", "three four", "five"]);
    }

    #[test]
    fn zero_max_is_clamped_to_one() {
        let w = split_into_windows("a b", 0);
        assert_eq!(w, vec!["a", "b"]);
    }
}
