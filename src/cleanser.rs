use once_cell::sync::Lazy;
use ammonia::Builder;
use regex::Regex;
use std::collections::HashSet;

static ALLOWED_CSS_PROPERTIES: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "color",
        "background-color",
        "font-size",
        "font-weight",
        "text-align",
        "text-decoration",
        "line-height",
        "margin",
        "padding",
        "border-width",
        "border-style",
        "border-color",
    ]
    .iter()
    .cloned()
    .collect()
});

static ALLOWED_COLOR_NAMES: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "aqua", "black", "blue", "fuchsia", "gray", "green", "lime",
        "maroon", "navy", "olive", "purple", "red", "silver", "teal",
        "white", "yellow",
        "currentcolor", "transparent",
        "rebeccapurple",
    ]
    .iter()
    .cloned()
    .collect()
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
                if ALLOWED_CSS_PROPERTIES.contains(prop_clean.as_str())
                {
                    if prop_clean == "background-color" || prop_clean == "color" || prop_clean == "border-color" 
                    {
                        let value_lower = value_clean.to_lowercase();
                        if !value_lower.starts_with('#') && 
                            !value_lower.starts_with("rgb(") && 
                            !value_lower.starts_with("rgba(") && 
                            !value_lower.starts_with("hsl(") && 
                            !value_lower.starts_with("hsla(") && 
                            !ALLOWED_COLOR_NAMES.contains(&value_lower.as_str()) 
                        {
                            None
                        } else {
                            Some(format!("{}: {};", prop_clean, value_clean))
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
static STYLE_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)\s+style\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]*))"#).unwrap()
});

/// The ammonia builder, allowing the "style" attribute.
static CLEANSER: Lazy<Builder<'static>> = Lazy::new(|| {
    let mut builder = Builder::default();
    builder.add_generic_attributes(&["style", "class", "data-src"]);
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

/// Clean the provided HTML and then sanitize inline style declarations.
pub fn clean(src: &str) -> String {
    if src.trim().is_empty() {
        return String::new();
    }
    if !ammonia::is_html(src) {
        return src.to_string();
    }
    let sanitized_html = CLEANSER.clean(src).to_string();
    sanitize_html_styles(sanitized_html)
}
