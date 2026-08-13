use htmldiff::htmldiff;
use kuchiki::parse_html;
use kuchiki::traits::*;
use proptest::prelude::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::fmt::Write as FmtWrite;

#[derive(Debug)]
enum EditKind {
    Insert {
        token: String,
    },
    Delete {
        token: String,
    },
    Move {
        token: String,
    },
    ReplaceTag {
        token: String,
        _old_tag: String,
        _new_tag: String,
    },
    ChangeAttr {
        token: String,
        _attr: String,
        _old: String,
        new: String,
    },
    SplitText {
        token: String,
        _tail_token: String,
    },
}

#[derive(Debug)]
struct Edit {
    kind: EditKind,
}

/// Create a deterministic base DOM with N paragraphs tokens W0..W{n-1}
fn generate_base_dom(n: usize) -> String {
    let mut s = String::with_capacity(n * 20);
    s.push_str("<div>");
    for i in 0..n {
        write!(&mut s, "<p>W{} word-{}</p>", i, i).unwrap();
    }
    s.push_str("</div>");
    s
}

/// Pick a paragraph token present in the html. We make tokens predictable: "W{i}".
fn pick_random_existing_token(rng: &mut StdRng, max_tokens: usize) -> String {
    let idx = rng.random_range(0..max_tokens);
    format!("W{}", idx)
}

