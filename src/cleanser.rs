use ammonia::Builder;
use regex::Regex;
use std::collections::HashSet;

static ALLOWED_CSS_PROPERTIES: std::sync::LazyLock<HashSet<&'static str>> =
    std::sync::LazyLock::new(|| {
        [
            "color",
            "background-color",
            "font-size",
            "font-family",
            "font-weight",
            "text-align",
            "text-decoration",
            "line-height",
            "margin",
            "padding",
            "border-width",
            "border-style",
            "border-color",
            "writing-mode",
            "fill",
            "stroke",
            "stroke-width",
            "opacity",
            "display",
            // Added for Yomitan
            "list-style-type",
            "list-style",
            "vertical-align",
            // Layout of images, blocks and tables
            "width",
            "max-width",
            "height",
            "border-bottom",
            "border-collapse",
            "border-radius",
            "box-shadow",
            "cursor",
        ]
        .iter()
        .copied()
        .collect()
    });

static ALLOWED_COLOR_NAMES: std::sync::LazyLock<HashSet<&'static str>> =
    std::sync::LazyLock::new(|| {
        [
            "aqua",
            "black",
            "blue",
            "fuchsia",
            "gray",
            "green",
            "lime",
            "maroon",
            "navy",
            "olive",
            "purple",
            "red",
            "silver",
            "teal",
            "white",
            "yellow",
            "currentcolor",
            "transparent",
            "rebeccapurple",
            // Additional safe keywords for SVG
            "none",
        ]
        .iter()
        .copied()
        .collect()
    });

static ALLOWED_WRITING_MODE_VALUES: std::sync::LazyLock<HashSet<&'static str>> =
    std::sync::LazyLock::new(|| {
        [
            "horizontal-tb",
            "vertical-rl",
            "vertical-lr",
            "sideways-rl",
            "sideways-lr",
            "inherit",
            "initial",
            "unset",
            "revert",
            "revert-layer",
        ]
        .iter()
        .copied()
        .collect()
    });

static ALLOWED_CSS_FUNCTIONS: std::sync::LazyLock<HashSet<&'static str>> =
    std::sync::LazyLock::new(|| {
        ["rgb", "rgba", "hsl", "hsla", "calc", "min", "max", "clamp"]
            .iter()
            .copied()
            .collect()
    });

static ALLOWED_CURSOR_VALUES: std::sync::LazyLock<HashSet<&'static str>> =
    std::sync::LazyLock::new(|| {
        [
            "auto",
            "default",
            "pointer",
            "text",
            "help",
            "wait",
            "crosshair",
            "move",
            "not-allowed",
            "grab",
            "grabbing",
            "zoom-in",
            "zoom-out",
            "none",
            "inherit",
            "initial",
            "unset",
            "revert",
            "revert-layer",
        ]
        .iter()
        .copied()
        .collect()
    });

