//! Pure transcript stitching.
//!
//! Chunks overlap in audio, so consecutive transcripts share words at the
//! boundary. `stitch` joins them while removing the duplicated overlap region.
//!
//!   chunk A: "... we should ship the release today"
//!   chunk B: "ship the release today and then review"
//!                 └── shared tail/prefix dropped ──┘
//!   result : "... we should ship the release today and then review"

/// Maximum number of trailing/leading words we search for an overlap.
const MAX_OVERLAP_WORDS: usize = 40;

/// PURE: join transcripts, de-duplicating the overlapping words between each
/// consecutive pair. Empty transcripts are skipped.
pub fn stitch(transcripts: &[String]) -> String {
    let mut result: Vec<String> = Vec::new();
    for t in transcripts {
        let words: Vec<&str> = t.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        if result.is_empty() {
            result.extend(words.iter().map(|w| w.to_string()));
            continue;
        }
        let overlap = longest_overlap(&result, &words);
        result.extend(words[overlap..].iter().map(|w| w.to_string()));
    }
    result.join(" ")
}

/// Length (in words) of the longest suffix of `prev` that equals a prefix of
/// `next`, capped at `MAX_OVERLAP_WORDS`. Case-insensitive, punctuation-trimmed.
fn longest_overlap(prev: &[String], next: &[&str]) -> usize {
    let max = MAX_OVERLAP_WORDS.min(prev.len()).min(next.len());
    for len in (1..=max).rev() {
        let prev_tail = &prev[prev.len() - len..];
        let next_head = &next[..len];
        if prev_tail
            .iter()
            .map(|w| norm(w))
            .eq(next_head.iter().map(|w| norm(w)))
        {
            return len;
        }
    }
    0
}

fn norm(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> String {
        x.to_string()
    }

    #[test]
    fn single_transcript_passthrough() {
        assert_eq!(stitch(&[s("hello world")]), "hello world");
    }

    #[test]
    fn empty_inputs_dropped() {
        assert_eq!(stitch(&[s(""), s("a b"), s("")]), "a b");
        assert_eq!(stitch(&[]), "");
    }

    #[test]
    fn overlap_is_deduped() {
        let a = s("we should ship the release today");
        let b = s("ship the release today and then review");
        assert_eq!(
            stitch(&[a, b]),
            "we should ship the release today and then review"
        );
    }

    #[test]
    fn overlap_is_case_and_punctuation_insensitive() {
        let a = s("the meeting is over.");
        let b = s("Over, we agreed on the plan");
        // "over" matches across "over." / "Over,"
        assert_eq!(stitch(&[a, b]), "the meeting is over. we agreed on the plan");
    }

    #[test]
    fn no_overlap_just_concatenates() {
        assert_eq!(stitch(&[s("alpha beta"), s("gamma delta")]), "alpha beta gamma delta");
    }

    #[test]
    fn three_chunks_each_overlap() {
        let a = s("one two three four");
        let b = s("three four five six");
        let c = s("five six seven eight");
        assert_eq!(stitch(&[a, b, c]), "one two three four five six seven eight");
    }
}
