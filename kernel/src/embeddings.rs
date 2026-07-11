//! Local, dependency-free text embeddings for semantic memory.
//!
//! Kerna's memory recall needs to rank past memories by relevance to the
//! current goal. Neural embeddings are the gold standard, but they require
//! either a network call (breaks the local-only privacy guarantee) or bundling
//! a large model (breaks the lightweight-binary goal). So the built-in default
//! is a **feature-hashing vectorizer**: a well-established technique that maps
//! token unigrams + bigrams into a fixed-dimension, L2-normalized vector using
//! a hash function as an implicit vocabulary.
//!
//! This is real lexical-semantic similarity — robust to word order, partial
//! overlap, and term frequency, and it ranks by cosine similarity rather than
//! binary substring matching. It is deterministic (same text → same vector),
//! offline, and adds zero heavy dependencies.
//!
//! For true neural semantics, this can be swapped for an OpenAI-compatible
//! `/embeddings` endpoint (e.g. Ollama's `nomic-embed-text` for local neural,
//! or OpenAI's `text-embedding-3-*`) — see `EMBEDDING_DIM` and `embed`.

/// Dimensionality of the embedding space. Fixed so vectors are comparable
/// across the whole store. 256 is a good balance of collision-resistance and
/// storage size for the hashing vectorizer.
pub const EMBEDDING_DIM: usize = 256;

/// English stopwords stripped before hashing so common filler words don't
/// dominate similarity. Deliberately small and generic.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "of", "to", "in", "on", "for", "is", "are", "was",
    "were", "be", "been", "it", "this", "that", "with", "as", "at", "by", "from", "i", "you", "we",
    "he", "she", "they", "my", "your",
];

/// Tokenize into lowercase alphanumeric words, dropping stopwords and very
/// short tokens.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .filter(|s| s.len() > 1 && !STOPWORDS.contains(&s.as_str()))
        .collect()
}

/// Deterministic 64-bit FNV-1a hash. Small, fast, no dependencies, stable
/// across runs and platforms (unlike `DefaultHasher`, which is not guaranteed
/// stable and would make stored vectors non-portable).
fn fnv1a(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in s.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Embed `text` into a fixed-dimension, L2-normalized vector using feature
/// hashing over unigrams and bigrams. Returns a zero vector for empty input.
pub fn embed(text: &str) -> Vec<f32> {
    let tokens = tokenize(text);
    let mut vec = vec![0.0f32; EMBEDDING_DIM];

    if tokens.is_empty() {
        return vec;
    }

    for tok in &tokens {
        // Whole-word feature.
        add_feature(&mut vec, tok);
        // Character trigrams (with word-boundary padding) make similarity robust
        // to morphology — "delete"/"deleting", "file"/"files" — and to typos,
        // since related words share most of their trigrams. This is the
        // fastText subword trick applied to a hashing vectorizer.
        for tri in char_ngrams(tok, 3) {
            add_feature(&mut vec, &tri);
        }
    }
    // Bigrams capture short phrases ("fail closed", "api key") that unigrams miss.
    for pair in tokens.windows(2) {
        let bigram = format!("{}_{}", pair[0], pair[1]);
        add_feature(&mut vec, &bigram);
    }

    l2_normalize(&mut vec);
    vec
}

/// Character n-grams of a token, padded with `#` boundaries so prefixes and
/// suffixes are captured (e.g. `#de`, `te#`). Returns the padded whole token
/// too when it is shorter than `n`.
fn char_ngrams(token: &str, n: usize) -> Vec<String> {
    let padded: Vec<char> = format!("#{}#", token).chars().collect();
    if padded.len() < n {
        return vec![padded.into_iter().collect()];
    }
    let mut out = Vec::with_capacity(padded.len() - n + 1);
    for window in padded.windows(n) {
        out.push(window.iter().collect());
    }
    out
}

/// Hash one feature into a bucket, using the hash's sign bit to pick +1/-1 so
/// collisions partly cancel instead of always accumulating (signed hashing).
fn add_feature(vec: &mut [f32], feature: &str) {
    let h = fnv1a(feature);
    let idx = (h % EMBEDDING_DIM as u64) as usize;
    let sign = if (h >> 63) & 1 == 1 { 1.0 } else { -1.0 };
    vec[idx] += sign;
}

fn l2_normalize(vec: &mut [f32]) {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in vec.iter_mut() {
            *x /= norm;
        }
    }
}

/// Cosine similarity of two vectors. Both are L2-normalized by `embed`, so this
/// is just a dot product, but we normalize defensively in case one side comes
/// from an external embedder. Handles differing lengths by using the overlap.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..n {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        assert_eq!(embed("the quick brown fox"), embed("the quick brown fox"));
    }

    #[test]
    fn correct_dimension_and_normalized() {
        let v = embed("kerna runs agents safely");
        assert_eq!(v.len(), EMBEDDING_DIM);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-4,
            "expected unit norm, got {}",
            norm
        );
    }

    #[test]
    fn empty_is_zero_vector() {
        let v = embed("");
        assert_eq!(v.len(), EMBEDDING_DIM);
        assert!(v.iter().all(|x| *x == 0.0));
        // Similarity with anything is 0, not NaN.
        assert_eq!(cosine_similarity(&v, &embed("hello")), 0.0);
    }

    #[test]
    fn similar_texts_rank_above_unrelated() {
        // Query about deleting files should match a file-deletion memory more
        // than an unrelated memory about the weather.
        let query = embed("how do I delete a file safely");
        let related = embed("deleting files requires confirmation for safety");
        let unrelated = embed("the weather today is sunny and warm");

        let sim_related = cosine_similarity(&query, &related);
        let sim_unrelated = cosine_similarity(&query, &unrelated);

        assert!(
            sim_related > sim_unrelated,
            "related {} should exceed unrelated {}",
            sim_related,
            sim_unrelated
        );
    }

    #[test]
    fn identical_text_is_max_similarity() {
        let a = embed("run the agent with a strict budget");
        let sim = cosine_similarity(&a, &a);
        assert!(
            (sim - 1.0).abs() < 1e-4,
            "self-similarity should be ~1, got {}",
            sim
        );
    }
}
