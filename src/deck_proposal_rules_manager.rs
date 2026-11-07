use crate::database;
use chrono::{Datelike, Utc};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeckProposalRules {
    pub deck: i64,
    // HTML Sanitization
    pub strip_colors: bool,
    pub strip_span: bool,
    pub strip_inline_styles: bool,
    pub strip_font_tags: bool,
    pub prevent_center_on_back: bool,
    pub trim_trailing_breaks: bool,
    pub normalize_spaces: bool,
    pub auto_repair_cloze: bool,
    pub strip_empty_elements: bool,

    // Tag Lifecycle
    pub auto_tag_new_cards: bool,
    pub new_card_tag_pattern: String,
    pub auto_tag_updated_cards: bool,
    pub updated_card_tag_pattern: String,
    pub prune_old_update_tags: bool,
    pub old_update_tag_pattern: String,
    pub forbidden_tags: String,
    pub normalize_tag_casing: String,
    pub protected_tag_namespaces: String,

    // Media Optimization
    pub enable_media_compression: bool,
    pub max_image_dimension: i32,
    pub image_quality: i32,
    pub auto_convert_heavy_images: bool,
    pub strip_exif_metadata: bool,
    pub max_media_file_size_mb: i32,

    // PR Policies
    pub require_rationale: bool,
    pub disallow_deletion_suggestions: bool,
    pub max_cards_per_proposal: i32,

    // Advanced Custom Rule Injections (.config format)
    pub custom_html_rules: String,
    pub custom_tag_rules: String,
}

impl Default for DeckProposalRules {
    fn default() -> Self {
        Self {
            deck: 0,
            strip_colors: false,
            strip_span: false,
            strip_inline_styles: false,
            strip_font_tags: false,
            prevent_center_on_back: false,
            trim_trailing_breaks: false,
            normalize_spaces: false,
            auto_repair_cloze: false,
            strip_empty_elements: false,

            auto_tag_new_cards: false,
            new_card_tag_pattern: "".to_string(),
            auto_tag_updated_cards: false,
            updated_card_tag_pattern: "".to_string(),
            prune_old_update_tags: false,
            old_update_tag_pattern: "".to_string(),
            forbidden_tags: "".to_string(),
            normalize_tag_casing: "none".to_string(),
            protected_tag_namespaces: "".to_string(),

            enable_media_compression: false,
            max_image_dimension: 0,
            image_quality: 0,
            auto_convert_heavy_images: false,
            strip_exif_metadata: false,
            max_media_file_size_mb: 0,

            require_rationale: false,
            disallow_deletion_suggestions: false,
            max_cards_per_proposal: 0,

            custom_html_rules: "".to_string(),
            custom_tag_rules: "".to_string(),
        }
    }
}