/// Allowed onclick patterns used by the Anki addon Kanji Popup Dictionary with mobile support
static ALLOWED_ONCLICK_REGEXES: std::sync::LazyLock<Vec<Regex>> = std::sync::LazyLock::new(|| {
    vec![
        // showKanjiPopup('X') or showKanjiPopup("X")
        Regex::new(r#"^showKanjiPopup\(\s*['\"][^'\"]+['\"]\s*\)$"#).unwrap(),
        // showLargePopup(this, 'KANJI')
        Regex::new(r#"^showLargePopup\(\s*this\s*,\s*['\"][^'\"]+['\"]\s*\)$"#).unwrap(),
        // hideKanjiPopup()
        Regex::new(r"^hideKanjiPopup\(\s*\)$").unwrap(),
        // toggleStory('id') and toggleKanjiDetails('id')
        Regex::new(r#"^toggleStory\(\s*['\"][^'\"]+['\"]\s*\)$"#).unwrap(),
        Regex::new(r#"^toggleKanjiDetails\(\s*['\"][^'\"]+['\"]\s*\)$"#).unwrap(),
        // simple this.style.display='none' (common inline onclick used in addon)
        Regex::new(r#"^this\.style\.display\s*=\s*['\"][^'\"]+['\"]$"#).unwrap(),
        // keep cards_ct_click pattern for the kbd special-case
        Regex::new(r"^cards_ct_click\('\d+'\)$").unwrap(),
    ]
});

static ALLOWED_IFRAME_SRC_REGEX: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"^https://(www\.)?youtube(-nocookie)?\.com/embed/").unwrap()
});

fn is_safe_css_value(value: &str) -> bool {
    if value.chars().any(char::is_control) {
        return false;
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    for ch in ['<', '>', '\\', '{', '}', '@', ':', '"', '&', '='] {
        if trimmed.contains(ch) {
            return false;
        }
    }
    if trimmed.contains("/*") || trimmed.contains("*/") || trimmed.contains("//") {
        return false;
    }
    let lower = trimmed.to_lowercase();
    if lower.contains("-moz-binding") || lower.contains("__pitch_accent_comment_") {
        return false;
    }

    if trimmed.contains('(') || trimmed.contains(')') {
        let mut depth = 0usize;
        let mut open_idx = None;

        for (idx, ch) in trimmed.char_indices() {
            if ch == '(' {
                if depth != 0 {
                    return false;
                }
                depth = 1;
                open_idx = Some(idx);

                let before_slice = &trimmed[..idx];
                match before_slice.chars().next_back() {
                    Some(c) if c.is_ascii_alphabetic() || c == '-' => {}
                    _ => return false,
                }

                let mut start = idx;
                for (c_idx, c) in before_slice.char_indices().rev() {
                    if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                        start = c_idx;
                    } else {
                        break;
                    }
                }
                let token = &trimmed[start..idx];
                if !token.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
                    return false;
                }
                if !ALLOWED_CSS_FUNCTIONS.contains(token.to_lowercase().as_str()) {
                    return false;
                }
            } else if ch == ')' {
                if depth != 1 {
                    return false;
                }
                depth = 0;
                let start_inner = match open_idx {
                    Some(o) => o + 1,
                    None => return false,
                };
                let end_inner = idx;
                let inner = &trimmed[start_inner..end_inner];
                if inner.trim().is_empty() {
                    return false;
                }
                let valid_inner = inner.chars().all(|c| {
                    c.is_ascii_alphanumeric()
                        || c.is_ascii_whitespace()
                        || matches!(c, '%' | '.' | ',' | '/' | '+' | '-')
                });
                if !valid_inner {
                    return false;
                }
                open_idx = None;
            }
        }

        if depth != 0 {
            return false;
        }
    }

    true
}

fn is_safe_cursor_value(value: &str) -> bool {
    let lower = value.to_lowercase();
    let mut cursor_token = lower.trim();
    if let Some(idx) = cursor_token.rfind('!') {
        let (head, tail) = cursor_token.split_at(idx);
        if tail[1..].trim() == "important" {
            cursor_token = head.trim();
        }
    }
    ALLOWED_CURSOR_VALUES.contains(cursor_token)
}

fn is_safe_color_value(value: &str) -> bool {
    let lower = value.to_lowercase();
    let mut color_token = lower.trim();
    if let Some(idx) = color_token.rfind('!') {
        let (head, tail) = color_token.split_at(idx);
        if tail[1..].trim() == "important" {
            color_token = head.trim();
        }
    }
    if color_token.is_empty() {
        return false;
    }
    if let Some(hex) = color_token.strip_prefix('#') {
        matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit())
    } else if let Some(inner) = color_token
        .strip_prefix("rgb(")
        .or_else(|| color_token.strip_prefix("rgba("))
        .or_else(|| color_token.strip_prefix("hsl("))
        .or_else(|| color_token.strip_prefix("hsla("))
        .and_then(|s| s.strip_suffix(')'))
    {
        !inner.trim().is_empty()
            && inner.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || c.is_ascii_whitespace()
                    || matches!(c, '%' | '.' | ',' | '/' | '+' | '-')
            })
    } else {
        ALLOWED_COLOR_NAMES.contains(color_token)
    }
}

