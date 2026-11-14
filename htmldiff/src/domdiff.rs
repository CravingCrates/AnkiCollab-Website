use kuchiki::traits::*;
use kuchiki::{parse_html, ElementData, NodeRef};
use std::collections::{HashMap, HashSet};
use unicode_segmentation::UnicodeSegmentation;

// Threshold to switch from O(n*m) LCS to linear greedy alignment for large sibling lists.
const LCS_THRESHOLD: usize = 200;
const MATHJAX_BR_SENTINEL: &str = "\u{E000}MJXBR\u{E001}";

#[derive(Debug, Clone)]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
    Unchanged,
}

#[derive(Debug, Clone)]
pub struct NodeChange {
    pub kind: ChangeKind,
    pub old: Option<NodeRef>,
    pub new: Option<NodeRef>,
    pub children: Vec<NodeChange>,
}

/// Match a plain `<br>`, `<br/>`, or `<br />` tag (case-insensitive) at the start of `s`.
fn match_br_tag(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.len() < 4 || b[0] != b'<' {
        return None;
    }
    if !b[1].eq_ignore_ascii_case(&b'b') || !b[2].eq_ignore_ascii_case(&b'r') {
        return None;
    }
    let mut k = 3;
    while k < b.len() && b[k].is_ascii_whitespace() {
        k += 1;
    }
    if k < b.len() && b[k] == b'/' {
        k += 1;
        while k < b.len() && b[k].is_ascii_whitespace() {
            k += 1;
        }
    }
    if k < b.len() && b[k] == b'>' {
        Some(k + 1)
    } else {
        None
    }
}

/// Returns true if the backslash at `backslash_idx` is not itself escaped by an odd number
/// of immediately preceding backslashes (so `\\[` in a LaTeX linebreak is not treated as `\[`).
fn is_unescaped_backslash(bytes: &[u8], backslash_idx: usize) -> bool {
    if bytes.get(backslash_idx) != Some(&b'\\') {
        return false;
    }
    let mut count = 0usize;
    let mut k = backslash_idx;
    while k > 0 && bytes[k - 1] == b'\\' {
        count += 1;
        k -= 1;
    }
    count.is_multiple_of(2)
}

/// Replace `<br>`/`<br/>` tags inside closed MathJax blocks (`\(...\)` or `\[...\]`)
/// that contain no other HTML tags and no nested openers, so `kuchiki` keeps the
/// multi-line formula in a single text node during diffing.
fn protect_mathjax_br(s: &str) -> String {
    if (!s.contains("\\(") && !s.contains("\\[")) || !s.to_ascii_lowercase().contains("<br") {
        return s.to_string();
    }

    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut last_copied = 0usize;
    let mut i = 0usize;

    while i + 1 < bytes.len() {
        let closer: Option<&[u8]> =
            if is_unescaped_backslash(bytes, i) && &bytes[i..i + 2] == b"\\(" {
                Some(b"\\)")
            } else if is_unescaped_backslash(bytes, i) && &bytes[i..i + 2] == b"\\[" {
                Some(b"\\]")
            } else {
                None
            };

        let Some(close_seq) = closer else {
            i += 1;
            continue;
        };

        let open_idx = i;
        let mut j = open_idx + 2;
        let mut br_ranges: Vec<(usize, usize)> = Vec::new();
        let mut closed_at: Option<usize> = None;
        let mut restart_at: Option<usize> = None;

        while j + 1 < bytes.len() {
            if is_unescaped_backslash(bytes, j) && &bytes[j..j + 2] == close_seq {
                closed_at = Some(j);
                break;
            }
            if is_unescaped_backslash(bytes, j)
                && (&bytes[j..j + 2] == b"\\(" || &bytes[j..j + 2] == b"\\[")
            {
                restart_at = Some(j);
                break;
            }
            if bytes[j] == b'<' {
                if let Some(tag_len) = match_br_tag(&s[j..]) {
                    br_ranges.push((j, j + tag_len));
                    j += tag_len;
                    continue;
                } else if bytes[j + 1].is_ascii_alphabetic()
                    || bytes[j + 1] == b'/'
                    || bytes[j + 1] == b'!'
                    || bytes[j + 1] == b'?'
                {
                    // Structural or non-<br> HTML tag (or anything html5ever would tokenize as a tag) interrupts the MathJax block.
                    restart_at = Some(j + 1);
                    break;
                }
            }
            j += 1;
        }

        if let Some(close_idx) = closed_at {
            if !br_ranges.is_empty() {
                out.push_str(&s[last_copied..open_idx]);
                let mut cursor = open_idx;
                for (bs, be) in br_ranges {
                    out.push_str(&s[cursor..bs]);
                    out.push_str(MATHJAX_BR_SENTINEL);
                    cursor = be;
                }
                out.push_str(&s[cursor..close_idx + 2]);
                last_copied = close_idx + 2;
            }
            i = close_idx + 2;
        } else if let Some(next_i) = restart_at {
            i = next_i;
        } else {
            break;
        }
    }

    if last_copied == 0 {
        return s.to_string();
    }
    out.push_str(&s[last_copied..]);
    out
}

fn restore_mathjax_br(s: String) -> String {
    if s.contains(MATHJAX_BR_SENTINEL) {
        s.replace(MATHJAX_BR_SENTINEL, "<br>")
    } else {
        s
    }
}

pub fn diff_html(old: &str, new: &str) -> String {
    let old_protected = protect_mathjax_br(old);
    let new_protected = protect_mathjax_br(new);
    // Wrap fragments to avoid full HTML skeleton differences
    let wrapped_old = format!("<div data-htmldiff-root>{}</div>", old_protected);
    let wrapped_new = format!("<div data-htmldiff-root>{}</div>", new_protected);
    let old_doc = parse_html().one(wrapped_old);
    let new_doc = parse_html().one(wrapped_new);
    // Find wrapper elements
    let old_root = find_wrapper(&old_doc).unwrap_or(old_doc);
    let new_root = find_wrapper(&new_doc).unwrap_or(new_doc);
    let change = diff_node(&old_root, &new_root);
    let mut out = String::new();
    for child in &change.children {
        render_change(child, &mut out);
    }
    let mut rendered = finalize_output(out);
    preserve_missing_simple_tokens(&old_protected, &new_protected, &mut rendered);
    restore_mathjax_br(rendered)
}

fn diff_node(old: &NodeRef, new: &NodeRef) -> NodeChange {
    // Root nodes
    if old.parent().is_none() && new.parent().is_none() {
        return NodeChange {
            kind: ChangeKind::Unchanged,
            old: Some(old.clone()),
            new: Some(new.clone()),
            children: diff_children(old, new),
        };
    }
    // Text nodes
    if let (Some(_ot), Some(_nt)) = (old.as_text(), new.as_text()) {
        let o_txt = old.as_text().unwrap().borrow();
        let n_txt = new.as_text().unwrap().borrow();
        if o_txt.as_str() == n_txt.as_str() {
            NodeChange {
                kind: ChangeKind::Unchanged,
                old: Some(old.clone()),
                new: Some(new.clone()),
                children: vec![],
            }
        } else if should_ignore_ws_diff(old, new, &o_txt, &n_txt) {
            // difference only in surrounding whitespace -> ignore noise (context aware)
            NodeChange {
                kind: ChangeKind::Unchanged,
                old: Some(old.clone()),
                new: Some(new.clone()),
                children: vec![],
            }
        } else {
            NodeChange {
                kind: ChangeKind::Modified,
                old: Some(old.clone()),
                new: Some(new.clone()),
                children: vec![],
            }
        }
    } else if let (Some(oel), Some(nel)) = (old.as_element(), new.as_element()) {
        if same_element(oel, nel) {
            let tag = oel.name.local.to_string().to_ascii_lowercase();
            if (tag == "script" || tag == "style") && collect_text(old) != collect_text(new) {
                // treat entire script/style as opaque; emit full replace so both versions appear
                NodeChange {
                    kind: ChangeKind::Modified,
                    old: Some(old.clone()),
                    new: Some(new.clone()),
                    children: vec![],
                }
            } else {
                NodeChange {
                    kind: ChangeKind::Unchanged,
                    old: Some(old.clone()),
                    new: Some(new.clone()),
                    children: diff_children(old, new),
                }
            }
        } else {
            // Equivalent inline formatting tags: treat as unchanged structurally so only deeper text diffs show.
            if is_inline_equiv(oel, nel) {
                NodeChange {
                    kind: ChangeKind::Unchanged,
                    old: Some(old.clone()),
                    new: Some(new.clone()),
                    children: diff_children(old, new),
                }
            } else if oel.name.local == nel.name.local {
                // attributes differ
                NodeChange {
                    kind: ChangeKind::Modified,
                    old: Some(old.clone()),
                    new: Some(new.clone()),
                    children: diff_children(old, new),
                }
            } else {
                NodeChange {
                    kind: ChangeKind::Modified,
                    old: Some(old.clone()),
                    new: Some(new.clone()),
                    children: vec![],
                }
            }
        }
    } else {
        NodeChange {
            kind: ChangeKind::Modified,
            old: Some(old.clone()),
            new: Some(new.clone()),
            children: vec![],
        }
    }
}