/// Fetch deck proposal rules from the database (walking up parent decks if `deck_id` is a subdeck) or return the default configuration.
pub async fn get_rules_with_client<C: tokio_postgres::GenericClient>(
    client: &C,
    deck_id: i64,
) -> Result<DeckProposalRules, Box<dyn std::error::Error + Send + Sync>> {
    let row = client
        .query_opt(
            "WITH RECURSIVE ancestors AS (
                 SELECT id, parent, 0 AS depth FROM anki.decks WHERE id = $1
                 UNION ALL
                 SELECT d.id, d.parent, a.depth + 1 FROM anki.decks d INNER JOIN ancestors a ON d.id = a.parent
             )
             SELECT r.deck, r.strip_colors, r.strip_span, r.strip_inline_styles, r.strip_font_tags,
                    r.prevent_center_on_back, r.trim_trailing_breaks, r.normalize_spaces,
                    r.auto_repair_cloze, r.strip_empty_elements, r.auto_tag_new_cards,
                    r.new_card_tag_pattern, r.auto_tag_updated_cards, r.updated_card_tag_pattern,
                    r.prune_old_update_tags, r.old_update_tag_pattern, r.forbidden_tags,
                    r.normalize_tag_casing, r.protected_tag_namespaces, r.enable_media_compression,
                    r.max_image_dimension, r.image_quality, r.auto_convert_heavy_images,
                    r.strip_exif_metadata, r.max_media_file_size_mb, r.require_rationale,
                    r.disallow_deletion_suggestions, r.max_cards_per_proposal,
                    r.custom_html_rules, r.custom_tag_rules
             FROM anki.deck_proposal_rules r
             JOIN ancestors a ON r.deck = a.id
             ORDER BY a.depth ASC
             LIMIT 1",
            &[&deck_id],
        )
        .await?;

    if let Some(r) = row {
        Ok(DeckProposalRules {
            deck: r.get(0),
            strip_colors: r.get(1),
            strip_span: r.get(2),
            strip_inline_styles: r.get(3),
            strip_font_tags: r.get(4),
            prevent_center_on_back: r.get(5),
            trim_trailing_breaks: r.get(6),
            normalize_spaces: r.get(7),
            auto_repair_cloze: r.get(8),
            strip_empty_elements: r.get(9),
            auto_tag_new_cards: r.get(10),
            new_card_tag_pattern: r.get(11),
            auto_tag_updated_cards: r.get(12),
            updated_card_tag_pattern: r.get(13),
            prune_old_update_tags: r.get(14),
            old_update_tag_pattern: r.get(15),
            forbidden_tags: r.get(16),
            normalize_tag_casing: r.get(17),
            protected_tag_namespaces: r.get(18),
            enable_media_compression: r.get(19),
            max_image_dimension: r.get(20),
            image_quality: r.get(21),
            auto_convert_heavy_images: r.get(22),
            strip_exif_metadata: r.get(23),
            max_media_file_size_mb: r.get(24),
            require_rationale: r.get(25),
            disallow_deletion_suggestions: r.get(26),
            max_cards_per_proposal: r.get(27),
            custom_html_rules: r.get(28),
            custom_tag_rules: r.get(29),
        })
    } else {
        Ok(DeckProposalRules {
            deck: deck_id,
            ..Default::default()
        })
    }
}

pub async fn get_rules(
    db_state: &database::AppState,
    deck_id: i64,
) -> Result<DeckProposalRules, Box<dyn std::error::Error + Send + Sync>> {
    let client = database::client(db_state).await?;
    get_rules_with_client(&*client, deck_id).await
}

