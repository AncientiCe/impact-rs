/// Calls `Vec::len` from the standard library. Nothing here has ever heard of `Basket`,
/// but `len` is a unique name in this project, which used to be enough to report this
/// function as an `Exact` caller of `Basket::len`.
pub fn count_words(words: &[String]) -> usize {
    words.len()
}
