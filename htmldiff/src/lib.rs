mod domdiff;
pub use domdiff::diff_html;

/// Compute an HTML diff between two sanitized HTML fragments, returning HTML with <ins>/<del> markup.
/// Never panics: on any internal failure returns the new string unchanged.
pub fn htmldiff(old: &str, new: &str) -> String {
    match std::panic::catch_unwind(|| domdiff::diff_html(old, new)) {
        Ok(s) => s,
        Err(_) => format!("<del data-diff>{}</del><ins data-diff>{}</ins>", old, new),
    }
}

#[cfg(test)]
mod tests {
    use super::diff_html;

    #[test]
    fn test_mathjax_inline_intra_diff() {
        let old = r"Calculate \( \frac{x^2 + 1}{2} \) for x.";
        let new = r"Calculate \( \frac{x^3 + 1}{2} \) for x.";
        let diff = diff_html(old, new);
        assert_eq!(
            diff,
            r"Calculate \( \frac{x^<del data-diff>2</del><ins data-diff>3</ins> + 1}{2} \) for x."
        );
    }

    #[test]
    fn test_mathjax_unclosed_fallback() {
        let old = r"Broken \( x + 1 text";
        let new = r"Broken \( x + 2 text";
        let diff = diff_html(old, new);
        assert!(diff.contains("<del data-diff>1</del><ins data-diff>2</ins>"));
    }
}