/// Upsert deck proposal rules into the database.
pub async fn save_rules(
    db_state: &database::AppState,
    deck_id: i64,
    rules: &DeckProposalRules,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = database::client(db_state).await?;
    let query = "
        INSERT INTO anki.deck_proposal_rules (
            deck, strip_colors, strip_span, strip_inline_styles, strip_font_tags,
            prevent_center_on_back, trim_trailing_breaks, normalize_spaces,
            auto_repair_cloze, strip_empty_elements, auto_tag_new_cards,
            new_card_tag_pattern, auto_tag_updated_cards, updated_card_tag_pattern,
            prune_old_update_tags, old_update_tag_pattern, forbidden_tags,
            normalize_tag_casing, protected_tag_namespaces, enable_media_compression,
            max_image_dimension, image_quality, auto_convert_heavy_images,
            strip_exif_metadata, max_media_file_size_mb, require_rationale,
            disallow_deletion_suggestions, max_cards_per_proposal,
            custom_html_rules, custom_tag_rules, updated_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
            $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26,
            $27, $28, $29, $30, NOW()
        )
        ON CONFLICT (deck) DO UPDATE SET
            strip_colors = EXCLUDED.strip_colors,
            strip_span = EXCLUDED.strip_span,
            strip_inline_styles = EXCLUDED.strip_inline_styles,
            strip_font_tags = EXCLUDED.strip_font_tags,
            prevent_center_on_back = EXCLUDED.prevent_center_on_back,
            trim_trailing_breaks = EXCLUDED.trim_trailing_breaks,
            normalize_spaces = EXCLUDED.normalize_spaces,
            auto_repair_cloze = EXCLUDED.auto_repair_cloze,
            strip_empty_elements = EXCLUDED.strip_empty_elements,
            auto_tag_new_cards = EXCLUDED.auto_tag_new_cards,
            new_card_tag_pattern = EXCLUDED.new_card_tag_pattern,
            auto_tag_updated_cards = EXCLUDED.auto_tag_updated_cards,
            updated_card_tag_pattern = EXCLUDED.updated_card_tag_pattern,
            prune_old_update_tags = EXCLUDED.prune_old_update_tags,
            old_update_tag_pattern = EXCLUDED.old_update_tag_pattern,
            forbidden_tags = EXCLUDED.forbidden_tags,
            normalize_tag_casing = EXCLUDED.normalize_tag_casing,
            protected_tag_namespaces = EXCLUDED.protected_tag_namespaces,
            enable_media_compression = EXCLUDED.enable_media_compression,
            max_image_dimension = EXCLUDED.max_image_dimension,
            image_quality = EXCLUDED.image_quality,
            auto_convert_heavy_images = EXCLUDED.auto_convert_heavy_images,
            strip_exif_metadata = EXCLUDED.strip_exif_metadata,
            max_media_file_size_mb = EXCLUDED.max_media_file_size_mb,
            require_rationale = EXCLUDED.require_rationale,
            disallow_deletion_suggestions = EXCLUDED.disallow_deletion_suggestions,
            max_cards_per_proposal = EXCLUDED.max_cards_per_proposal,
            custom_html_rules = EXCLUDED.custom_html_rules,
            custom_tag_rules = EXCLUDED.custom_tag_rules,
            updated_at = NOW();";

    client
        .execute(
            query,
            &[
                &deck_id,
                &rules.strip_colors,
                &rules.strip_span,
                &rules.strip_inline_styles,
                &rules.strip_font_tags,
                &rules.prevent_center_on_back,
                &rules.trim_trailing_breaks,
                &rules.normalize_spaces,
                &rules.auto_repair_cloze,
                &rules.strip_empty_elements,
                &rules.auto_tag_new_cards,
                &rules.new_card_tag_pattern,
                &rules.auto_tag_updated_cards,
                &rules.updated_card_tag_pattern,
                &rules.prune_old_update_tags,
                &rules.old_update_tag_pattern,
                &rules.forbidden_tags,
                &rules.normalize_tag_casing,
                &rules.protected_tag_namespaces,
                &rules.enable_media_compression,
                &rules.max_image_dimension,
                &rules.image_quality,
                &rules.auto_convert_heavy_images,
                &rules.strip_exif_metadata,
                &rules.max_media_file_size_mb,
                &rules.require_rationale,
                &rules.disallow_deletion_suggestions,
                &rules.max_cards_per_proposal,
                &rules.custom_html_rules,
                &rules.custom_tag_rules,
            ],
        )
        .await?;

    Ok(())
}

static SPAN_OPEN_RE: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"(?i)<span(?:\s+[^>]*)?>").unwrap());
static SPAN_CLOSE_RE: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"(?i)</span>").unwrap());
static FONT_OPEN_RE: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"(?i)<font(?:\s+[^>]*)?>").unwrap());
static FONT_CLOSE_RE: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"(?i)</font>").unwrap());
static STYLE_ATTR_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r#"(?i)\s+style\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]*)"#).unwrap()
});
static COLOR_STYLE_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r#"(?i)(?:background-color|color)\s*:\s*[^;"]+;?"#).unwrap()
});
static CENTER_TAG_RE: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"(?i)</?center>").unwrap());
static CENTER_STYLE_RE: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r#"(?i)text-align\s*:\s*center;?"#).unwrap());
static CLOZE_TYPO_RE: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"\{\{c(\d+):([^\}:][^\}]*)\}\}").unwrap());
static TRAILING_BREAKS_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"(?i)(?:<br\s*/?>|<div>\s*<br\s*/?>\s*</div>|\s)+$").unwrap()
});
static EMPTY_TAGS_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"(?i)<(b|strong)[^>]*>\s*</(?:b|strong)>|<(i|em)[^>]*>\s*</(?:i|em)>|<p[^>]*>\s*</p>|<div[^>]*>\s*</div>|<span[^>]*>\s*</span>").unwrap()
});

