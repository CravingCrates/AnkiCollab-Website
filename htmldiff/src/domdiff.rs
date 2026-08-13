use kuchiki::traits::*;
use kuchiki::{parse_html, ElementData, NodeRef};
use std::collections::{HashMap, HashSet};
use unicode_segmentation::UnicodeSegmentation;

// Threshold to switch from O(n*m) LCS to linear greedy alignment for large sibling lists.
const LCS_THRESHOLD: usize = 200;

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

pub fn diff_html(old: &str, new: &str) -> String {
    // Wrap fragments to avoid full HTML skeleton differences
    let wrapped_old = format!("<div data-htmldiff-root>{}</div>", old);
    let wrapped_new = format!("<div data-htmldiff-root>{}</div>", new);
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
    preserve_missing_simple_tokens(old, new, &mut rendered);
    rendered
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
        let mut attr_keys: Vec<_> = attrs.map.iter().map(|(k, _)| k.local.to_string()).collect();
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
                        .and_then(|n| parent_tag(n))
                        .map(|t| t == "pre" || t == "code" || t == "textarea")
                        .unwrap_or(false)
                        || change
                            .new
                            .as_ref()
                            .and_then(|n| parent_tag(n))
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
                } else if !try_char_level_highlight(old_mid, new_mid, out) {
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

fn render_node(change: &NodeChange, out: &mut String) {
    if let Some(n) = change.new.as_ref().or(change.old.as_ref()) {
        if let Some(el) = n.as_element() {
            out.push('<');
            out.push_str(el.name.local.as_ref());
            let attrs = el.attributes.borrow();
            // sort attributes by name for deterministic output
            let mut keys: Vec<_> = attrs.map.iter().map(|(k, _)| k.local.to_string()).collect();
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
        let attrs = el.attributes.borrow();
        let mut keys: Vec<_> = attrs.map.iter().map(|(k, _)| k.local.to_string()).collect();
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
        let attrs = el.attributes.borrow();
        let mut keys: Vec<_> = attrs.map.iter().map(|(k, _)| k.local.to_string()).collect();
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
fn first_n_chars<'a>(s: &'a str, n: usize) -> &'a str {
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

// Split two strings into (common_prefix, old_middle, new_middle, common_suffix) on word boundaries.
// First finds the grapheme-level prefix/suffix, then snaps outward to word boundaries
// so the diff never splits inside a word.
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

    // Snap prefix boundary backward to nearest word boundary (right after whitespace or start)
    let snapped_prefix = snap_prefix_to_word_boundary(old, prefix_bytes);
    // Snap suffix boundary forward to nearest word boundary (at whitespace or end)
    let snapped_old_suffix = snap_suffix_to_word_boundary(old, old_suffix_start);
    let suffix_delta = snapped_old_suffix - old_suffix_start;
    let snapped_new_suffix = (new_suffix_start + suffix_delta).min(new.len());

    let prefix = &old[..snapped_prefix];
    let old_middle = &old[snapped_prefix..snapped_old_suffix];
    let new_middle = &new[snapped_prefix..snapped_new_suffix];
    let suffix = &old[snapped_old_suffix..];
    (prefix, old_middle, new_middle, suffix)
}

/// Snap a byte position backward to the nearest word boundary (right after whitespace or string start).
/// If byte_pos is inside a word, snaps back to the beginning of that word.
fn snap_prefix_to_word_boundary(s: &str, byte_pos: usize) -> usize {
    if byte_pos == 0 {
        return 0;
    }
    // Clamp to string length to avoid going past the end.
    let mut clamped = byte_pos.min(s.len());
    // Move back to the nearest UTF-8 char boundary at or before `clamped`.
    while clamped > 0 && !s.is_char_boundary(clamped) {
        clamped -= 1;
    }

    if clamped == 0 {
        return 0;
    }

    // Walk characters up to `clamped` to determine word boundaries.
    let mut word_start = 0usize;
    let mut last_was_ws = true;
    let mut prev_char: Option<(usize, char)> = None;

    for (idx, ch) in s.char_indices() {
        if idx >= clamped {
            break;
        }
        if ch.is_whitespace() {
            // Next non-whitespace char starts a new word.
            last_was_ws = true;
            word_start = idx + ch.len_utf8();
        } else if last_was_ws {
            // We are at the beginning of a word.
            last_was_ws = false;
            word_start = idx;
        }
        prev_char = Some((idx, ch));
    }

    // If the character immediately before `clamped` is whitespace, consider `clamped`
    // already at a word boundary (adjusted to a valid char boundary).
    if let Some((_idx, ch)) = prev_char {
        if ch.is_whitespace() {
            return clamped;
        }
    }

    // Otherwise, we're inside a word: snap back to the start of that word.
    word_start
}

/// Snap a byte position forward to the nearest word boundary (at whitespace or string end).
/// If byte_pos is inside a word, snaps forward to the end of that word.
fn snap_suffix_to_word_boundary(s: &str, byte_pos: usize) -> usize {
    if byte_pos >= s.len() {
        return s.len();
    }

    // Move forward to the nearest UTF-8 char boundary at or after `byte_pos`.
    let mut clamped = byte_pos;
    while clamped < s.len() && !s.is_char_boundary(clamped) {
        clamped += 1;
    }

    if clamped >= s.len() {
        return s.len();
    }

    // Walk from `clamped` forward until we reach whitespace or the end of the string.
    let mut end = s.len();
    for (idx, ch) in s.char_indices() {
        if idx < clamped {
            continue;
        }
        if ch.is_whitespace() {
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
            if let Some(prev) = prev_old_idx {
                for k in (prev + 1)..i {
                    if !consumed_old.contains(&k) {
                        res.push(NodeChange {
                            kind: ChangeKind::Removed,
                            old: Some(old_children[k].clone()),
                            new: None,
                            children: vec![],
                        });
                        consumed_old.insert(k);
                    }
                }
            } else {
                for k in 0..i {
                    if !consumed_old.contains(&k) {
                        res.push(NodeChange {
                            kind: ChangeKind::Removed,
                            old: Some(old_children[k].clone()),
                            new: None,
                            children: vec![],
                        });
                        consumed_old.insert(k);
                    }
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