/// Sanitize inline style declarations by retaining only allowed CSS properties.
fn sanitize_style(style: &str) -> String {
    // FIX: HTML entities like &quot; contain semicolons, which breaks the split(';') below.
    // We safely replace quote entities with single quotes (valid in CSS) so properties aren't shattered.
    let normalized_style = style
        .replace("&quot;", "'")
        .replace("&#39;", "'")
        .replace("&#34;", "'")
        .replace("&apos;", "'");

    normalized_style
        .split(';')
        .filter_map(|decl| {
            let decl = decl.trim();
            if decl.is_empty() {
                return None;
            }
            // Split property and value.
            if let Some((prop, value)) = decl.split_once(':') {
                let prop_clean = prop.trim().to_lowercase();
                let value_clean = value.trim();
                if !is_safe_css_value(value_clean) {
                    return None;
                }
                if ALLOWED_CSS_PROPERTIES.contains(prop_clean.as_str()) {
                    if matches!(
                        prop_clean.as_str(),
                        "background-color" | "color" | "border-color" | "fill" | "stroke"
                    ) {
                        if is_safe_color_value(value_clean) {
                            Some(format!("{prop_clean}: {value_clean};"))
                        } else {
                            None
                        }
                    } else if prop_clean == "cursor" {
                        if is_safe_cursor_value(value_clean) {
                            Some(format!("{prop_clean}: {value_clean};"))
                        } else {
                            None
                        }
                    } else if prop_clean == "writing-mode" {
                        let value_lower = value_clean.to_lowercase();
                        if ALLOWED_WRITING_MODE_VALUES.contains(value_lower.as_str()) {
                            Some(format!("{prop_clean}: {value_clean};"))
                        } else {
                            None
                        }
                    } else {
                        Some(format!("{prop_clean}: {value_clean};"))
                    }
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect::<String>()
}

static STYLE_TAG_REGEX: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"(?is)<style\b[^>]*>(.*?)</style>").unwrap());

#[allow(dead_code)]
pub fn strip_style_tags(html: &str) -> String {
    STYLE_TAG_REGEX.replace_all(html, "").to_string()
}

fn strip_css_comments(css: &str) -> String {
    let mut result = String::with_capacity(css.len());
    let mut remainder = css;
    while let Some(start) = remainder.find("/*") {
        result.push_str(&remainder[..start]);
        let after_start = &remainder[start + 2..];
        if let Some(end) = after_start.find("*/") {
            remainder = &after_start[end + 2..];
        } else {
            remainder = "";
            break;
        }
    }
    result.push_str(remainder);
    result
}

fn sanitize_selector(selector: &str) -> Option<String> {
    if selector.contains("/*") || selector.contains("//") {
        return None;
    }
    if selector.to_lowercase().contains("__pitch_accent_comment_") {
        return None;
    }
    for c in selector.chars() {
        if !(c.is_ascii_alphanumeric()
            || c.is_ascii_whitespace()
            || matches!(
                c,
                '_' | '-' | '.' | '#' | ',' | ':' | '>' | '+' | '~' | '(' | ')'
            ))
        {
            return None;
        }
    }

    let sub_selectors: Vec<String> = selector
        .split(',')
        .map(|s| {
            let words: Vec<&str> = s.split_whitespace().collect();
            words.join(" ")
        })
        .collect();

    if sub_selectors.is_empty() || sub_selectors.iter().any(String::is_empty) {
        return None;
    }

    for sub in &sub_selectors {
        let has_class_or_id = sub
            .as_bytes()
            .windows(2)
            .any(|w| (w[0] == b'.' || w[0] == b'#') && w[1].is_ascii_alphanumeric());
        if !has_class_or_id {
            return None;
        }

        let sub_lower = sub.to_lowercase();
        if sub_lower.contains(":root") {
            return None;
        }

        let is_ident_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
        for word in sub_lower.split(|c| !is_ident_char(c)) {
            if word == "html" || word == "body" {
                return None;
            }
        }
    }

    Some(sub_selectors.join(", "))
}

fn sanitize_stylesheet(css: &str) -> String {
    let clean_css = strip_css_comments(css);
    let mut rules = Vec::new();

    for segment in clean_css.split('}') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        if let Some((selector, body)) = segment.split_once('{') {
            if body.contains('{') {
                continue;
            }
            if let Some(selector_clean) = sanitize_selector(selector) {
                let sanitized_body = sanitize_style(body);
                if !sanitized_body.is_empty() {
                    rules.push(format!("{selector_clean} {{ {sanitized_body} }}"));
                }
            }
        }
    }

    if rules.is_empty() {
        String::new()
    } else {
        format!("<style>{}</style>", rules.join("\n"))
    }
}

/// This regex matches pitch accent plugin comments that need to be preserved.
static PITCH_ACCENT_COMMENTS_REGEX: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"<!--\s*(user_)?accent_(start|end)\s*-->").unwrap());