/// Sanitize proposed HTML content according to the deck's automated formatting rules.
pub fn sanitize_html_content(raw: &str, is_back: bool, rules: &DeckProposalRules) -> String {
    if raw.trim().is_empty() {
        return String::new();
    }

    let mut result = raw.to_string();

    // 1. Normalize spaces
    if rules.normalize_spaces {
        result = result
            .replace("&nbsp;", " ")
            .replace('\u{00A0}', " ")
            .replace('\u{200B}', "");
    }

    // 2. Cloze auto-repair
    if rules.auto_repair_cloze {
        result = CLOZE_TYPO_RE
            .replace_all(&result, "{{c$1::$2}}")
            .to_string();
    }

    // 3. Strip or unwrap <span>
    if rules.strip_span {
        result = SPAN_OPEN_RE.replace_all(&result, "").to_string();
        result = SPAN_CLOSE_RE.replace_all(&result, "").to_string();
    }

    // 4. Strip or unwrap <font>
    if rules.strip_font_tags {
        result = FONT_OPEN_RE.replace_all(&result, "").to_string();
        result = FONT_CLOSE_RE.replace_all(&result, "").to_string();
    }

    // 5. Strip all inline styles or strip colors
    if rules.strip_inline_styles {
        result = STYLE_ATTR_RE.replace_all(&result, "").to_string();
    } else if rules.strip_colors {
        result = STYLE_ATTR_RE
            .replace_all(&result, |caps: &regex::Captures| {
                let full = caps.get(0).map_or("", |m| m.as_str());
                let cleaned = COLOR_STYLE_RE.replace_all(full, "").to_string();
                if cleaned.contains("style=\"\"")
                    || cleaned.contains("style=''")
                    || cleaned.trim() == "style="
                {
                    String::new()
                } else {
                    cleaned
                }
            })
            .to_string();
    }

    // 6. Prevent center on back
    if is_back && rules.prevent_center_on_back {
        result = CENTER_TAG_RE.replace_all(&result, "").to_string();
        result = STYLE_ATTR_RE
            .replace_all(&result, |caps: &regex::Captures| {
                let full = caps.get(0).map_or("", |m| m.as_str());
                let cleaned = CENTER_STYLE_RE.replace_all(full, "").to_string();
                if cleaned.contains("style=\"\"")
                    || cleaned.contains("style=''")
                    || cleaned.trim() == "style="
                {
                    String::new()
                } else {
                    cleaned
                }
            })
            .to_string();
    }

    // 7. Strip empty elements
    if rules.strip_empty_elements {
        for _ in 0..3 {
            let next = EMPTY_TAGS_RE.replace_all(&result, "").to_string();
            if next == result {
                break;
            }
            result = next;
        }
    }

    // 8. Trim trailing linebreaks & empty divs
    if rules.trim_trailing_breaks {
        result = TRAILING_BREAKS_RE.replace(&result, "").to_string();
    }

    // 9. Custom handwritten HTML rules (.config format)
    if !rules.custom_html_rules.trim().is_empty() {
        result = apply_custom_html_rules(result, &rules.custom_html_rules);
    }

    result
}

