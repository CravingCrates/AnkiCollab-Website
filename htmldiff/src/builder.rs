use crate::html;
use crate::wu::{diff, Edit};

pub fn build_htmldiff<'a, F>(a: &'a str, b: &'a str, mut callback: F)
where
    F: FnMut(&'a str) -> (),
{
    let old_words = html::split(a);
    let new_words = html::split(b);
    let ses = diff(&old_words, &new_words);
    let mut i = 0usize;
    while i < ses.len() {
        // Detect a simple style/tag replacement pattern:
        // Delete(<tagA>), Add(<tagB>), Common(...)*, Delete(</tagA>), Add(</tagB>)
        // with only Common edits in between. If found, emit
        // <del><tagA>...content...</tagA></del><ins><tagB>...content...</tagB></ins>
        let replacement_emitted = if i + 4 < ses.len() {
            if let (Edit::Delete { old: old_open_idx }, Edit::Add { new: new_open_idx }) =
                (&ses[i], &ses[i + 1])
            {
                let old_open = old_words[*old_open_idx];
                let new_open = new_words[*new_open_idx];
                if is_open_tag(old_open) && is_open_tag(new_open) {
                    // collect following commons
                    let mut commons: Vec<(usize, usize)> = Vec::new();
                    let mut j = i + 2;
                    while j < ses.len() {
                        match &ses[j] {
                            Edit::Common { old, new } => {
                                commons.push((*old, *new));
                                j += 1;
                            }
                            _ => break,
                        }
                    }
                    // need Delete(old_close), Add(new_close)
                    if j + 1 < ses.len() {
                        if let (
                            Edit::Delete { old: old_close_idx },
                            Edit::Add { new: new_close_idx },
                        ) = (&ses[j], &ses[j + 1])
                        {
                            let old_close = old_words[*old_close_idx];
                            let new_close = new_words[*new_close_idx];
                            if is_matching_closing_tag(old_open, old_close)
                                && is_matching_closing_tag(new_open, new_close)
                            {
                                // ensure no structural edits inside (only commons)
                                // Emit replacement block
                                callback("<del data-diff>");
                                callback(old_open);
                                for (o, _n) in &commons {
                                    callback(old_words[*o]);
                                }
                                callback(old_close);
                                callback("</del><ins data-diff>");
                                callback(new_open);
                                for (_o, n) in &commons {
                                    callback(new_words[*n]);
                                }
                                callback(new_close);
                                callback("</ins>");
                                i = j + 2; // skip consumed edits
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        if replacement_emitted {
            continue;
        }

        match &ses[i] {
            Edit::Common { old, new: _ } => {
                callback(old_words[*old]);
            }
            Edit::Add { new } => {
                let word = new_words[*new];
                if is_tag(word) && !is_img_tag(word) {
                    callback(word);
                } else {
                    callback("<ins data-diff>");
                    callback(word);
                    callback("</ins>");
                }
            }
            Edit::Delete { old } => {
                let word = old_words[*old];
                if is_tag(word) && !is_img_tag(word) {
                    callback(word);
                } else {
                    callback("<del data-diff>");
                    callback(word);
                    callback("</del>");
                }
            }
        }
        i += 1;
    }
}

fn is_img_tag(s: &str) -> bool {
    s.starts_with("<img")
}

fn is_tag(s: &str) -> bool {
    s.starts_with("<")
}

fn is_open_tag(s: &str) -> bool {
    is_tag(s) && !s.starts_with("</") && !s.ends_with("/>")
}

fn is_matching_closing_tag(open: &str, close: &str) -> bool {
    if !open.starts_with('<') || !close.starts_with("</") {
        return false;
    }
    let name_open = extract_tag_name(open);
    let name_close = extract_tag_name(close);
    match (name_open, name_close) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

fn extract_tag_name(tag: &str) -> Option<&str> {
    // assumes called with either <tag ...> or </tag>
    if tag.len() < 3 {
        return None;
    }
    let bytes = tag.as_bytes();
    let (start, mut i) = if bytes[1] == b'/' {
        (2usize, 2usize)
    } else {
        (1usize, 1usize)
    };
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'>' || c.is_ascii_whitespace() || c == b'/' {
            break;
        }
        i += 1;
    }
    if i > start {
        Some(&tag[start..i])
    } else {
        None
    }
}