/// Extract pitch accent comments from HTML before cleaning.
fn extract_pitch_accent_comments(html: &str) -> Vec<String> {
    PITCH_ACCENT_COMMENTS_REGEX
        .find_iter(html)
        .map(|m| m.as_str().to_string())
        .collect()
}

static PITCH_ACCENT_PLACEHOLDER_REGEX: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"(?i)__pitch_accent_comment_").unwrap());

/// The ammonia builder, allowing the "style" attribute.
static CLEANSER: std::sync::LazyLock<Builder<'static>> = std::sync::LazyLock::new(|| {
    let mut builder = Builder::default();

    builder.rm_clean_content_tags(&["style"]);
    builder.add_tags(&["style"]);

    // Allow common attributes plugin uses, plus Yomitan metadata attributes.
    // We will special-case `onclick` in the attribute_filter.
    builder.add_generic_attributes(&[
        "style",
        "class",
        "data-src",
        "id",
        "onclick",
        "title",
        // Yomitan specific attributes
        "data-sc-content",
        "data-sc-class",
        "data-sc-code",
        "data-sc-source",
        "lang",
    ]);

    builder.add_tags(&["font"]);
    builder.add_tag_attributes("font", &["color"]);

    // Allow a constrained subset of SVG needed for user accent pitch graphics.
    builder.add_tags(&[
        "svg", "text", "path", "circle",
        // Common HTML elements used by plugins & Yomitan
        "div", "span", "h2", "button", "table", "tr", "td", "ul", "ol", "li", "small", "hr", "ruby",
        "rt", "i", "b", "a", "iframe",
    ]);

    builder.add_tag_attributes("svg", &["width", "height", "viewBox", "class"]);
    builder.add_tag_attributes("text", &["x", "y", "style"]);
    builder.add_tag_attributes("path", &["d", "style"]);
    builder.add_tag_attributes("circle", &["r", "cx", "cy", "style"]);
    builder.add_tag_attributes("kbd", &["onclick", "ondblclick"]); // https://ankiweb.net/shared/info/1170639320
    builder.add_tag_attributes(
        "iframe",
        &[
            "width",
            "height",
            "src",
            "title",
            "frameborder",
            "allow",
            "referrerpolicy",
            "allowfullscreen",
        ],
    );
    builder.attribute_filter(|tag, attr, value| {
        if tag == "style" {
            return None;
        }

        if attr.eq_ignore_ascii_case("style") {
            let sanitized = sanitize_style(value);
            if sanitized.is_empty() {
                return None;
            }
            return Some(sanitized.into());
        }

        if tag == "iframe" && attr == "src" {
            if ALLOWED_IFRAME_SRC_REGEX.is_match(value) {
                return Some(value.into());
            }
            return None;
        }

        // If this is an onclick attribute, only allow it when it matches
        // one of the approved patterns for the addon.
        if attr == "onclick" {
            for re in ALLOWED_ONCLICK_REGEXES.iter() {
                if re.is_match(value) {
                    return Some(value.into());
                }
            }
            // no match -> strip the onclick
            return None;
        }

        // Keep the existing kbd special-case for cards_ct_click.
        if tag == "kbd" && (attr == "onclick" || attr == "ondblclick") {
            let re = Regex::new(r"^cards_ct_click\('\d+'\)$").unwrap();
            if re.is_match(value) {
                Some(value.into())
            } else {
                None
            }
        } else {
            // default handling for other attributes
            Some(value.into())
        }
    });
    builder
});