fn same_element(a: &ElementData, b: &ElementData) -> bool {
    if a.name.local != b.name.local {
        return false;
    }
    let a_binding = a.attributes.borrow();
    let b_binding = b.attributes.borrow();
    if a_binding.map.len() != b_binding.map.len() {
        return false;
    }
    for (k, va) in a_binding.map.iter() {
        if let Some(vb) = b_binding.map.get(k) {
            if va != vb {
                return false;
            }
        } else {
            return false;
        }
    }
    true
}

// Child diff using LCS on signatures.
fn diff_children(old: &NodeRef, new: &NodeRef) -> Vec<NodeChange> {
    let old_children: Vec<_> = old.children().collect();
    let new_children: Vec<_> = new.children().collect();
    let old_sig: Vec<String> = old_children.iter().map(signature).collect();
    let new_sig: Vec<String> = new_children.iter().map(signature).collect();

    if old_sig.len() > LCS_THRESHOLD || new_sig.len() > LCS_THRESHOLD {
        return greedy_children_diff(&old_children, &new_children, &old_sig, &new_sig);
    }

    let lcs = lcs_table(&old_sig, &new_sig);
    // Backtrack to mark matches
    let mut matches = vec![]; // pairs (i,j)
    let mut i = old_sig.len();
    let mut j = new_sig.len();
    while i > 0 && j > 0 {
        if old_sig[i - 1] == new_sig[j - 1] {
            matches.push((i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if lcs[i - 1][j] >= lcs[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    matches.reverse();
    // Build changes
    let mut res = Vec::new();
    let mut oi = 0usize;
    let mut nj = 0usize;
    let mut m_idx = 0usize;
    while oi < old_children.len() || nj < new_children.len() {
        if m_idx < matches.len() {
            let (mi, mj) = matches[m_idx];
            if oi == mi && nj == mj {
                // matched pair
                res.push(diff_node(&old_children[oi], &new_children[nj]));
                oi += 1;
                nj += 1;
                m_idx += 1;
                continue;
            }
        }
        // If current old index is part of next match but new isn't, then additions before match
        if m_idx < matches.len() {
            let (mi, mj) = matches[m_idx];
            if oi < mi && nj < mj {
                // ambiguous; treat as modification by position
                res.push(NodeChange {
                    kind: ChangeKind::Modified,
                    old: Some(old_children[oi].clone()),
                    new: Some(new_children[nj].clone()),
                    children: vec![],
                });
                oi += 1;
                nj += 1;
                continue;
            }
            if oi < mi {
                // removals until match
                res.push(NodeChange {
                    kind: ChangeKind::Removed,
                    old: Some(old_children[oi].clone()),
                    new: None,
                    children: vec![],
                });
                oi += 1;
                continue;
            }
            if nj < mj {
                // additions until match
                res.push(NodeChange {
                    kind: ChangeKind::Added,
                    old: None,
                    new: Some(new_children[nj].clone()),
                    children: vec![],
                });
                nj += 1;
                continue;
            }
        } else {
            // No more matches
            if oi < old_children.len() && nj < new_children.len() {
                // Recurse even if signature didn't match to attempt finer-grained diff inside
                res.push(diff_node(&old_children[oi], &new_children[nj]));
                oi += 1;
                nj += 1;
                continue;
            } else if oi < old_children.len() {
                res.push(NodeChange {
                    kind: ChangeKind::Removed,
                    old: Some(old_children[oi].clone()),
                    new: None,
                    children: vec![],
                });
                oi += 1;
                continue;
            } else if nj < new_children.len() {
                res.push(NodeChange {
                    kind: ChangeKind::Added,
                    old: None,
                    new: Some(new_children[nj].clone()),
                    children: vec![],
                });
                nj += 1;
                continue;
            }
        }
    }
    res
}

fn lcs_table(a: &[String], b: &[String]) -> Vec<Vec<usize>> {
    let mut dp = vec![vec![0; b.len() + 1]; a.len() + 1];
    for i in 0..a.len() {
        for j in 0..b.len() {
            if a[i] == b[j] {
                dp[i + 1][j + 1] = dp[i][j] + 1;
            } else {
                dp[i + 1][j + 1] = dp[i + 1][j].max(dp[i][j + 1]);
            }
        }
    }
    dp
}

fn signature(n: &NodeRef) -> String {
    if let Some(el) = n.as_element() {
        let attrs = el.attributes.borrow();
        let id = attrs.get("id").unwrap_or("");
        let class = attrs.get("class").unwrap_or("");
        // normalize attribute key ordering for stability
        let mut attr_keys: Vec<_> = attrs.map.keys().map(|k| k.local.to_string()).collect();
        attr_keys.sort_unstable();
        let first_text = first_text_child(n);
        let snippet = first_n_chars(&first_text, 16);
        format!(
            "E:{}#{}.{},{}|{}|{}",
            el.name.local,
            id,
            class,
            attrs.map.len(),
            attr_keys.join(","),
            snippet
        )
    } else if let Some(t) = n.as_text() {
        let txt = t.borrow();
        let trimmed = txt.trim();
        let short = first_n_chars(trimmed, 16);
        format!("T:{}", short)
    } else {
        "O".into()
    }
}

fn first_text_child(n: &NodeRef) -> String {
    for child in n.children() {
        if let Some(t) = child.as_text() {
            let s = t.borrow();
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    String::new()
}

fn render_change(change: &NodeChange, out: &mut String) {
    match change.kind {
        ChangeKind::Unchanged => render_node(change, out),
        ChangeKind::Added => {
            out.push_str("<ins data-diff>");
            render_node(change, out);
            out.push_str("</ins>");
        }
        ChangeKind::Removed => {
            out.push_str("<del data-diff>");
            if let Some(old) = &change.old {
                render_single_node(old, out);
            } else {
                for ch in &change.children {
                    render_change(ch, out);
                }
            }
            out.push_str("</del>");
        }
        ChangeKind::Modified => {
            if is_text_change(change) {
                // inline word diff / minimization
                let (old_t, new_t) = text_pair(change).unwrap();
                if old_t == new_t {
                    // identical text (should normally be Unchanged already)
                    out.push_str(&new_t);
                    return;
                }
                if old_t.trim() == new_t.trim() {
                    // Only suppress whitespace-only differences outside pre/code/textarea
                    let parent_is_pre_like = change
                        .old
                        .as_ref()
                        .and_then(parent_tag)
                        .map(|t| t == "pre" || t == "code" || t == "textarea")
                        .unwrap_or(false)
                        || change
                            .new
                            .as_ref()
                            .and_then(parent_tag)
                            .map(|t| t == "pre" || t == "code" || t == "textarea")
                            .unwrap_or(false);
                    if !parent_is_pre_like {
                        out.push_str(&new_t);
                        return;
                    }
                }
                // Compute minimal differing middle using char-based prefix/suffix to avoid multi-byte split.
                let (prefix, old_mid, new_mid, suffix) = split_diff_regions(&old_t, &new_t);
                // Large pure append or truncate: show full replacement so new string appears intact (helps certain consumers/tests)
                if suffix.is_empty() && prefix.len() == old_t.len() {
                    // appended text
                    if new_t.len() - old_t.len() > 2 {
                        // threshold
                        out.push_str("<del data-diff>");
                        out.push_str(&old_t);
                        out.push_str("</del><ins data-diff>");
                        out.push_str(&new_t);
                        out.push_str("</ins>");
                        return;
                    }
                }
                if suffix.is_empty() && prefix.len() == new_t.len() {
                    // truncated
                    if old_t.len() - new_t.len() > 2 {
                        out.push_str("<del data-diff>");
                        out.push_str(&old_t);
                        out.push_str("</del><ins data-diff>");
                        out.push_str(&new_t);
                        out.push_str("</ins>");
                        return;
                    }
                }
                if !prefix.is_empty() {
                    out.push_str(prefix);
                }
                if old_mid.is_empty() && !new_mid.is_empty() {
                    out.push_str("<ins data-diff>");
                    out.push_str(new_mid);
                    out.push_str("</ins>");
                } else if new_mid.is_empty() && !old_mid.is_empty() {
                    out.push_str("<del data-diff>");
                    out.push_str(old_mid);
                    out.push_str("</del>");
                } else if !try_mathjax_intra_diff(old_mid, new_mid, out)
                    && !try_char_level_highlight(old_mid, new_mid, out)
                {
                    out.push_str("<del data-diff>");
                    out.push_str(old_mid);
                    out.push_str("</del><ins data-diff>");
                    out.push_str(new_mid);
                    out.push_str("</ins>");
                }
                if !suffix.is_empty() {
                    out.push_str(suffix);
                }
            } else if is_element_with_modified_attrs(change) {
                // Replace opening tag only (simpler: full element replace)
                out.push_str("<del data-diff>");
                if let Some(o) = &change.old {
                    render_single_node(o, out);
                }
                out.push_str("</del><ins data-diff>");
                if let Some(n) = &change.new {
                    render_single_node(n, out);
                }
                out.push_str("</ins>");
            } else {
                out.push_str("<del data-diff>");
                if let Some(o) = &change.old {
                    render_single_node(o, out);
                }
                out.push_str("</del><ins data-diff>");
                if let Some(n) = &change.new {
                    render_single_node(n, out);
                }
                out.push_str("</ins>");
            }
        }
    }
}

fn is_text_change(c: &NodeChange) -> bool {
    c.old.as_ref().and_then(|n| n.as_text()).is_some()
        && c.new.as_ref().and_then(|n| n.as_text()).is_some()
}
fn text_pair(c: &NodeChange) -> Option<(String, String)> {
    Some((
        c.old.as_ref()?.as_text()?.borrow().to_string(),
        c.new.as_ref()?.as_text()?.borrow().to_string(),
    ))
}
fn is_element_with_modified_attrs(c: &NodeChange) -> bool {
    if let (Some(o), Some(n)) = (&c.old, &c.new) {
        if let (Some(oe), Some(ne)) = (o.as_element(), n.as_element()) {
            return oe.name.local == ne.name.local;
        }
    }
    false
}

fn write_sorted_attrs(el: &ElementData, out: &mut String) {
    let attrs = el.attributes.borrow();
    // sort attributes by name for deterministic output
    let mut keys: Vec<_> = attrs.map.keys().map(|k| k.local.to_string()).collect();
    keys.sort_unstable();
    for k in keys {
        if let Some(v) = attrs.get(k.as_str()) {
            out.push(' ');
            out.push_str(k.as_str());
            out.push('=');
            out.push('"');
            out.push_str(v);
            out.push('"');
        }
    }
}

fn render_node(change: &NodeChange, out: &mut String) {
    if let Some(n) = change.new.as_ref().or(change.old.as_ref()) {
        if let Some(el) = n.as_element() {
            out.push('<');
            out.push_str(el.name.local.as_ref());
            write_sorted_attrs(el, out);
            out.push('>');
            if change.children.is_empty() {
                // raw render
                for child in n.children() {
                    render_subtree(&child, out);
                }
            } else {
                for ch in &change.children {
                    render_change(ch, out);
                }
            }
            out.push_str("</");
            out.push_str(el.name.local.as_ref());
            out.push('>');
        } else if let Some(t) = n.as_text() {
            out.push_str(t.borrow().as_ref());
        } else {
            for ch in &change.children {
                render_change(ch, out);
            }
        }
    }
}

fn render_subtree(node: &NodeRef, out: &mut String) {
    if let Some(el) = node.as_element() {
        out.push('<');
        out.push_str(el.name.local.as_ref());
        write_sorted_attrs(el, out);
        out.push('>');
        for c in node.children() {
            render_subtree(&c, out);
        }
        out.push_str("</");
        out.push_str(el.name.local.as_ref());
        out.push('>');
    } else if let Some(t) = node.as_text() {
        out.push_str(t.borrow().as_ref());
    }
}

fn render_single_node(n: &NodeRef, out: &mut String) {
    if let Some(el) = n.as_element() {
        out.push('<');
        out.push_str(el.name.local.as_ref());
        write_sorted_attrs(el, out);
        out.push('>');
        for child in n.children() {
            render_single_node(&child, out);
        }
        out.push_str("</");
        out.push_str(el.name.local.as_ref());
        out.push('>');
    } else if let Some(t) = n.as_text() {
        out.push_str(t.borrow().as_ref());
    }
}

fn find_wrapper(doc: &NodeRef) -> Option<NodeRef> {
    for desc in doc.descendants() {
        if let Some(el) = desc.as_element() {
            let attrs = el.attributes.borrow();
            if attrs.contains("data-htmldiff-root") {
                return Some(desc.clone());
            }
        }
    }
    None
}

fn collect_text(node: &NodeRef) -> String {
    let mut acc = String::new();
    for child in node.descendants() {
        if let Some(t) = child.as_text() {
            acc.push_str(t.borrow().as_ref());
        }
    }
    acc
}

// Return the first n Unicode scalar values of s without splitting a multibyte character.
fn first_n_chars(s: &str, n: usize) -> &str {
    if n == 0 {
        return "";
    }
    // Grapheme-aware prefix boundary
    let mut count = 0;
    let mut byte_idx = 0;
    for g in s.graphemes(true) {
        if count == n {
            break;
        }
        byte_idx += g.len();
        count += 1;
    }
    if count < n {
        s
    } else {
        &s[..byte_idx]
    }
}

/// When a word-level replacement has only a tiny character change (1–3 graphemes),
/// emits del/ins with inner `<span data-diff-char>` markers around the exact
/// changed characters, creating a subtle deeper highlight.  Returns false if the
/// change is too large for char-level highlighting.
fn try_char_level_highlight(old_mid: &str, new_mid: &str, out: &mut String) -> bool {
    if old_mid.contains("\\(")
        || old_mid.contains("\\[")
        || new_mid.contains("\\(")
        || new_mid.contains("\\[")
    {
        return false;
    }

    let old_gr: Vec<&str> = old_mid.graphemes(true).collect();
    let new_gr: Vec<&str> = new_mid.graphemes(true).collect();
    if old_gr.is_empty() || new_gr.is_empty() {
        return false;
    }

    let mut pre = 0;
    while pre < old_gr.len() && pre < new_gr.len() && old_gr[pre] == new_gr[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < old_gr.len().saturating_sub(pre)
        && suf < new_gr.len().saturating_sub(pre)
        && old_gr[old_gr.len() - 1 - suf] == new_gr[new_gr.len() - 1 - suf]
    {
        suf += 1;
    }

    let old_changed = old_gr.len() - pre - suf;
    let new_changed = new_gr.len() - pre - suf;
    let max_changed = old_changed.max(new_changed);
    let min_total = old_gr.len().min(new_gr.len());

    // Only highlight if: small change (1–3 graphemes), word is meaningful (≥3),
    // and the change is less than half the word.
    if max_changed == 0 || max_changed > 3 || min_total < 3 {
        return false;
    }
    if max_changed * 2 > min_total {
        return false;
    }

    // Emit <del> with inner char highlight
    out.push_str("<del data-diff>");
    for g in &old_gr[..pre] {
        out.push_str(g);
    }
    if old_changed > 0 {
        out.push_str("<span data-diff-char>");
        for g in &old_gr[pre..old_gr.len() - suf] {
            out.push_str(g);
        }
        out.push_str("</span>");
    }
    for g in &old_gr[old_gr.len() - suf..] {
        out.push_str(g);
    }
    out.push_str("</del>");

    // Emit <ins> with inner char highlight
    out.push_str("<ins data-diff>");
    for g in &new_gr[..pre] {
        out.push_str(g);
    }
    if new_changed > 0 {
        out.push_str("<span data-diff-char>");
        for g in &new_gr[pre..new_gr.len() - suf] {
            out.push_str(g);
        }
        out.push_str("</span>");
    }
    for g in &new_gr[new_gr.len() - suf..] {
        out.push_str(g);
    }
    out.push_str("</ins>");

    true
}

/// Locate closed inline `\(...\)` and display `\[...\]` MathJax spans in a text node.
/// Unclosed openers (or openers interrupted by another opener) are skipped.
fn find_mathjax_spans(s: &str) -> Vec<(usize, usize)> {
    if !s.contains("\\(") && !s.contains("\\[") {
        return Vec::new();
    }
    let mut spans = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        let close_seq: Option<&[u8]> =
            if is_unescaped_backslash(bytes, i) && &bytes[i..i + 2] == b"\\(" {
                Some(b"\\)")
            } else if is_unescaped_backslash(bytes, i) && &bytes[i..i + 2] == b"\\[" {
                Some(b"\\]")
            } else {
                None
            };
        let Some(closer) = close_seq else {
            i += 1;
            continue;
        };
        let mut j = i + 2;
        let mut restarted = false;
        let mut closed = false;
        while j + 1 < bytes.len() {
            if is_unescaped_backslash(bytes, j) && &bytes[j..j + 2] == closer {
                spans.push((i, j + 2));
                i = j + 2;
                closed = true;
                break;
            }
            if is_unescaped_backslash(bytes, j)
                && (&bytes[j..j + 2] == b"\\(" || &bytes[j..j + 2] == b"\\[")
            {
                i = j;
                restarted = true;
                break;
            }
            j += 1;
        }
        if !closed && !restarted {
            break;
        }
    }
    spans
}

const MAX_TEX_LCS_TOKENS: usize = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TexKind {
    Whitespace,
    BrSentinel,
    Command,
    OpenBrace,
    CloseBrace,
    ClozeOpen,
    LeftDelim,
    RightDelim,
    BeginEnv,
    EndEnv,
    OpenBracket,
    CloseBracket,
    SupSub,
    Number,
    Ident,
    Operator,
    Other,
}

#[derive(Debug, Clone, Copy)]
struct TexToken {
    start: usize,
    end: usize,
    kind: TexKind,
}

impl TexToken {
    fn lexeme(self, s: &str) -> &str {
        &s[self.start..self.end]
    }

    fn is_meaningful(self) -> bool {
        !matches!(
            self.kind,
            TexKind::Whitespace | TexKind::BrSentinel | TexKind::OpenBrace | TexKind::CloseBrace
        )
    }
}

/// Tokenizes inner TeX source without splitting commands, cloze openers, `\begin`/`\end`,
/// `\left`/`\right`, `<br>` sentinels, numbers, identifiers, or multi-char arrows.
fn tokenize_tex(s: &str) -> Vec<TexToken> {
    let mut tokens = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        if s[i..].starts_with(MATHJAX_BR_SENTINEL) {
            let end = i + MATHJAX_BR_SENTINEL.len();
            tokens.push(TexToken {
                start: i,
                end,
                kind: TexKind::BrSentinel,
            });
            i = end;
            continue;
        }

        let b = bytes[i];
        if b == b'\\' {
            if i + 1 >= bytes.len() {
                tokens.push(TexToken {
                    start: i,
                    end: i + 1,
                    kind: TexKind::Operator,
                });
                i += 1;
                continue;
            }
            if bytes[i + 1].is_ascii_alphabetic() {
                let mut end = i + 2;
                while end < bytes.len() && bytes[end].is_ascii_alphabetic() {
                    end += 1;
                }
                let cmd = &s[i..end];
                if cmd == "\\begin" || cmd == "\\end" {
                    let kind = if cmd == "\\begin" {
                        TexKind::BeginEnv
                    } else {
                        TexKind::EndEnv
                    };
                    let mut k = end;
                    while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t') {
                        k += 1;
                    }
                    if k < bytes.len() && bytes[k] == b'{' {
                        if let Some(rel_close) = s[k + 1..].find('}') {
                            let env_inner = &s[k + 1..k + 1 + rel_close];
                            if !env_inner.contains('{') && !env_inner.contains('\n') {
                                end = k + 1 + rel_close + 1;
                            }
                        }
                    }
                    tokens.push(TexToken {
                        start: i,
                        end,
                        kind,
                    });
                    i = end;
                    continue;
                }
                if cmd == "\\left" || cmd == "\\right" {
                    let kind = if cmd == "\\left" {
                        TexKind::LeftDelim
                    } else {
                        TexKind::RightDelim
                    };
                    let mut k = end;
                    while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t') {
                        k += 1;
                    }
                    if k < bytes.len() {
                        if bytes[k] == b'\\' {
                            if k + 1 < bytes.len() && bytes[k + 1].is_ascii_alphabetic() {
                                let mut d_end = k + 2;
                                while d_end < bytes.len() && bytes[d_end].is_ascii_alphabetic() {
                                    d_end += 1;
                                }
                                end = d_end;
                            } else if k + 1 < bytes.len() {
                                let ch_len = s[k + 1..].chars().next().map_or(1, char::len_utf8);
                                end = k + 1 + ch_len;
                            }
                        } else {
                            let ch_len = s[k..].chars().next().map_or(1, char::len_utf8);
                            end = k + ch_len;
                        }
                    }
                    tokens.push(TexToken {
                        start: i,
                        end,
                        kind,
                    });
                    i = end;
                    continue;
                }
                tokens.push(TexToken {
                    start: i,
                    end,
                    kind: TexKind::Command,
                });
                i = end;
                continue;
            }
            let ch_len = s[i + 1..].chars().next().map_or(1, char::len_utf8);
            let end = i + 1 + ch_len;
            tokens.push(TexToken {
                start: i,
                end,
                kind: TexKind::Command,
            });
            i = end;
            continue;
        }

        if b == b'{' {
            if i + 5 < bytes.len()
                && bytes[i + 1] == b'{'
                && (bytes[i + 2] == b'c' || bytes[i + 2] == b'C')
            {
                let mut k = i + 3;
                while k < bytes.len() && bytes[k].is_ascii_digit() {
                    k += 1;
                }
                if k > i + 3 && k + 1 < bytes.len() && bytes[k] == b':' && bytes[k + 1] == b':' {
                    let end = k + 2;
                    tokens.push(TexToken {
                        start: i,
                        end,
                        kind: TexKind::ClozeOpen,
                    });
                    i = end;
                    continue;
                }
            }
            tokens.push(TexToken {
                start: i,
                end: i + 1,
                kind: TexKind::OpenBrace,
            });
            i += 1;
            continue;
        }
        if b == b'}' {
            tokens.push(TexToken {
                start: i,
                end: i + 1,
                kind: TexKind::CloseBrace,
            });
            i += 1;
            continue;
        }
        if b == b'[' {
            tokens.push(TexToken {
                start: i,
                end: i + 1,
                kind: TexKind::OpenBracket,
            });
            i += 1;
            continue;
        }
        if b == b']' {
            tokens.push(TexToken {
                start: i,
                end: i + 1,
                kind: TexKind::CloseBracket,
            });
            i += 1;
            continue;
        }
        if b == b'^' || b == b'_' {
            tokens.push(TexToken {
                start: i,
                end: i + 1,
                kind: TexKind::SupSub,
            });
            i += 1;
            continue;
        }
        if b.is_ascii_digit() {
            let mut end = i + 1;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            tokens.push(TexToken {
                start: i,
                end,
                kind: TexKind::Number,
            });
            i = end;
            continue;
        }
        if b.is_ascii_alphabetic() {
            let mut end = i + 1;
            while end < bytes.len() && bytes[end].is_ascii_alphabetic() {
                end += 1;
            }
            tokens.push(TexToken {
                start: i,
                end,
                kind: TexKind::Ident,
            });
            i = end;
            continue;
        }
        if b == b' ' || b == b'\t' {
            let mut end = i + 1;
            while end < bytes.len() && (bytes[end] == b' ' || bytes[end] == b'\t') {
                end += 1;
            }
            tokens.push(TexToken {
                start: i,
                end,
                kind: TexKind::Whitespace,
            });
            i = end;
            continue;
        }
        if s[i..].starts_with("<->") {
            tokens.push(TexToken {
                start: i,
                end: i + 3,
                kind: TexKind::Operator,
            });
            i += 3;
            continue;
        }
        if ["->", "<-", "<=", ">=", "!=", "==", ":="]
            .iter()
            .any(|op| s[i..].starts_with(op))
        {
            tokens.push(TexToken {
                start: i,
                end: i + 2,
                kind: TexKind::Operator,
            });
            i += 2;
            continue;
        }

        let ch = s[i..].chars().next().unwrap();
        let end = i + ch.len_utf8();
        let kind = if ch.is_whitespace() {
            TexKind::Whitespace
        } else if ch.is_ascii() {
            TexKind::Operator
        } else {
            TexKind::Other
        };
        tokens.push(TexToken {
            start: i,
            end,
            kind,
        });
        i = end;
    }

    tokens
}

/// Number of mandatory brace/atom arguments taken by common TeX/LaTeX math commands.
fn tex_command_arity(cmd: &str) -> usize {
    match cmd {
        "\\frac" | "\\dfrac" | "\\tfrac" | "\\cfrac" | "\\binom" | "\\dbinom" | "\\tbinom"
        | "\\overset" | "\\underset" | "\\stackrel" | "\\textcolor" | "\\dv" | "\\pdv"
        | "\\fdv" | "\\cancelto" => 2,
        "\\genfrac" => 6,
        "\\sideset" => 3,
        "\\sqrt"
        | "\\vec"
        | "\\hat"
        | "\\widehat"
        | "\\bar"
        | "\\overline"
        | "\\underline"
        | "\\tilde"
        | "\\widetilde"
        | "\\dot"
        | "\\ddot"
        | "\\dddot"
        | "\\overbrace"
        | "\\underbrace"
        | "\\overleftarrow"
        | "\\overrightarrow"
        | "\\overleftrightarrow"
        | "\\xleftarrow"
        | "\\xrightarrow"
        | "\\text"
        | "\\textbf"
        | "\\textit"
        | "\\textrm"
        | "\\textsf"
        | "\\texttt"
        | "\\mathrm"
        | "\\mathbf"
        | "\\mathit"
        | "\\mathsf"
        | "\\mathtt"
        | "\\mathcal"
        | "\\mathbb"
        | "\\mathfrak"
        | "\\mathscr"
        | "\\boldsymbol"
        | "\\bm"
        | "\\pmb"
        | "\\operatorname"
        | "\\pmod"
        | "\\pod"
        | "\\phantom"
        | "\\hphantom"
        | "\\vphantom"
        | "\\cancel"
        | "\\bcancel"
        | "\\xcancel"
        | "\\boxed"
        | "\\ce"
        | "\\pu"
        | "\\bbox" => 1,
        _ => 0,
    }
}

fn tex_command_accepts_optional_arg(cmd: &str) -> bool {
    matches!(
        cmd,
        "\\sqrt" | "\\xleftarrow" | "\\xrightarrow" | "\\bbox" | "\\\\"
    )
}

/// Consume optional whitespace and `[...]` brackets, returning index after optional arg.
fn skip_optional_bracket_arg(toks: &[TexToken], mut idx: usize, end: usize) -> Option<usize> {
    while idx < end && toks[idx].kind == TexKind::Whitespace {
        idx += 1;
    }
    if idx < end && toks[idx].kind == TexKind::OpenBracket {
        let mut depth = 1usize;
        idx += 1;
        while idx < end && depth > 0 {
            match toks[idx].kind {
                TexKind::OpenBracket => depth += 1,
                TexKind::CloseBracket => depth -= 1,
                _ => {}
            }
            idx += 1;
        }
        if depth > 0 {
            return None;
        }
    }
    Some(idx)
}

/// Checks if a command at `toks[cmd_idx]` completes `arity` mandatory args inside `[start..end]`.
fn command_args_complete(toks: &[TexToken], cmd_idx: usize, end: usize, arity: usize) -> bool {
    let mut idx = cmd_idx + 1;
    if tex_command_accepts_optional_arg_cmd(toks[cmd_idx].kind) {
        let Some(next) = skip_optional_bracket_arg(toks, idx, end) else {
            return false;
        };
        idx = next;
    }
    for _ in 0..arity {
        while idx < end && toks[idx].kind == TexKind::Whitespace {
            idx += 1;
        }
        if idx >= end {
            return false;
        }
        if matches!(toks[idx].kind, TexKind::OpenBrace | TexKind::ClozeOpen) {
            let mut depth = if toks[idx].kind == TexKind::ClozeOpen {
                2usize
            } else {
                1usize
            };
            idx += 1;
            while idx < end && depth > 0 {
                match toks[idx].kind {
                    TexKind::OpenBrace => depth += 1,
                    TexKind::ClozeOpen => depth += 2,
                    TexKind::CloseBrace => depth = depth.saturating_sub(1),
                    _ => {}
                }
                idx += 1;
            }
            if depth > 0 {
                return false;
            }
        } else {
            idx += 1;
        }
    }
    true
}

fn tex_command_accepts_optional_arg_cmd(kind: TexKind) -> bool {
    kind == TexKind::Command
}

/// Checks structural balance of `{}` / cloze / `\left..\right` / `\begin..\end` and dangling commands/`^`/`_`
/// on `src_toks[start..end]`. Returns `(needs_expand_left, needs_expand_right)`.
fn tex_hunk_needs_expansion(
    src: &str,
    toks: &[TexToken],
    start: usize,
    end: usize,
) -> (bool, bool) {
    let mut needs_left = false;
    let mut needs_right = false;

    let mut brace_depth = 0isize;
    let mut lr_depth = 0isize;
    let mut env_depth = 0isize;

    for idx in start..end {
        match toks[idx].kind {
            TexKind::OpenBrace => brace_depth += 1,
            TexKind::ClozeOpen => brace_depth += 2,
            TexKind::CloseBrace => {
                brace_depth -= 1;
                if brace_depth < 0 {
                    needs_left = true;
                    brace_depth = 0;
                }
            }
            TexKind::LeftDelim => lr_depth += 1,
            TexKind::RightDelim => {
                lr_depth -= 1;
                if lr_depth < 0 {
                    needs_left = true;
                    lr_depth = 0;
                }
            }
            TexKind::BeginEnv => env_depth += 1,
            TexKind::EndEnv => {
                env_depth -= 1;
                if env_depth < 0 {
                    needs_left = true;
                    env_depth = 0;
                }
            }
            TexKind::Command => {
                let lex = toks[idx].lexeme(src);
                let arity = tex_command_arity(lex);
                if arity > 0 && !command_args_complete(toks, idx, end, arity) {
                    needs_right = true;
                }
            }
            _ => {}
        }
    }

    if brace_depth > 0 || lr_depth > 0 || env_depth > 0 {
        needs_right = true;
    }

    // Dangling trailing `^` or `_` inside the hunk needs its following script atom.
    if let Some(last) = toks[start..end]
        .iter()
        .rev()
        .find(|t| t.kind != TexKind::Whitespace)
    {
        if last.kind == TexKind::SupSub {
            needs_right = true;
        }
    }

    // Leading `[` right after an optional-arg command (e.g. `\sqrt` `[3]` `{x}` or `\\[4pt]`)
    // should pull in the command and its mandatory argument.
    if start > 0 {
        if let Some(first) = toks[start..end]
            .iter()
            .find(|t| t.kind != TexKind::Whitespace)
        {
            if first.kind == TexKind::OpenBracket
                && tex_command_accepts_optional_arg(toks[start - 1].lexeme(src))
            {
                needs_left = true;
            }
        }
    }

    // Unclosed `[` right after an optional-arg command inside the hunk.
    for idx in start..end {
        if toks[idx].kind == TexKind::Command
            && tex_command_accepts_optional_arg(toks[idx].lexeme(src))
            && skip_optional_bracket_arg(toks, idx + 1, end).is_none()
        {
            needs_right = true;
        }
    }

    (needs_left, needs_right)
}

fn collect_tex_hunks(
    old_match: &[Option<usize>],
    new_match: &[Option<usize>],
) -> Vec<(usize, usize, usize, usize)> {
    let mut hunks = Vec::new();
    let mut oi = 0usize;
    let mut ni = 0usize;
    while oi < old_match.len() || ni < new_match.len() {
        while oi < old_match.len() && ni < new_match.len() && old_match[oi] == Some(ni) {
            oi += 1;
            ni += 1;
        }
        if oi >= old_match.len() && ni >= new_match.len() {
            break;
        }
        let o_s = oi;
        let n_s = ni;
        while oi < old_match.len() && old_match[oi].is_none() {
            oi += 1;
        }
        while ni < new_match.len() && new_match[ni].is_none() {
            ni += 1;
        }
        hunks.push((o_s, oi, n_s, ni));
    }
    hunks
}

/// Aligns matching TeX tokens across `old_in` and `new_in`, then expands any hunk that would
/// leave unbalanced TeX groups/environments or dangling command arguments. Returns formatted
/// inner diff markup or `None` when a whole-formula diff is clearer.
fn diff_tex_inner(old_in: &str, new_in: &str) -> Option<String> {
    let old_toks = tokenize_tex(old_in);
    let new_toks = tokenize_tex(new_in);
    if old_toks.is_empty() || new_toks.is_empty() {
        return None;
    }

    let mut old_match = vec![None; old_toks.len()];
    let mut new_match = vec![None; new_toks.len()];

    let mut pre = 0usize;
    while pre < old_toks.len()
        && pre < new_toks.len()
        && old_toks[pre].lexeme(old_in) == new_toks[pre].lexeme(new_in)
    {
        old_match[pre] = Some(pre);
        new_match[pre] = Some(pre);
        pre += 1;
    }

    let mut suf = 0usize;
    while suf < old_toks.len() - pre && suf < new_toks.len() - pre {
        let oi = old_toks.len() - 1 - suf;
        let ni = new_toks.len() - 1 - suf;
        if old_toks[oi].lexeme(old_in) != new_toks[ni].lexeme(new_in) {
            break;
        }
        old_match[oi] = Some(ni);
        new_match[ni] = Some(oi);
        suf += 1;
    }

    let o_mid_end = old_toks.len() - suf;
    let n_mid_end = new_toks.len() - suf;
    if pre < o_mid_end && pre < n_mid_end {
        let m = o_mid_end - pre;
        let n = n_mid_end - pre;
        if m > MAX_TEX_LCS_TOKENS || n > MAX_TEX_LCS_TOKENS {
            return None;
        }
        let mut dp = vec![0u32; (m + 1) * (n + 1)];
        let stride = n + 1;
        for i in (0..m).rev() {
            for j in (0..n).rev() {
                dp[i * stride + j] =
                    if old_toks[pre + i].lexeme(old_in) == new_toks[pre + j].lexeme(new_in) {
                        dp[(i + 1) * stride + (j + 1)] + 1
                    } else {
                        dp[(i + 1) * stride + j].max(dp[i * stride + (j + 1)])
                    };
            }
        }
        let mut i = 0usize;
        let mut j = 0usize;
        while i < m && j < n {
            if old_toks[pre + i].lexeme(old_in) == new_toks[pre + j].lexeme(new_in) {
                old_match[pre + i] = Some(pre + j);
                new_match[pre + j] = Some(pre + i);
                i += 1;
                j += 1;
            } else if dp[(i + 1) * stride + j] >= dp[i * stride + (j + 1)] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }

    loop {
        let hunks = collect_tex_hunks(&old_match, &new_match);
        let mut changed = false;

        for &(o_s, o_e, n_s, n_e) in &hunks {
            let (ol, or) = tex_hunk_needs_expansion(old_in, &old_toks, o_s, o_e);
            let (nl, nr) = tex_hunk_needs_expansion(new_in, &new_toks, n_s, n_e);
            if ol || nl {
                if o_s == 0 || n_s == 0 {
                    return None;
                }
                old_match[o_s - 1] = None;
                new_match[n_s - 1] = None;
                changed = true;
            }
            if or || nr {
                if o_e >= old_toks.len() || n_e >= new_toks.len() {
                    return None;
                }
                old_match[o_e] = None;
                new_match[n_e] = None;
                changed = true;
            }
        }

        if changed {
            continue;
        }

        // Coalesce consecutive hunks separated by a single horizontal whitespace token.
        for pair in hunks.windows(2) {
            let left = pair[0];
            let right = pair[1];
            if right.0 == left.1 + 1
                && right.2 == left.3 + 1
                && old_toks[left.1].kind == TexKind::Whitespace
                && !old_toks[left.1].lexeme(old_in).contains('\n')
            {
                old_match[left.1] = None;
                new_match[left.3] = None;
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    let final_hunks = collect_tex_hunks(&old_match, &new_match);
    if final_hunks.is_empty() {
        return None;
    }

    let old_sig = old_toks.iter().filter(|t| t.is_meaningful()).count();
    let new_sig = new_toks.iter().filter(|t| t.is_meaningful()).count();
    let kept_sig = old_toks
        .iter()
        .enumerate()
        .filter(|&(i, t)| old_match[i].is_some() && t.is_meaningful())
        .count();
    let max_sig = old_sig.max(new_sig);
    if kept_sig == 0 || (max_sig > 0 && kept_sig * 2 <= max_sig) {
        return None;
    }
    if max_sig >= 6 && final_hunks.len() >= 4 && kept_sig < final_hunks.len() * 3 {
        return None;
    }

    let mut out = String::with_capacity(old_in.len() + new_in.len() + final_hunks.len() * 36);
    let mut o_cursor = 0usize;
    let mut n_cursor = 0usize;

    for (o_s, o_e, n_s, n_e) in final_hunks {
        if o_cursor < o_s {
            let byte_s = old_toks[o_cursor].start;
            let byte_e = old_toks[o_s - 1].end;
            out.push_str(&old_in[byte_s..byte_e]);
        }
        if o_s < o_e {
            let byte_s = old_toks[o_s].start;
            let byte_e = old_toks[o_e - 1].end;
            out.push_str("<del data-diff>");
            out.push_str(&old_in[byte_s..byte_e]);
            out.push_str("</del>");
        }
        if n_s < n_e {
            let byte_s = new_toks[n_s].start;
            let byte_e = new_toks[n_e - 1].end;
            out.push_str("<ins data-diff>");
            out.push_str(&new_in[byte_s..byte_e]);
            out.push_str("</ins>");
        }
        o_cursor = o_e;
        n_cursor = n_e;
    }

    if o_cursor < old_toks.len() && n_cursor < new_toks.len() {
        let byte_s = old_toks[o_cursor].start;
        out.push_str(&old_in[byte_s..]);
    }

    Some(out)
}

fn split_single_mathjax_with_hws(s: &str) -> Option<(&str, &str, &str, &str, &str)> {
    let bytes = s.as_bytes();
    let mut lead = 0usize;
    while lead < bytes.len() && (bytes[lead] == b' ' || bytes[lead] == b'\t') {
        lead += 1;
    }
    let mut trail = bytes.len();
    while trail > lead && (bytes[trail - 1] == b' ' || bytes[trail - 1] == b'\t') {
        trail -= 1;
    }
    let core = &s[lead..trail];
    if core.len() < 4 {
        return None;
    }
    let spans = find_mathjax_spans(core);
    if spans.len() != 1 || spans[0] != (0, core.len()) {
        return None;
    }
    Some((
        &s[..lead],
        &core[..2],
        &core[2..core.len() - 2],
        &core[core.len() - 2..],
        &s[trail..],
    ))
}

fn emit_ws_diff(old_ws: &str, new_ws: &str, out: &mut String) {
    if old_ws == new_ws {
        out.push_str(old_ws);
        return;
    }
    if !old_ws.is_empty() {
        out.push_str("<del data-diff>");
        out.push_str(old_ws);
        out.push_str("</del>");
    }
    if !new_ws.is_empty() {
        out.push_str("<ins data-diff>");
        out.push_str(new_ws);
        out.push_str("</ins>");
    }
}

#[derive(Clone, Copy)]
enum TopSegment<'a> {
    Math {
        full: &'a str,
        open: &'a str,
        inner: &'a str,
        close: &'a str,
    },
    Text(&'a str),
}

impl<'a> TopSegment<'a> {
    fn as_str(self) -> &'a str {
        match self {
            TopSegment::Math { full, .. } | TopSegment::Text(full) => full,
        }
    }
}

fn split_plain_words<'a>(s: &'a str, out: &mut Vec<TopSegment<'a>>) {
    let mut cur = 0usize;
    let mut in_ws = None;
    for (idx, ch) in s.char_indices() {
        let ws = ch.is_whitespace();
        match in_ws {
            None => in_ws = Some(ws),
            Some(prev) if prev != ws => {
                out.push(TopSegment::Text(&s[cur..idx]));
                cur = idx;
                in_ws = Some(ws);
            }
            _ => {}
        }
    }
    if cur < s.len() {
        out.push(TopSegment::Text(&s[cur..]));
    }
}

fn segment_top_level<'a>(s: &'a str, spans: &[(usize, usize)]) -> Vec<TopSegment<'a>> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    for &(st, en) in spans {
        if st > cursor {
            split_plain_words(&s[cursor..st], &mut out);
        }
        let full = &s[st..en];
        out.push(TopSegment::Math {
            full,
            open: &full[..2],
            inner: &full[2..full.len() - 2],
            close: &full[full.len() - 2..],
        });
        cursor = en;
    }
    if cursor < s.len() {
        split_plain_words(&s[cursor..], &mut out);
    }
    out
}

/// Attempts best-effort intra-formula diffing when `old_mid` and `new_mid` contain matching
/// closed MathJax formulas. Returns `false` if no clean alignment applies so the caller can
/// emit a normal block replacement.
fn try_mathjax_intra_diff(old_mid: &str, new_mid: &str, out: &mut String) -> bool {
    if let (
        Some((o_lead, o_open, o_in, o_close, o_trail)),
        Some((n_lead, n_open, n_in, _n_close, n_trail)),
    ) = (
        split_single_mathjax_with_hws(old_mid),
        split_single_mathjax_with_hws(new_mid),
    ) {
        if o_open != n_open {
            return false;
        }
        let Some(inner_diff) = diff_tex_inner(o_in, n_in) else {
            return false;
        };
        emit_ws_diff(o_lead, n_lead, out);
        out.push_str(o_open);
        out.push_str(&inner_diff);
        out.push_str(o_close);
        emit_ws_diff(o_trail, n_trail, out);
        return true;
    }

    let old_spans = find_mathjax_spans(old_mid);
    let new_spans = find_mathjax_spans(new_mid);
    if old_spans.is_empty() || new_spans.is_empty() {
        return false;
    }

    let old_segs = segment_top_level(old_mid, &old_spans);
    let new_segs = segment_top_level(new_mid, &new_spans);
    let m = old_segs.len();
    let n = new_segs.len();
    if m == 0 || n == 0 || m > MAX_TEX_LCS_TOKENS || n > MAX_TEX_LCS_TOKENS {
        return false;
    }

    let mut dp = vec![0u32; (m + 1) * (n + 1)];
    let stride = n + 1;
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            let eq = match (old_segs[i], new_segs[j]) {
                (TopSegment::Math { full: a, .. }, TopSegment::Math { full: b, .. })
                | (TopSegment::Text(a), TopSegment::Text(b)) => a == b,
                _ => false,
            };
            dp[i * stride + j] = if eq {
                dp[(i + 1) * stride + (j + 1)] + 1
            } else {
                dp[(i + 1) * stride + j].max(dp[i * stride + (j + 1)])
            };
        }
    }

    let mut anchors = Vec::new();
    let mut i = 0usize;
    let mut j = 0usize;
    while i < m && j < n {
        let eq = match (old_segs[i], new_segs[j]) {
            (TopSegment::Math { full: a, .. }, TopSegment::Math { full: b, .. })
            | (TopSegment::Text(a), TopSegment::Text(b)) => a == b,
            _ => false,
        };
        if eq {
            anchors.push((i, j));
            i += 1;
            j += 1;
        } else if dp[(i + 1) * stride + j] >= dp[i * stride + (j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }

    // Require at least one aligned anchor or a 1-to-1 intra-formula diff.
    let mut buf = String::with_capacity(old_mid.len() + new_mid.len() + 32);
    let mut improved = !anchors.is_empty();
    let mut oi = 0usize;
    let mut ni = 0usize;

    for (ao, an) in anchors.into_iter().chain(std::iter::once((m, n))) {
        if oi < ao || ni < an {
            let mut emitted_intra = false;
            if ao - oi == 1 && an - ni == 1 {
                if let (
                    TopSegment::Math {
                        open: oo,
                        inner: oi_str,
                        close: oc,
                        ..
                    },
                    TopSegment::Math {
                        open: no,
                        inner: ni_str,
                        ..
                    },
                ) = (old_segs[oi], new_segs[ni])
                {
                    if oo == no {
                        if let Some(inner_diff) = diff_tex_inner(oi_str, ni_str) {
                            buf.push_str(oo);
                            buf.push_str(&inner_diff);
                            buf.push_str(oc);
                            improved = true;
                            emitted_intra = true;
                        }
                    }
                }
            }
            if !emitted_intra {
                if oi < ao {
                    buf.push_str("<del data-diff>");
                    for seg in &old_segs[oi..ao] {
                        buf.push_str(seg.as_str());
                    }
                    buf.push_str("</del>");
                }
                if ni < an {
                    buf.push_str("<ins data-diff>");
                    for seg in &new_segs[ni..an] {
                        buf.push_str(seg.as_str());
                    }
                    buf.push_str("</ins>");
                }
            }
        }
        if ao < m {
            buf.push_str(old_segs[ao].as_str());
        }
        oi = ao + 1;
        ni = an + 1;
    }

    if !improved {
        return false;
    }
    out.push_str(&buf);
    true
}

fn snap_prefix_for_spans(pos: usize, spans: &[(usize, usize)]) -> usize {
    for &(start, end) in spans {
        if pos > start && pos < end {
            return start;
        }
    }
    pos
}

fn snap_suffix_for_spans(pos: usize, spans: &[(usize, usize)]) -> usize {
    for &(start, end) in spans {
        if pos > start && pos < end {
            return end;
        }
    }
    pos
}

// Split two strings into (common_prefix, old_middle, new_middle, common_suffix) on word and atomic MathJax boundaries.
// First finds the grapheme-level prefix/suffix, then snaps outward to word and MathJax boundaries
// so the diff never splits inside a word or a closed MathJax formula.
fn split_diff_regions<'a>(old: &'a str, new: &'a str) -> (&'a str, &'a str, &'a str, &'a str) {
    let old_gr: Vec<&str> = old.graphemes(true).collect();
    let new_gr: Vec<&str> = new.graphemes(true).collect();
    let mut pre = 0usize;
    while pre < old_gr.len() && pre < new_gr.len() && old_gr[pre] == new_gr[pre] {
        pre += 1;
    }
    let mut suf = 0usize;
    while suf < old_gr.len() - pre
        && suf < new_gr.len() - pre
        && old_gr[old_gr.len() - 1 - suf] == new_gr[new_gr.len() - 1 - suf]
    {
        suf += 1;
    }
    let prefix_bytes: usize = old_gr[..pre].iter().map(|g| g.len()).sum();
    let old_suffix_start: usize = old_gr[..old_gr.len() - suf].iter().map(|g| g.len()).sum();
    let new_suffix_start: usize = new_gr[..new_gr.len() - suf].iter().map(|g| g.len()).sum();

    let old_mj = find_mathjax_spans(old);
    let new_mj = find_mathjax_spans(new);

    // Snap prefix boundary backward to nearest word or atomic MathJax boundary.
    let mut snapped_prefix = snap_prefix_to_word_boundary(old, prefix_bytes, &old_mj)
        .min(snap_prefix_to_word_boundary(new, prefix_bytes, &new_mj));
    loop {
        let adjusted = snap_prefix_for_spans(snapped_prefix, &old_mj)
            .min(snap_prefix_for_spans(snapped_prefix, &new_mj));
        if adjusted == snapped_prefix {
            break;
        }
        snapped_prefix = snap_prefix_to_word_boundary(old, adjusted, &old_mj)
            .min(snap_prefix_to_word_boundary(new, adjusted, &new_mj));
    }

    // Snap suffix boundary forward symmetrically so both old and new end cleanly
    // outside words and outside any closed MathJax spans.
    let mut common_suffix_len = old.len() - old_suffix_start;
    debug_assert_eq!(common_suffix_len, new.len() - new_suffix_start);
    loop {
        let old_pos = old.len() - common_suffix_len;
        let new_pos = new.len() - common_suffix_len;
        let next_old = snap_suffix_to_word_boundary(old, old_pos, &old_mj);
        let next_new = snap_suffix_to_word_boundary(new, new_pos, &new_mj);
        let next_common = (old.len() - next_old).min(new.len() - next_new);
        if next_common == common_suffix_len {
            break;
        }
        common_suffix_len = next_common;
    }
    let snapped_old_suffix = (old.len() - common_suffix_len).max(snapped_prefix);
    let snapped_new_suffix = (new.len() - common_suffix_len).max(snapped_prefix);

    let prefix = &old[..snapped_prefix];
    let old_middle = &old[snapped_prefix..snapped_old_suffix];
    let new_middle = &new[snapped_prefix..snapped_new_suffix];
    let suffix = &old[snapped_old_suffix..];
    (prefix, old_middle, new_middle, suffix)
}

/// Snap a byte position backward to the nearest word or MathJax span boundary.
fn snap_prefix_to_word_boundary(s: &str, byte_pos: usize, mj_spans: &[(usize, usize)]) -> usize {
    if byte_pos == 0 {
        return 0;
    }
    let mut clamped = byte_pos.min(s.len());
    while clamped > 0 && !s.is_char_boundary(clamped) {
        clamped -= 1;
    }
    clamped = snap_prefix_for_spans(clamped, mj_spans);

    if clamped == 0
        || mj_spans
            .iter()
            .any(|&(st, en)| clamped == st || clamped == en)
    {
        return clamped;
    }

    let mut word_start = 0usize;
    let mut last_was_ws = true;
    let mut prev_char: Option<(usize, char)> = None;

    for (idx, ch) in s.char_indices() {
        if idx >= clamped {
            break;
        }
        if mj_spans.iter().any(|&(st, en)| idx == st || idx == en) {
            word_start = idx;
            last_was_ws = ch.is_whitespace();
        }
        if ch.is_whitespace() {
            last_was_ws = true;
            word_start = idx + ch.len_utf8();
        } else if last_was_ws {
            last_was_ws = false;
            word_start = idx;
        }
        prev_char = Some((idx, ch));
    }

    if let Some((_idx, ch)) = prev_char {
        if ch.is_whitespace() {
            return clamped;
        }
    }

    word_start
}

/// Snap a byte position forward to the nearest word or MathJax span boundary.
fn snap_suffix_to_word_boundary(s: &str, byte_pos: usize, mj_spans: &[(usize, usize)]) -> usize {
    if byte_pos >= s.len() {
        return s.len();
    }

    let mut clamped = byte_pos;
    while clamped < s.len() && !s.is_char_boundary(clamped) {
        clamped += 1;
    }
    clamped = snap_suffix_for_spans(clamped, mj_spans);

    if clamped >= s.len()
        || mj_spans
            .iter()
            .any(|&(st, en)| clamped == st || clamped == en)
    {
        return clamped.min(s.len());
    }

    let mut end = s.len();
    for (idx, ch) in s.char_indices() {
        if idx < clamped {
            continue;
        }
        if ch.is_whitespace() || mj_spans.iter().any(|&(st, _)| idx == st) {
            end = idx;
            break;
        }
    }

    end
}

// Greedy diff for large sibling lists: match by first occurrence of unique signatures.
fn greedy_children_diff(
    old_children: &[NodeRef],
    new_children: &[NodeRef],
    old_sig: &[String],
    new_sig: &[String],
) -> Vec<NodeChange> {
    let mut res = Vec::new();
    let mut old_map: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, s) in old_sig.iter().enumerate() {
        old_map.entry(s).or_default().push(i);
    }
    let mut consumed_old: HashSet<usize> = HashSet::new();
    let mut prev_old_idx: Option<usize> = None;
    for (j, s) in new_sig.iter().enumerate() {
        let candidate = old_map
            .get(s.as_str())
            .and_then(|v| v.iter().find(|idx| !consumed_old.contains(idx)).cloned());
        if let Some(i) = candidate {
            // emit removals for skipped old nodes before this match
            let start = prev_old_idx.map_or(0, |prev| prev + 1);
            for (k, old_child) in old_children.iter().enumerate().take(i).skip(start) {
                if !consumed_old.contains(&k) {
                    res.push(NodeChange {
                        kind: ChangeKind::Removed,
                        old: Some(old_child.clone()),
                        new: None,
                        children: vec![],
                    });
                    consumed_old.insert(k);
                }
            }
            res.push(diff_node(&old_children[i], &new_children[j]));
            consumed_old.insert(i);
            prev_old_idx = Some(i);
        } else {
            res.push(NodeChange {
                kind: ChangeKind::Added,
                old: None,
                new: Some(new_children[j].clone()),
                children: vec![],
            });
        }
    }
    // Remaining old nodes
    for (i, node) in old_children.iter().enumerate() {
        if !consumed_old.contains(&i) {
            res.push(NodeChange {
                kind: ChangeKind::Removed,
                old: Some(node.clone()),
                new: None,
                children: vec![],
            });
        }
    }
    res
}

// Post-process output to collapse adjacent identical ins/del wrappers and remove empty tags.
fn finalize_output(s: String) -> String {
    // Iteratively collapse empty and adjacent same-type diff tags until stable.
    let mut cur = s;
    loop {
        let mut out = String::with_capacity(cur.len());
        let mut i = 0;
        let bytes = cur.as_bytes();
        while i < bytes.len() {
            // The patterns are pure ASCII so byte-slice comparison is safe; i always at char boundary.
            if bytes.len() - i >= 21 {
                let slice = &bytes[i..i + 21];
                if slice == b"<del data-diff></del>"
                    || slice == b"<ins data-diff></ins>"
                    || slice == b"</del><del data-diff>"
                    || slice == b"</ins><ins data-diff>"
                {
                    i += 21;
                    continue;
                }
            }
            // Copy one UTF-8 char
            let b = bytes[i];
            let char_len = if b < 0x80 {
                1
            } else if b & 0b1110_0000 == 0b1100_0000 {
                2
            } else if b & 0b1111_0000 == 0b1110_0000 {
                3
            } else {
                4
            };
            let end = i + char_len;
            // Safety: end <= bytes.len() due to UTF-8 validity of original string.
            out.push_str(&cur[i..end]);
            i = end;
        }
        if out.len() == cur.len() {
            return out; // no more changes
        }
        cur = out;
    }
}

fn is_inline_equiv(a: &ElementData, b: &ElementData) -> bool {
    use std::borrow::Cow;
    let a_tag = a.name.local.to_string().to_ascii_lowercase();
    let b_tag = b.name.local.to_string().to_ascii_lowercase();
    if a_tag == b_tag {
        return false;
    } // same handled earlier
    fn canonical(t: &str) -> Cow<'static, str> {
        match t {
            "b" | "strong" => Cow::Borrowed("strong"),
            "i" | "em" => Cow::Borrowed("em"),
            other => Cow::Owned(other.to_string()),
        }
    }
    canonical(&a_tag) == canonical(&b_tag)
        && a.attributes.borrow().map.is_empty()
        && b.attributes.borrow().map.is_empty()
}