/// Parse and execute line-by-line custom HTML rules (.config syntax).
pub fn apply_custom_html_rules(mut html: String, custom_rules: &str) -> String {
    const MAX_DIRECTIVES: usize = 64;
    const REGEX_SIZE_LIMIT: usize = 256 * 1024;
    const MAX_HTML_OUTPUT_BYTES: usize = 512 * 1024;
    let build_regex = |pat: &str| {
        RegexBuilder::new(pat)
            .size_limit(REGEX_SIZE_LIMIT)
            .dfa_size_limit(REGEX_SIZE_LIMIT)
            .build()
    };

    for line in custom_rules.lines().take(MAX_DIRECTIVES) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some((directive, args)) = trimmed.split_once(':') {
            let dir = directive.trim().to_lowercase();
            let args = args.trim();

            match dir.as_str() {
                "strip-tags" | "unwrap-tags" => {
                    for tag in args.split(',').take(MAX_DIRECTIVES) {
                        let t = tag.trim().to_lowercase();
                        if !t.is_empty() {
                            let open_pattern = format!(r"(?i)<{}(?:\s+[^>]*)?>", regex::escape(&t));
                            let close_pattern = format!(r"(?i)</{}>", regex::escape(&t));
                            if let (Ok(o), Ok(c)) =
                                (build_regex(&open_pattern), build_regex(&close_pattern))
                            {
                                html = o.replace_all(&html, "").to_string();
                                html = c.replace_all(&html, "").to_string();
                            }
                        }
                    }
                }
                "remove-elements" => {
                    for tag in args.split(',').take(MAX_DIRECTIVES) {
                        let t = tag.trim().to_lowercase();
                        if !t.is_empty() {
                            let pattern = format!(
                                r"(?i)<{}(?:\s+[^>]*)?>.*?</{}>",
                                regex::escape(&t),
                                regex::escape(&t)
                            );
                            if let Ok(re) = build_regex(&pattern) {
                                html = re.replace_all(&html, "").to_string();
                            }
                        }
                    }
                }
                "strip-styles" => {
                    for prop in args.split(',').take(MAX_DIRECTIVES) {
                        let p = prop.trim().to_lowercase();
                        if !p.is_empty() {
                            let pattern = format!(r#"(?i){}\s*:\s*[^;"]+;?"#, regex::escape(&p));
                            if let Ok(re) = build_regex(&pattern) {
                                html = STYLE_ATTR_RE
                                    .replace_all(&html, |caps: &regex::Captures| {
                                        let full = caps.get(0).map_or("", |m| m.as_str());
                                        let cleaned = re.replace_all(full, "").to_string();
                                        if cleaned.contains("style=\"\"")
                                            || cleaned.contains("style=''")
                                            || cleaned.trim() == "style="
                                        {
                                            String::new()
                                        } else {
                                            cleaned
                                        }
                                    })
                                    .to_string();
                            }
                        }
                    }
                }
                "strip-attributes" => {
                    for attr in args.split(',').take(MAX_DIRECTIVES) {
                        let a = attr.trim().to_lowercase();
                        if !a.is_empty() {
                            let pattern = format!(
                                r#"(?i)\s+{}\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]*)"#,
                                regex::escape(&a)
                            );
                            if let Ok(re) = build_regex(&pattern) {
                                html = re.replace_all(&html, "").to_string();
                            }
                        }
                    }
                }
                "strip-empty" => {
                    for tag in args.split(',').take(MAX_DIRECTIVES) {
                        let t = tag.trim().to_lowercase();
                        if !t.is_empty() {
                            let pattern = format!(
                                r"(?i)<{}(?:\s+[^>]*)?>\s*</{}>",
                                regex::escape(&t),
                                regex::escape(&t)
                            );
                            if let Ok(re) = build_regex(&pattern) {
                                for _ in 0..3 {
                                    let next = re.replace_all(&html, "").to_string();
                                    if next == html {
                                        break;
                                    }
                                    html = next;
                                }
                            }
                        }
                    }
                }
                "replace" => {
                    if let Some((from_part, to_part)) = args.split_once("=>") {
                        let from_str = from_part.trim().trim_matches('"').trim_matches('\'');
                        let to_str = to_part.trim().trim_matches('"').trim_matches('\'');
                        if !from_str.is_empty() {
                            let next = html.replace(from_str, to_str);
                            if next.len() <= MAX_HTML_OUTPUT_BYTES {
                                html = next;
                            }
                        }
                    }
                }
                "regex-replace" => {
                    if let Some((from_part, to_part)) = args.split_once("=>") {
                        let pat_raw = from_part.trim();
                        let to_str = to_part.trim().trim_matches('"').trim_matches('\'');
                        let pattern = pat_raw
                            .strip_prefix('/')
                            .and_then(|s| s.strip_suffix('/'))
                            .unwrap_or(pat_raw);
                        if let Ok(re) = build_regex(pattern) {
                            let next = re.replace_all(&html, to_str).to_string();
                            if next.len() <= MAX_HTML_OUTPUT_BYTES {
                                html = next;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    html
}

/// Parse and execute line-by-line custom Tag rules (.config syntax).
pub fn apply_custom_tag_rules(mut tags: Vec<String>, custom_rules: &str) -> Vec<String> {
    for line in custom_rules.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some((directive, args)) = trimmed.split_once(':') {
            let dir = directive.trim().to_lowercase();
            let args = args.trim();

            match dir.as_str() {
                "block" | "remove" => {
                    for pat in args.split(',') {
                        let p = pat.trim().to_lowercase();
                        if !p.is_empty() {
                            tags.retain(|t| {
                                let t_low = t.to_lowercase();
                                if let Some(prefix) = p.strip_suffix('*') {
                                    !t_low.starts_with(prefix)
                                } else {
                                    t_low != p
                                }
                            });
                        }
                    }
                }
                "allow-only" => {
                    let patterns: Vec<String> = args
                        .split(',')
                        .map(|s| s.trim().to_lowercase())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !patterns.is_empty() {
                        tags.retain(|t| {
                            let t_low = t.to_lowercase();
                            patterns.iter().any(|p| {
                                if let Some(prefix) = p.strip_suffix('*') {
                                    t_low.starts_with(prefix)
                                } else {
                                    &t_low == p
                                }
                            })
                        });
                    }
                }
                "replace" => {
                    if let Some((from_part, to_part)) = args.split_once("=>") {
                        let from_str = from_part
                            .trim()
                            .trim_matches('"')
                            .trim_matches('\'')
                            .to_string();
                        let to_str = to_part
                            .trim()
                            .trim_matches('"')
                            .trim_matches('\'')
                            .to_string();
                        for t in &mut tags {
                            if *t == from_str {
                                *t = to_str.clone();
                            }
                        }
                    }
                }
                "prefix-all" => {
                    let prefix = args.trim().trim_matches('"').trim_matches('\'');
                    if !prefix.is_empty() {
                        for t in &mut tags {
                            if !t.starts_with(prefix) {
                                *t = format!("{prefix}{t}");
                            }
                        }
                    }
                }
                "case" => {
                    let c = args.trim().to_lowercase();
                    if c == "lower" || c == "lowercase" {
                        for t in &mut tags {
                            *t = t.to_lowercase();
                        }
                    } else if c == "upper" || c == "uppercase" {
                        for t in &mut tags {
                            *t = t.to_uppercase();
                        }
                    }
                }
                _ => {}
            }
        }
    }
    tags
}

/// Resolve date tokens in tag patterns:
/// `{YYYY}` -> "2026", `{YY}` -> "26", `{MM}` -> "10", `{DD}` -> "19",
/// `{YYYY-MM}` -> "2026-10", `{MM/YY}` -> "10/26", `{MM-YY}` -> "10-26"
pub fn resolve_date_pattern(pattern: &str) -> String {
    let now = Utc::now();
    let yyyy = format!("{:04}", now.year());
    let yy = format!("{:02}", now.year() % 100);
    let mm = format!("{:02}", now.month());
    let dd = format!("{:02}", now.day());
    let yyyy_mm = format!("{yyyy}-{mm}");
    let mm_yy = format!("{mm}/{yy}");
    let mm_hyphen_yy = format!("{mm}-{yy}");

    pattern
        .replace("{YYYY-MM}", &yyyy_mm)
        .replace("{yyyy-mm}", &yyyy_mm)
        .replace("{MM/YY}", &mm_yy)
        .replace("{mm/yy}", &mm_yy)
        .replace("{MM-YY}", &mm_hyphen_yy)
        .replace("{mm-yy}", &mm_hyphen_yy)
        .replace("{YYYY}", &yyyy)
        .replace("{yyyy}", &yyyy)
        .replace("{YY}", &yy)
        .replace("{yy}", &yy)
        .replace("{MM}", &mm)
        .replace("{mm}", &mm)
        .replace("{DD}", &dd)
        .replace("{dd}", &dd)
}

/// Check if a proposed tag matches any entry in the forbidden tags blacklist.
pub fn is_tag_forbidden(tag: &str, forbidden_tags: &str) -> bool {
    let clean_tag = tag.trim().to_lowercase();
    if clean_tag.is_empty() {
        return false;
    }

    for item in forbidden_tags.split(',') {
        let pattern = item.trim().to_lowercase();
        if pattern.is_empty() {
            continue;
        }
        if let Some(prefix) = pattern.strip_suffix('*') {
            if clean_tag.starts_with(prefix) {
                return true;
            }
        } else if clean_tag == pattern {
            return true;
        }
    }
    false
}

/// Check if a tag belongs to a protected namespace (non-maintainers cannot alter).
pub fn is_tag_protected(tag: &str, protected_namespaces: &str) -> bool {
    let clean_tag = tag.trim().to_lowercase();
    if clean_tag.is_empty() || protected_namespaces.trim().is_empty() {
        return false;
    }

    for item in protected_namespaces.split(',') {
        let pattern = item.trim().to_lowercase();
        if pattern.is_empty() {
            continue;
        }
        if let Some(prefix) = pattern.strip_suffix('*') {
            if clean_tag.starts_with(prefix) {
                return true;
            }
        } else if clean_tag == pattern {
            return true;
        }
    }
    false
}

/// Apply automated tagging and old-tag pruning when a card proposal is approved.
pub async fn apply_tagging_rules_on_approval(
    tx: &tokio_postgres::Transaction<'_>,
    note_id: i64,
    is_new: bool,
    rules: &DeckProposalRules,
    commit_id: Option<i32>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if is_new && rules.auto_tag_new_cards {
        let tag_name = resolve_date_pattern(&rules.new_card_tag_pattern);
        let tag_trimmed = tag_name.trim();
        if !tag_trimmed.is_empty() {
            tx.execute(
                "INSERT INTO anki.tags (note, content, commit, reviewed, action)
                 SELECT $1, $2, $3, true, true
                 WHERE NOT EXISTS (
                     SELECT 1 FROM anki.tags WHERE note = $1 AND content = $2 AND reviewed = true
                 )",
                &[&note_id, &tag_trimmed, &commit_id],
            )
            .await?;
        }
    } else if !is_new && rules.auto_tag_updated_cards {
        let tag_name = resolve_date_pattern(&rules.updated_card_tag_pattern);
        let tag_trimmed = tag_name.trim();
        if !tag_trimmed.is_empty() {
            // 1. Purge old update tags if configured
            if rules.prune_old_update_tags {
                let pattern = rules.old_update_tag_pattern.trim();
                let meaningful_base = pattern.trim_matches('*').trim();
                if !meaningful_base.is_empty() {
                    let escaped = pattern
                        .replace('\\', "\\\\")
                        .replace('%', "\\%")
                        .replace('_', "\\_");
                    let like_pattern = if let Some(prefix) = escaped.strip_suffix('*') {
                        format!("{}%", prefix.replace('*', "%"))
                    } else if escaped.contains('*') {
                        escaped.replace('*', "%")
                    } else {
                        format!("{escaped}%")
                    };

                    // Delete older version tags matching the pattern (except the new tag being added)
                    tx.execute(
                        "DELETE FROM anki.tags
                         WHERE note = $1 AND content LIKE $2 ESCAPE '\\' AND content <> $3",
                        &[&note_id, &like_pattern, &tag_trimmed],
                    )
                    .await?;
                }
            }

            // 2. Insert the fresh version update tag if not already present
            tx.execute(
                "INSERT INTO anki.tags (note, content, commit, reviewed, action)
                 SELECT $1, $2, $3, true, true
                 WHERE NOT EXISTS (
                     SELECT 1 FROM anki.tags WHERE note = $1 AND content = $2 AND reviewed = true
                 )",
                &[&note_id, &tag_trimmed, &commit_id],
            )
            .await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_natively_off_default() {
        let rules = DeckProposalRules::default();
        assert!(!rules.strip_colors);
        assert!(!rules.strip_span);
        assert!(!rules.strip_inline_styles);
        assert!(!rules.strip_font_tags);
        assert!(!rules.prevent_center_on_back);
        assert!(!rules.trim_trailing_breaks);
        assert!(!rules.normalize_spaces);
        assert!(!rules.auto_repair_cloze);
        assert!(!rules.strip_empty_elements);
        assert!(!rules.auto_tag_new_cards);
        assert!(!rules.auto_tag_updated_cards);
        assert!(!rules.prune_old_update_tags);
        assert!(!rules.enable_media_compression);
        assert!(!rules.auto_convert_heavy_images);
        assert!(!rules.strip_exif_metadata);
        assert!(!rules.require_rationale);
        assert!(!rules.disallow_deletion_suggestions);
        assert_eq!(rules.max_cards_per_proposal, 0);
        assert_eq!(rules.max_media_file_size_mb, 0);
        assert_eq!(rules.custom_html_rules, "");
        assert_eq!(rules.custom_tag_rules, "");
    }

    #[test]
    fn test_unwrap_span_preserves_content() {
        let mut rules = DeckProposalRules::default();
        rules.strip_span = true;
        let input = "<p>Avant <span style=\"color: red; font-size: 16px;\">notion importante</span> après</p>";
        let output = sanitize_html_content(input, false, &rules);
        assert_eq!(output, "<p>Avant notion importante après</p>");
    }

    #[test]
    fn test_strip_font_tags() {
        let mut rules = DeckProposalRules::default();
        rules.strip_font_tags = true;
        let input = "<div><font color=\"#ff0000\">Texte rouge</font></div>";
        let output = sanitize_html_content(input, false, &rules);
        assert_eq!(output, "<div>Texte rouge</div>");
    }

    #[test]
    fn test_trim_trailing_breaks() {
        let mut rules = DeckProposalRules::default();
        rules.trim_trailing_breaks = true;
        let input = "Contenu utile<br><br/><div><br></div>   ";
        let output = sanitize_html_content(input, false, &rules);
        assert_eq!(output, "Contenu utile");
    }

    #[test]
    fn test_prevent_center_on_back() {
        let mut rules = DeckProposalRules::default();
        rules.prevent_center_on_back = true;
        let input = "<div style=\"text-align: center;\">Réponse</div>";
        let output_back = sanitize_html_content(input, true, &rules);
        assert!(!output_back.contains("center"));

        let output_front = sanitize_html_content(input, false, &rules);
        assert!(output_front.contains("text-align: center;"));
    }

    #[test]
    fn test_cloze_auto_repair() {
        let mut rules = DeckProposalRules::default();
        rules.auto_repair_cloze = true;
        let input = "Le diagnostic repose sur {{c1:l'ECG}}.";
        let output = sanitize_html_content(input, false, &rules);
        assert_eq!(output, "Le diagnostic repose sur {{c1::l'ECG}}.");
    }

    #[test]
    fn test_custom_html_rules() {
        let mut rules = DeckProposalRules::default();
        rules.custom_html_rules = r#"
            # Remove custom spans and dangerous elements
            strip-tags: span, em
            remove-elements: script, iframe
            strip-styles: color, font-family
            replace: "foo" => "bar"
        "#
        .to_string();

        let input = r#"<span>Important <script>alert(1)</script> <em>message</em> with foo and <div style="color: red; font-family: Arial; font-weight: bold;">alert</div></span>"#;
        let output = sanitize_html_content(input, false, &rules);

        assert!(!output.contains("<span>"));
        assert!(!output.contains("<em>"));
        assert!(!output.contains("<script>"));
        assert!(!output.contains("alert(1)"));
        assert!(!output.contains("color: red"));
        assert!(!output.contains("font-family: Arial"));
        assert!(output.contains("font-weight: bold"));
        assert!(output.contains("bar"));
    }

    #[test]
    fn test_custom_tag_rules() {
        let custom_rules = r#"
            # Comment line
            block: marked, leech, flag*
            replace: "old_item" => "Item-100"
            prefix-all: "EDN::"
            case: lower
        "#;

        let tags = vec![
            "marked".to_string(),
            "flag2".to_string(),
            "old_item".to_string(),
            "Cardio".to_string(),
        ];
        let processed = apply_custom_tag_rules(tags, custom_rules);

        assert!(!processed.contains(&"marked".to_string()));
        assert!(!processed.contains(&"flag2".to_string()));
        assert!(processed.contains(&"edn::item-100".to_string()));
        assert!(processed.contains(&"edn::cardio".to_string()));
    }

    #[test]
    fn test_resolve_date_pattern() {
        let res = resolve_date_pattern("NEW::{YYYY}");
        let year = Utc::now().year().to_string();
        assert_eq!(res, format!("NEW::{year}"));

        let res_maj = resolve_date_pattern("MAJ::{MM/YY}");
        let now = Utc::now();
        let expected = format!("MAJ::{:02}/{:02}", now.month(), now.year() % 100);
        assert_eq!(res_maj, expected);
    }

    #[test]
    fn test_is_tag_forbidden() {
        let forbidden = "marked, leech, flag*, todo*";
        assert!(is_tag_forbidden("marked", forbidden));
        assert!(is_tag_forbidden("leech", forbidden));
        assert!(is_tag_forbidden("flag1", forbidden));
        assert!(is_tag_forbidden("TODO_revoir", forbidden));
        assert!(!is_tag_forbidden("EDN::Cardio", forbidden));
        assert!(!is_tag_forbidden("Source::2026::Cardio", forbidden));
    }
}