/// Restore pitch accent comments to cleaned HTML.
fn restore_pitch_accent_comments(cleaned_html: &str, comments: &[String]) -> String {
    let mut result = cleaned_html.to_string();

    // Replace placeholder markers with actual comments
    for (i, comment) in comments.iter().enumerate() {
        let placeholder = format!("__PITCH_ACCENT_COMMENT_{i}__");
        result = result.replace(&placeholder, comment);
    }

    result
}

/// Clean the provided HTML and then sanitize inline style declarations.
pub fn clean(src: &str) -> String {
    if src.trim().is_empty() {
        return String::new();
    }
    if !ammonia::is_html(src) {
        return src.to_string();
    }

    // Strip pre-existing pitch accent placeholder markers to prevent forgery
    let mut sanitized_src = src.to_string();
    while PITCH_ACCENT_PLACEHOLDER_REGEX.is_match(&sanitized_src) {
        sanitized_src = PITCH_ACCENT_PLACEHOLDER_REGEX
            .replace_all(&sanitized_src, "")
            .to_string();
    }

    // Extract pitch accent comments before cleaning
    let pitch_accent_comments = extract_pitch_accent_comments(&sanitized_src);

    // Replace comments with placeholders before cleaning
    let mut html_with_placeholders = sanitized_src;
    for (i, comment) in pitch_accent_comments.iter().enumerate() {
        let placeholder = format!("__PITCH_ACCENT_COMMENT_{i}__");
        html_with_placeholders = html_with_placeholders.replace(comment, &placeholder);
    }

    let sanitized_html = CLEANSER.clean(&html_with_placeholders).to_string();

    let style_sanitized = STYLE_TAG_REGEX.replace_all(&sanitized_html, |caps: &regex::Captures| {
        let inner_css = caps.get(1).map_or("", |m| m.as_str());
        sanitize_stylesheet(inner_css)
    });

    // Restore pitch accent comments
    restore_pitch_accent_comments(&style_sanitized, &pitch_accent_comments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_layout_styles() {
        let html = r#"<span style="width: 100%; max-width: 330px; height: 2em">x</span>"#;
        let expected = r#"<span style="width: 100%;max-width: 330px;height: 2em;">x</span>"#;
        assert_eq!(clean(html), expected);
    }

    #[test]
    fn keeps_table_borders() {
        let html = r#"<table style="border-collapse: collapse"></table>"#;
        assert_eq!(
            clean(html),
            r#"<table style="border-collapse: collapse;"></table>"#
        );
        let html = r#"<p style="border-bottom: 1px solid #ccc">x</p>"#;
        assert_eq!(
            clean(html),
            r#"<p style="border-bottom: 1px solid #ccc;">x</p>"#
        );
    }

    #[test]
    fn still_drops_unknown_styles() {
        let html = r#"<img src="a.jpg" style="height: auto; position: absolute">"#;
        assert_eq!(clean(html), r#"<img src="a.jpg" style="height: auto;">"#);
    }

    #[test]
    fn ac1_deck_description_style_tag_and_body_sanitized() {
        let input = r#"<html>
<div class="container"><div class="amboss-ankiphil"><a href="https://next.amboss.com/de/article/B80zK3"><img src="_amboss-feat-ankiphil2.png" style="width:900px;max-width:100%"></a></div>
<div class="license">© 2026 AMBOSS feat. Ankiphil. All rights reserved.</div>
<style>
.container {
max-width: 950px;
margin: auto;
}
.amboss-ankiphil {
text-align: center;
border-radius: 22px;
box-shadow: 0 8px 10px -1px hsl(220deg 25% 2% / 0.45), 0 2px 4px -2px hsl(220deg 25% 2% / 0.4);
background-color: #09464D;
padding: 7px 10px 0px 10px !important;
width: 100%;
}
.nightMode .amboss-ankiphil {
background-color: #1B5B65;
box-shadow: inset 0 1px 0 0 rgba(255, 255, 255, 0.1), 0 10px 30px -5px rgba(0, 0, 0, 0.4);

}
.amboss-ankiphil:hover {
background-color: #0e7b88;
cursor: pointer;
}
.license {
font-size: 10px;
text-align: right;
}
</style><b>Viel Erfolg mit Ankiphil :)</b>

Die Karteikarten wurden mit größtmöglicher Sorgfalt erstellt, jedoch kann für die Richtigkeit, Vollständigkeit und Aktualität keine Gewähr übernommen werden. Die Inhalte und Informationen dienen nur Informations- und Lernzwecken und erteilen ausdrücklich keine Ratschläge oder Empfehlungen hinsichtlich der Therapie konkreter Erkrankungen.</div></html>"#;

        let expected_style = "<style>.container { max-width: 950px;margin: auto; }\n.amboss-ankiphil { text-align: center;border-radius: 22px;box-shadow: 0 8px 10px -1px hsl(220deg 25% 2% / 0.45), 0 2px 4px -2px hsl(220deg 25% 2% / 0.4);background-color: #09464D;padding: 7px 10px 0px 10px !important;width: 100%; }\n.nightMode .amboss-ankiphil { background-color: #1B5B65;box-shadow: inset 0 1px 0 0 rgba(255, 255, 255, 0.1), 0 10px 30px -5px rgba(0, 0, 0, 0.4); }\n.amboss-ankiphil:hover { background-color: #0e7b88;cursor: pointer; }\n.license { font-size: 10px;text-align: right; }</style>";

        let cleaned = clean(input);
        assert!(cleaned.contains(expected_style));
        assert!(cleaned.contains(r#"style="width: 900px;max-width: 100%;""#));
        assert!(cleaned.contains(r#"<div class="container">"#));
        assert!(cleaned.contains("<b>Viel Erfolg mit Ankiphil :)</b>"));
    }

    #[test]
    fn attribute_breakout_prevented() {
        let payload = r#"<div title="a style='color:red' onclick=alert(1)">x</div>"#;
        let cleaned = clean(payload);
        assert_eq!(
            cleaned,
            r#"<div title="a style='color:red' onclick=alert(1)">x</div>"#
        );
        assert!(!cleaned.contains(r#"onclick="alert(1)""#));
    }

    #[test]
    fn disallowed_css_functions_rejected() {
        // image-set
        assert_eq!(
            clean(r#"<span style="box-shadow: image-set('a.png')">x</span>"#),
            "<span>x</span>"
        );
        assert_eq!(
            clean(r"<style>.a { box-shadow: image-set('a.png'); }</style>"),
            ""
        );

        // src
        assert_eq!(
            clean(r#"<span style="box-shadow: src('a.png')">x</span>"#),
            "<span>x</span>"
        );
        assert_eq!(
            clean(r"<style>.a { box-shadow: src('a.png'); }</style>"),
            ""
        );

        // u&Tab;rl
        assert_eq!(
            clean(r#"<span style="color: u&Tab;rl('javascript:alert(1)')">x</span>"#),
            "<span>x</span>"
        );
        assert_eq!(
            clean(r"<style>.a { color: u&Tab;rl('javascript:alert(1)'); }</style>"),
            ""
        );

        // var
        assert_eq!(
            clean(r#"<span style="color: var(--danger)">x</span>"#),
            "<span>x</span>"
        );
        assert_eq!(clean(r"<style>.a { color: var(--danger); }</style>"), "");
    }

    #[test]
    fn cursor_values_validation() {
        assert_eq!(
            clean(r#"<span style="cursor: pointer">x</span>"#),
            r#"<span style="cursor: pointer;">x</span>"#
        );
        assert_eq!(
            clean(r#"<span style="cursor: pointer !important">x</span>"#),
            r#"<span style="cursor: pointer !important;">x</span>"#
        );
        assert_eq!(
            clean(r#"<span style="cursor: url(evil.cur), auto">x</span>"#),
            "<span>x</span>"
        );
        assert_eq!(
            clean(r#"<span style="cursor: malicious">x</span>"#),
            "<span>x</span>"
        );
        assert_eq!(
            clean(r"<style>.a { cursor: wait; cursor: invalid; }</style>"),
            "<style>.a { cursor: wait; }</style>"
        );
    }

    #[test]
    fn global_selectors_rejected() {
        assert_eq!(clean("<style>body { color: red; }</style>"), "");
        assert_eq!(clean("<style>html { color: red; }</style>"), "");
        assert_eq!(clean("<style>:root { color: red; }</style>"), "");
        assert_eq!(clean("<style>* { color: red; }</style>"), "");
        assert_eq!(clean("<style>button { color: red; }</style>"), "");
        assert_eq!(clean("<style>body.main { color: red; }</style>"), "");
        assert_eq!(
            clean("<style>html body .container { color: red; }</style>"),
            ""
        );
        assert_eq!(clean("<style>:root .container { color: red; }</style>"), "");
        assert_eq!(
            clean("<style>.container button { color: red; }</style>"),
            "<style>.container button { color: red; }</style>"
        );
    }

    #[test]
    fn pitch_accent_placeholder_injection() {
        let payload = r"<style>.a { color: __PITCH_ACCENT_COMMENT_0__; }</style><!-- user_accent_start -->red<!-- user_accent_end -->";
        let cleaned = clean(payload);
        assert!(!cleaned.contains("__PITCH_ACCENT_COMMENT_0__"));
        assert_eq!(
            cleaned,
            "<!-- user_accent_start -->red<!-- user_accent_end -->"
        );

        let payload2 = r#"<div style="color: __PITCH_ACCENT_COMMENT_0__">x</div>"#;
        assert_eq!(clean(payload2), "<div>x</div>");
    }

    #[test]
    fn ac2_malicious_style_payloads_neutralized() {
        // Breakout attempt
        let breakout = "<style></style><script>alert(1)</script><style></style>";
        let cleaned = clean(breakout);
        assert!(!cleaned.contains("script"));
        assert!(!cleaned.contains("alert"));

        // Rules with @import, @media, url(...), expression(...), javascript:, backslash escapes (\), or attribute selectors
        let bad_rules = r#"<style>
            @import "evil.css";
            @media screen { .a { color: red; } }
            .b { background-image: url(https://evil.com/x.png); }
            .c { width: expression(alert(1)); }
            .d { color: javascript:alert(1); }
            .e\escape { color: blue; }
            input[value^=a] { color: green; }
            </style>"#;
        assert_eq!(clean(bad_rules), "");

        // Empty <style></style> or blocks where all rules are invalid are stripped completely
        assert_eq!(clean("<style></style>"), "");
        assert_eq!(clean("<style>   \n  </style>"), "");
        assert_eq!(clean("<style>/* only comment */</style>"), "");
        assert_eq!(clean("<style>.bad { unknown-prop: 123; }</style>"), "");

        // Attributes on <style> tags are stripped
        let with_attrs = r#"<style onclick="alert(1)" id="my-style" style="display:none">.valid { color: red; }</style>"#;
        assert_eq!(clean(with_attrs), "<style>.valid { color: red; }</style>");
    }

    #[test]
    fn ac3_strip_style_tags_removes_style_blocks() {
        let html_with_style = r"<p>Hello</p><style>.foo { color: red; }</style><span>World</span>";
        let cleaned = clean(html_with_style);
        assert!(cleaned.contains("<style>"));
        let stripped = strip_style_tags(&cleaned);
        assert_eq!(stripped, "<p>Hello</p><span>World</span>");
    }
}
