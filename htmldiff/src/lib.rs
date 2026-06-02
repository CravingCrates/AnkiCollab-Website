mod domdiff;
/// Compute an HTML diff between two sanitized HTML fragments, returning HTML with <ins>/<del> markup.
/// Never panics: on any internal failure returns the new string unchanged.
pub fn htmldiff(old: &str, new: &str) -> String {
	match std::panic::catch_unwind(|| domdiff::diff_html(old, new)) {
		Ok(s) => s,
		Err(_) => format!("<del data-diff>{}</del><ins data-diff>{}</ins>", old, new),
	}
}
