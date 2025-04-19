use once_cell::sync::Lazy;
use ammonia::Builder;
use regex::Regex;

/// Sanitize inline style declarations by retaining only allowed CSS properties.
fn sanitize_style(style: &str, allowed_props: &[&str], allowed_colors: &[&str]) -> String {
    style
        .split(';')
        .filter_map(|decl| {
            let decl = decl.trim();
            if decl.is_empty() {
                return None;
            }
            // Split property and value.
            if let Some((prop, value)) = decl.split_once(':') {
                // Compare property names case-insensitively.
                let prop_clean = prop.trim().to_lowercase();                
                if allowed_props
                    .iter()
                    .any(|&allowed| allowed.eq_ignore_ascii_case(&prop_clean))
                {
                    if prop_clean == "background-color" || prop_clean == "color" 
                    {
                        if !value.trim().starts_with('#') && 
                            !value.trim().starts_with("rgb") && 
                            !value.trim().starts_with("rgba") && 
                            !value.trim().starts_with("hsl") && 
                            !value.trim().starts_with("hsla") && 
                            !allowed_colors.contains(&value.trim()) 
                        {
                            None
                        } else {
                            Some(format!("{}: {};", prop_clean, value.trim()))
                        }
                    } else {
                        Some(format!("{}: {};", prop_clean, value.trim()))
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
    Regex::new(r#"(?i)style\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]*))"#).unwrap()
});

/// The ammonia builder, allowing the "style" attribute.
static CLEANSER: Lazy<Builder<'static>> = Lazy::new(|| {
    let mut builder = Builder::default();
    builder.add_generic_attributes(&["style"]);
    builder
});

/// Post-process the sanitized HTML to restrict inline styles.
fn sanitize_html_styles(html: String) -> String {
    // Define allowed CSS properties.
    let allowed_props = [
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
    ];
    let allowed_colors = [
        "aqua", "black", "blue", 
        "fuchsia", "gray", "green", 
        "lime", "maroon", "navy", 
        "olive", "purple", "red", 
        "silver", "teal", "white", "yellow"
    ];
    STYLE_REGEX
        .replace_all(&html, |caps: &regex::Captures| {
            let original = if let Some(m) = caps.get(1) {
                m.as_str()
            } else if let Some(m) = caps.get(2) {
                m.as_str()
            } else {
                ""
            };
            let sanitized = sanitize_style(original, &allowed_props, &allowed_colors);
            if sanitized.is_empty() {
                String::new()
            } else {
                format!("style=\"{sanitized}\"")
            }
        })
        .to_string()
}

/// Clean the provided HTML and then sanitize inline style declarations.
pub fn clean(src: &str) -> String {
    if !ammonia::is_html(src) {
        return src.to_string();
    }
    let sanitized_html = CLEANSER.clean(src).to_string();
    sanitize_html_styles(sanitized_html)
}