fn parent_tag(node: &NodeRef) -> Option<String> {
    node.parent().and_then(|p| {
        p.as_element()
            .map(|e| e.name.local.to_string().to_ascii_lowercase())
    })
}

fn should_ignore_ws_diff(old: &NodeRef, new: &NodeRef, o_txt: &str, n_txt: &str) -> bool {
    // Normalize NBSP (U+00A0) to regular space for comparison
    let o_norm = o_txt.replace('\u{00a0}', " ");
    let n_norm = n_txt.replace('\u{00a0}', " ");
    if o_norm.trim() == n_norm.trim() {
        if let Some(tag) = parent_tag(old) {
            if tag == "pre" || tag == "code" || tag == "textarea" {
                return false;
            }
        }
        if let Some(tag) = parent_tag(new) {
            if tag == "pre" || tag == "code" || tag == "textarea" {
                return false;
            }
        }
        // Only suppress if both versions still have boundary whitespace (so pure trimming shows as diff)
        let old_boundary = o_norm.starts_with(' ') || o_norm.ends_with(' ');
        let new_boundary = n_norm.starts_with(' ') || n_norm.ends_with(' ');
        if old_boundary && new_boundary {
            return true;
        }
        // Also suppress if the only difference is nbsp vs space
        if o_norm == n_norm {
            return true;
        }
    }
    false
}

// Safety belt: ensure simple alphanumeric tokens (pattern W\d+) removed from old but absent in new remain represented.
fn preserve_missing_simple_tokens(old: &str, new: &str, out: &mut String) {
    // Collect tokens from old
    let mut i = 0;
    let bytes = old.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'W' {
            let start = i;
            i += 1;
            let mut had_digit = false;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                had_digit = true;
                i += 1;
            }
            if had_digit {
                let tok = &old[start..i];
                if !new.contains(tok) && !out.contains(tok) {
                    out.push_str("<del data-diff>");
                    out.push_str(tok);
                    out.push_str("</del>");
                }
            }
        } else {
            i += 1;
        }
    }
}