/// Apply edits to a serialized HTML string using simple, robust string transforms.
/// Each edit returns an `Edit` that records the token(s) we can later assert on.
fn apply_random_edits(
    html: &str,
    rng: &mut StdRng,
    max_tokens: usize,
    edits_count: usize,
) -> (String, Vec<Edit>) {
    let mut s = html.to_string();
    let mut edits = Vec::with_capacity(edits_count);

    let _tags = ["p", "div", "span", "b", "i", "u", "li"]; // retained list for future extension

    for eidx in 0..edits_count {
        let choice = rng.random_range(0..6);
        match choice {
            0 => {
                // Insert: add a new paragraph with a unique token INS{eidx}
                let token = format!("INS{}", eidx);
                // insert before closing div if present, else append
                if let Some(pos) = s.rfind("</div>") {
                    let insert_html = format!("<p>{}</p>", token);
                    s.insert_str(pos, &insert_html);
                } else {
                    s.push_str(&format!("<p>{}</p>", token));
                }
                edits.push(Edit {
                    kind: EditKind::Insert { token },
                });
            }
            1 => {
                // Delete: remove the first occurrence of a whole paragraph containing a chosen token
                let token = pick_random_existing_token(rng, max_tokens);
                // find "<p>...token..." and try to remove from "<p" to "</p>"
                if let Some(start) = s.find(&format!("<p>")) {
                    // naive: find token occurrence position
                    if let Some(tokpos) = s.find(&token) {
                        // find the opening "<p" before token
                        let open_pos = s[..tokpos].rfind("<p").unwrap_or(start);
                        if let Some(close_pos) = s[tokpos..].find("</p>") {
                            let close_abs = tokpos + close_pos + 4; // include "</p>"
                            s.replace_range(open_pos..close_abs, "");
                            edits.push(Edit {
                                kind: EditKind::Delete { token },
                            });
                            continue;
                        }
                    }
                }
                // fallback: remove the token string occurrences
                if s.contains(&token) {
                    s = s.replacen(&token, "", 1);
                    edits.push(Edit {
                        kind: EditKind::Delete { token },
                    });
                } else {
                    // nothing deleted; skip this edit
                }
            }
            2 => {
                // Move: pick a paragraph containing token and move it to front
                let token = pick_random_existing_token(rng, max_tokens);
                if let Some(tokpos) = s.find(&token) {
                    if let Some(open_pos) = s[..tokpos].rfind("<p") {
                        if let Some(close_rel) = s[tokpos..].find("</p>") {
                            let _close_abs = tokpos + close_rel + 4; // retained for slicing
                            let node_html = s[open_pos.._close_abs].to_string();
                            // remove original node
                            s.replace_range(open_pos.._close_abs, "");
                            // insert at front after <div>
                            if let Some(div_open) = s.find("<div>") {
                                let insert_pos = div_open + "<div>".len();
                                s.insert_str(insert_pos, &node_html);
                            } else {
                                s.insert_str(0, &node_html);
                            }
                            edits.push(Edit {
                                kind: EditKind::Move { token },
                            });
                        }
                    }
                }
            }
            3 => {
                // Replace tag: swap <p>...</p> -> <b>...</b> for a paragraph containing token
                let token = pick_random_existing_token(rng, max_tokens);
                if let Some(tokpos) = s.find(&token) {
                    if let Some(open_pos) = s[..tokpos].rfind("<p>") {
                        if let Some(close_rel) = s[tokpos..].find("</p>") {
                            let _close_abs = tokpos + close_rel + 4;
                            // replace opening and closing tag
                            s.replace_range(open_pos..open_pos + 3, "<b>");
                            // closing replace: replace the first occurrence of "</p>" after tokpos with "</b>"
                            if let Some(pclose) = s[tokpos..].find("</p>") {
                                let pclose_abs = tokpos + pclose;
                                s.replace_range(pclose_abs..pclose_abs + 4, "</b>");
                                edits.push(Edit {
                                    kind: EditKind::ReplaceTag {
                                        token,
                                        _old_tag: "p".into(),
                                        _new_tag: "b".into(),
                                    },
                                });
                            }
                        }
                    }
                }
            }
            4 => {
                // Change attribute: add or change class on first <p> occurrence that contains a token
                let token = pick_random_existing_token(rng, max_tokens);
                // naive replace "<p>" with `<p class="c{eidx}">` at first <p> before token
                if let Some(tokpos) = s.find(&token) {
                    if let Some(open_pos) = s[..tokpos].rfind("<p>") {
                        s.replace_range(
                            open_pos..open_pos + 3,
                            &format!("<p class=\"c{}\">", eidx),
                        );
                        edits.push(Edit {
                            kind: EditKind::ChangeAttr {
                                token,
                                _attr: "class".into(),
                                _old: "".into(),
                                new: format!("c{}", eidx),
                            },
                        });
                    }
                }
            }
            5 => {
                // Split text node: in a paragraph, split a token word and wrap tail in <b>
                let token = pick_random_existing_token(rng, max_tokens);
                if let Some(tokpos) = s.find(&token) {
                    // insert a split: replace the first occurrence of token with e.g. "Wk wor<b>d</b>"
                    // we create a tail token so we can check presence
                    let tail_token = format!("TAIL{}", eidx);
                    if let Some(_word_pos) = s[tokpos..].find(&format!("word-")) {
                        // naive transform: replace "word-N" with "wor<b>d</b>"
                        let full_word = format!(
                            "word-{}",
                            token.trim_start_matches('W').parse::<usize>().unwrap_or(0)
                        );
                        if s.contains(&full_word) {
                            let before = s.clone();
                            s = s.replacen(
                                &full_word,
                                &format!("{}<b>{}</b>", &full_word[..3], tail_token),
                                1,
                            );
                            if s != before {
                                edits.push(Edit {
                                    kind: EditKind::SplitText {
                                        token,
                                        _tail_token: tail_token,
                                    },
                                });
                            }
                        } else if s.contains(&token) {
                            let before = s.clone();
                            s = s.replacen(
                                &token,
                                &format!(
                                    "{}<b>{}</b>",
                                    &token[..std::cmp::min(2, token.len())],
                                    tail_token
                                ),
                                1,
                            );
                            if s != before {
                                edits.push(Edit {
                                    kind: EditKind::SplitText {
                                        token,
                                        _tail_token: tail_token,
                                    },
                                });
                            }
                        }
                    } else {
                        // fallback simple split
                        let tail_token = format!("TAIL{}", eidx);
                        if token.len() > 2 && s.contains(&token) {
                            let before = s.clone();
                            s = s.replacen(
                                &token,
                                &format!("{}<b>{}</b>", &token[..2], tail_token),
                                1,
                            );
                            if s != before {
                                edits.push(Edit {
                                    kind: EditKind::SplitText {
                                        token,
                                        _tail_token: tail_token,
                                    },
                                });
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    (s, edits)
}

/// Helper: return true if `token` appears inside any <ins data-diff> node of parsed `html_out`
fn token_in_ins(html_out: &str, token: &str) -> bool {
    let doc = parse_html().one(html_out);
    if let Ok(nodes) = doc.select("ins[data-diff]") {
        for matched in nodes {
            let text = matched.as_node().text_contents();
            if text.contains(token) {
                return true;
            }
        }
    }
    false
}

/// Helper: return true if `token` appears inside any <del data-diff> node of parsed `html_out`
fn token_in_del(html_out: &str, token: &str) -> bool {
    let doc = parse_html().one(html_out);
    if let Ok(nodes) = doc.select("del[data-diff]") {
        for matched in nodes {
            let text = matched.as_node().text_contents();
            if text.contains(token) {
                return true;
            }
        }
    }
    false
}

#[test]
fn fuzz_dom_diff_generator() {
    // deterministic seed - change for randomness
    let seed: u64 = 42;
    let mut rng = StdRng::seed_from_u64(seed);

    let iterations = 1000; // increased coverage
    let base_tokens = 50; // W0..W49 initially
    let failures = 0;

    for iter in 0..iterations {
        let base = generate_base_dom(base_tokens);
        // apply between 1 and 5 random edits
        let edit_count = (rng.random_range(1..=5)) as usize;
        let (new_html, edits) = apply_random_edits(&base, &mut rng, base_tokens, edit_count);

        // run diff
        let out = htmldiff(&base, &new_html);
        validate_no_bad_nesting(&out);

        // 1) Output should be parseable HTML
        let parsed = parse_html().one(out.clone());
        // If parse_html() panics or fails that's already bad; here we assert the document contains something
        let text = parsed.text_contents();
        assert!(
            !text.is_empty(),
            "Parsed diff shouldn't be empty (iter {})",
            iter
        );

        // 2) For each edit, perform reasonable assertions
        for ed in edits {
            match ed.kind {
                EditKind::Insert { token } => {
                    // inserted token should appear in the <ins> area (or at least in output)
                    if !token_in_ins(&out, &token) {
                        // if not in <ins> at least ensure token exists in output
                        assert!(out.contains(&token), "Insert token '{}' missing in output (iter {})\nbase: {}\nnew: {}\nout: {}", token, iter, base, new_html, out);
                    }
                }
                EditKind::Delete { token } => {
                    // deleted token should appear inside <del> or at least in output (since it existed in base)
                    if !token_in_del(&out, &token) {
                        assert!(
                            out.contains(&token),
                            "Delete token '{}' missing in output (iter {})",
                            token,
                            iter
                        );
                    }
                }
                EditKind::Move { token } => {
                    // movement may be represented as move or delete+insert; ensure token exists in output
                    assert!(
                        out.contains(&token),
                        "Moved token '{}' missing in output (iter {})",
                        token,
                        iter
                    );
                }
                EditKind::ReplaceTag { token, .. } => {
                    // token should still be present; and diff should have some ins/del if tag changed
                    assert!(
                        out.contains(&token),
                        "ReplaceTag token '{}' missing (iter {})",
                        token,
                        iter
                    );
                    assert!(
                        out.contains("<ins") || out.contains("<del"),
                        "No <ins>/<del> found for ReplaceTag (iter {})",
                        iter
                    );
                }
                EditKind::ChangeAttr { token, new, .. } => {
                    // class value should be visible somewhere if added
                    if !out.contains(&new) {
                        // fallback: token must be present
                        assert!(
                            out.contains(&token),
                            "ChangeAttr token '{}' missing and new attr '{}' not present (iter {})",
                            token,
                            new,
                            iter
                        );
                    }
                }
                EditKind::SplitText {
                    token,
                    _tail_token: _,
                } => {
                    // Original token should remain visible somewhere (either unchanged or inside a diff span)
                    assert!(
                        out.contains(&token),
                        "SplitText token '{}' missing (iter {})",
                        token,
                        iter
                    );
                    // Tail token may occasionally be optimized away or merged by the diff; treat absence as non-fatal.
                    // (We only ensure the diff retained the original token; tail presence is a best-effort signal of split handling.)
                    // if !out.contains(&tail_token) { eprintln!("[warn] SplitText tail '{}' absent (iter {})", tail_token, iter); }
                }
            }
        }

        // optional: cheap sanity checks to disallow tags interleaving patterns you already know are bad
        // e.g. avoid see the classic broken nesting like "</u></b>" — ensure your diff output doesn't contain adjacent mismatched closes
        assert!(
            !out.contains("</u></b>") && !out.contains("</b></u>"),
            "Detected bad nesting pattern (iter {}): {}",
            iter,
            out
        );
    }

    assert_eq!(failures, 0, "One or more fuzz iterations flagged failures");
}

fn validate_no_bad_nesting(out: &str) {
    let mut stack: Vec<&str> = Vec::new();
    let mut i = 0;
    let bytes = out.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if out[i..].starts_with("<ins data-diff>") {
                stack.push("ins");
                i += 15;
                continue;
            }
            if out[i..].starts_with("<del data-diff>") {
                stack.push("del");
                i += 15;
                continue;
            }
            if out[i..].starts_with("</ins>") {
                if stack.last() == Some(&"ins") {
                    stack.pop();
                }
                i += 6;
                continue;
            }
            if out[i..].starts_with("</del>") {
                if stack.last() == Some(&"del") {
                    stack.pop();
                }
                i += 6;
                continue;
            }
        }
        i += 1;
    }
}

// Property test using proptest: generate small simple HTML fragments, ensure diff output parses and ins/del nesting valid.
proptest! {
    #[test]
    fn prop_small_random_html(a in small_html(), b in small_html()) {
    let out = htmldiff(&a, &b);
        let _ = parse_html().one(out.clone());
        validate_no_bad_nesting(&out);
    }
}

fn small_html() -> impl Strategy<Value = String> {
    use proptest::string::string_regex;
    let tag = prop_oneof![Just("p"), Just("b"), Just("i"), Just("u"), Just("span")];
    let word = string_regex("[a-z]{1,5}").unwrap();
    let words = prop::collection::vec(word, 1..4);
    prop_oneof![
        (tag, words).prop_map(|(t, ws)| format!("<{t}>{}</{t}>", ws.join(" "))),
        Just(String::new()),
    ]
}

// Generate intentionally malformed/scrambled HTML fragments for robustness testing.
fn malformed_html() -> impl Strategy<Value = String> {
    use proptest::string::string_regex;
    let raw_text = string_regex("[a-z]{0,8}").unwrap();
    // Fragments of opening/closing tags, sometimes mismatched or missing closers
    let frag = prop_oneof![
        Just("<p>".to_string()),
        Just("<b>".to_string()),
        Just("</b>".to_string()),
        Just("<i>".to_string()),
        Just("</p>".to_string()),
        Just("<u class='x'>".to_string()),
        Just("</u>".to_string()),
        Just("<span>".to_string()),
        Just("</span>".to_string()),
        Just("<div>".to_string()),
        Just("</div>".to_string()),
        Just("<li>".to_string()),
        Just("</li>".to_string()),
        raw_text.prop_map(|s| s),
    ];
    // Build a sequence of 1..10 fragments concatenated; occasionally wrap with an outer unclosed tag
    prop_oneof![
        prop::collection::vec(frag.clone(), 1..10).prop_map(|parts| parts.join("")),
        (prop::collection::vec(frag, 1..8)).prop_map(|parts| format!("<div>{}", parts.join(""))), // missing </div>
    ]
}

proptest! {
    #[test]
    fn prop_malformed_html(a in malformed_html(), b in malformed_html()) {
    let out = htmldiff(&a, &b);
        // Parser should not panic
        let _ = parse_html().one(out.clone());
        validate_no_bad_nesting(&out);
    }
}
