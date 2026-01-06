use ammonia::Builder;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;

static ALLOWED_CSS_PROPERTIES: Lazy<HashSet<&'static str>> = Lazy::new(|| {
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
    ]
    .iter()
    .cloned()
    .collect()
});

static ALLOWED_COLOR_NAMES: Lazy<HashSet<&'static str>> = Lazy::new(|| {
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
    .cloned()
    .collect()
});

static ALLOWED_WRITING_MODE_VALUES: Lazy<HashSet<&'static str>> = Lazy::new(|| {
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
    .cloned()
    .collect()
});

/// Allowed onclick patterns used by the Anki addon Kanji Popup Dictionary with mobile support
static ALLOWED_ONCLICK_REGEXES: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // showKanjiPopup('X') or showKanjiPopup("X")
        Regex::new(r#"^showKanjiPopup\(\s*['\"][^'\"]+['\"]\s*\)$"#).unwrap(),
        // showLargePopup(this, 'KANJI')
        Regex::new(r#"^showLargePopup\(\s*this\s*,\s*['\"][^'\"]+['\"]\s*\)$"#).unwrap(),
        // hideKanjiPopup()
        Regex::new(r#"^hideKanjiPopup\(\s*\)$"#).unwrap(),
        // toggleStory('id') and toggleKanjiDetails('id')
        Regex::new(r#"^toggleStory\(\s*['\"][^'\"]+['\"]\s*\)$"#).unwrap(),
        Regex::new(r#"^toggleKanjiDetails\(\s*['\"][^'\"]+['\"]\s*\)$"#).unwrap(),
        // simple this.style.display='none' (common inline onclick used in addon)
        Regex::new(r#"^this\.style\.display\s*=\s*['\"][^'\"]+['\"]$"#).unwrap(),
        // keep cards_ct_click pattern for the kbd special-case
        Regex::new(r#"^cards_ct_click\('\d+'\)$"#).unwrap(),
    ]
});

static ALLOWED_IFRAME_SRC_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"^https://(www\.)?youtube(-nocookie)?\.com/embed/"#).unwrap()
});

/// Sanitize inline style declarations by retaining only allowed CSS properties.
fn sanitize_style(style: &str) -> String {
    style
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
                if ALLOWED_CSS_PROPERTIES.contains(prop_clean.as_str()) {
                    if matches!(
                        prop_clean.as_str(),
                        "background-color" | "color" | "border-color" | "fill" | "stroke"
                    ) {
                        let value_lower = value_clean.to_lowercase();
                        if !value_lower.starts_with('#')
                            && !value_lower.starts_with("rgb(")
                            && !value_lower.starts_with("rgba(")
                            && !value_lower.starts_with("hsl(")
                            && !value_lower.starts_with("hsla(")
                            && !ALLOWED_COLOR_NAMES.contains(&value_lower.as_str())
                        {
                            None
                        } else {
                            Some(format!("{}: {};", prop_clean, value_clean))
                        }
                    } else if prop_clean == "writing-mode" {
                        let value_lower = value_clean.to_lowercase();
                        if ALLOWED_WRITING_MODE_VALUES.contains(value_lower.as_str()) {
                            Some(format!("{}: {};", prop_clean, value_clean))
                        } else {
                            None
                        }
                    } else {
                        Some(format!("{}: {};", prop_clean, value_clean))
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

/// This regex matches a style attribute (case-insensitive) using either double or single quotes or unquoted attributes.
static STYLE_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)\s+style\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]*))"#).unwrap());

/// This regex matches pitch accent plugin comments that need to be preserved.
static PITCH_ACCENT_COMMENTS_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<!--\s*(user_)?accent_(start|end)\s*-->").unwrap());

/// Extract pitch accent comments from HTML before cleaning.
fn extract_pitch_accent_comments(html: &str) -> Vec<String> {
    PITCH_ACCENT_COMMENTS_REGEX
        .find_iter(html)
        .map(|m| m.as_str().to_string())
        .collect()
}

/// The ammonia builder, allowing the "style" attribute.
static CLEANSER: Lazy<Builder<'static>> = Lazy::new(|| {
    let mut builder = Builder::default();
    // Allow common attributes plugin uses. We will special-case `onclick`
    // in the attribute_filter to only permit a tiny whitelist of call
    // signatures.
    builder.add_generic_attributes(&["style", "class", "data-src", "id", "onclick"]);
    builder.add_tags(&["font"]);
    builder.add_tag_attributes("font", &["color"]);
    // Allow a constrained subset of SVG needed for user accent pitch graphics.
    builder.add_tags(&[
        "svg", "text", "path", "circle",
        // Add common HTML elements used by the addon Kanji Popup Dictionary with mobile support
        "div", "span", "h2", "button", "table", "tr", "td", "ul", "li", "small", "hr", "ruby", "rt",
        "iframe",
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

/// Post-process the sanitized HTML to restrict inline styles.
fn sanitize_html_styles(html: String) -> String {
    STYLE_REGEX
        .replace_all(&html, |caps: &regex::Captures| {
            let original = caps
                .get(1) // Double quotes
                .or_else(|| caps.get(2)) // Single quotes
                .or_else(|| caps.get(3)) // Unquoted
                .map_or("", |m| m.as_str()); // Use empty string if no match (shouldn't happen with this regex)

            let sanitized = sanitize_style(original);
            if sanitized.is_empty() {
                String::new()
            } else {
                format!(" style=\"{sanitized}\"")
            }
        })
        .to_string()
}

/// Restore pitch accent comments to cleaned HTML.
fn restore_pitch_accent_comments(cleaned_html: &str, comments: &[String]) -> String {
    let mut result = cleaned_html.to_string();

    // Replace placeholder markers with actual comments
    for (i, comment) in comments.iter().enumerate() {
        let placeholder = format!("__PITCH_ACCENT_COMMENT_{}__", i);
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

    // Extract pitch accent comments before cleaning
    let pitch_accent_comments = extract_pitch_accent_comments(src);

    // Replace comments with placeholders before cleaning
    let mut html_with_placeholders = src.to_string();
    for (i, comment) in pitch_accent_comments.iter().enumerate() {
        let placeholder = format!("__PITCH_ACCENT_COMMENT_{}__", i);
        html_with_placeholders = html_with_placeholders.replace(comment, &placeholder);
    }

    let sanitized_html = CLEANSER.clean(&html_with_placeholders).to_string();
    let style_sanitized = sanitize_html_styles(sanitized_html);

    // Restore pitch accent comments
    restore_pitch_accent_comments(&style_sanitized, &pitch_accent_comments)
}
