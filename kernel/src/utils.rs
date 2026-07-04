/// Safely truncate a string to at most `max_chars` characters,
/// respecting UTF-8 character boundaries. Returns a `&str` slice
/// that is never in the middle of a multi-byte character.
pub fn safe_truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => &s[..byte_idx],
        None => s,
    }
}
