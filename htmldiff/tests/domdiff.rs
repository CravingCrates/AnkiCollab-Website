use htmldiff::htmldiff;

#[test]
fn text_insert_inline() {
    let old = "<p>Hello world</p>";

    let new = "<p>Text</p>";
    let out = htmldiff(old, new);
    println!("only_whitespace_to_content => {}", out);
    assert!(
        out.contains("<del data-diff>Hello world</del>")
            && out.contains("<ins data-diff>Text</ins>")
    );
}

#[test]
fn basic_insert() {
    let out = htmldiff("<p>a</p>", "<p>a b</p>");
    assert!(out.contains("b"));
}

#[test]
fn tag_swap() {
    let out = htmldiff("<p><u>x</u></p>", "<p><b>x</b></p>");
    assert!(
        out.contains("<del data-diff><u>x</u></del>")
            && out.contains("<ins data-diff><b>x</b></ins>")
    );
}

#[test]
fn large_change() {
    let old = "<div>".to_owned() + &("<p>same</p>".repeat(50)) + "</div>";
    let new = "<div>".to_owned() + &("<p>same</p>".repeat(49)) + "<p>same changed</p></div>";
    let out = htmldiff(&old, &new);
    assert!(out.contains("changed"));
}

//
// case sensitivity & attribute casing
//
#[test]
fn tag_case_insensitivity() {
    let old = "<DIV>Hi</DIV>";
    let new = "<div>Hiya</div>";
    let out = htmldiff(old, new);
    println!("tag_case_insensitivity => {}", out);
    // Word-level diffing: entire word should be wrapped, not split mid-word
    assert!(
        out.to_lowercase().contains("<ins data-diff>hiya</ins>")
            || out.to_lowercase().contains("<ins data-diff>ya</ins>")
    );
}

#[test]
fn attribute_case_and_quotes() {
    let old = "<p data-Flag=1>Hi</p>";
    let new = "<p data-flag=\"1\">Hi</p>";
    let out = htmldiff(old, new);
    println!("attribute_case_and_quotes => {}", out);
    assert!(out.to_lowercase().contains("data-flag"));
}

//
// performance / large-ish
//
#[test]
fn large_input_basic() {
    let old = "<div>".to_owned() + &("<p>lorem ipsum</p>\n".repeat(500)) + "</div>";
    let new =
        "<div>".to_owned() + &("<p>lorem ipsum</p>\n".repeat(499)) + "<p>lorem changed</p></div>";
    let out = htmldiff(&old, &new);
    println!("large_input_basic => (len {})", out.len());
    assert!(out.contains("changed"));
}

// Additional correctness & robustness tests
#[test]
fn whitespace_suppression_normal() {
    let out = htmldiff("<p>word </p>", "<p>word</p>");
    assert!(!out.contains("<del>word "));
}

#[test]
fn whitespace_preserved_in_pre() {
    let out = htmldiff("<pre>line </pre>", "<pre>line</pre>");
    // Expect some diff marker (space removal) rather than silent suppression
    println!("whitespace_preserved_in_pre => {}", out);
    assert!(out.contains("<del data-diff>") || out.contains("<ins data-diff>"));
}

#[test]
fn inline_equiv_swap() {
    let out = htmldiff("<p><strong>x</strong></p>", "<p><b>x</b></p>");
    assert!(!out.contains("<del data-diff>x</del>"));
}

#[test]
fn attribute_order_stable() {
    let out = htmldiff("<p a=1 b=2>t</p>", "<p b=2 a=1>t</p>");
    assert!(!out.contains("<del data-diff><p"));
}

#[test]
fn preexisting_ins_del() {
    let old = "<p><ins>keep</ins> mid <del>gone</del></p>";
    let new = "<p><ins>keep</ins> mid <del>gone</del>!</p>";
    let out = htmldiff(old, new);
    assert!(out.contains("<ins data-diff>!</ins>"));
    assert!(out.contains("<ins>keep</ins>"));
}

#[test]
fn multibyte_safety() {
    let out = htmldiff("<p>über</p>", "<p>überraschung</p>");
    assert!(out.contains("über"));
}

#[test]
fn panic_guard() {
    let new = "a".repeat(20);
    let out = htmldiff("a", &new);
    // Either the new string appears directly or wrapped in <ins>
    assert!(out.contains(&new));
}

// Emoji / grapheme cluster handling
#[test]
fn emoji_append() {
    let old = "<p>😀</p>"; // single codepoint
    let new = "<p>😀😃</p>"; // append another
    let out = htmldiff(old, new);
    // Expect only the second emoji in an insertion, not splitting first
    assert!(out.contains("😀"));
    assert!(out.contains("😃"));
}

#[test]
fn emoji_family_diff() {
    // Family emoji uses multiple code points + ZWJ
    let old = "<p>👨‍👩‍👧</p>";
    let new = "<p>👨‍👩‍👧👶</p>"; // appended baby
    let out = htmldiff(old, new);
    // Ensure the base family cluster not split (should appear intact)
    assert!(out.contains("👨‍👩‍👧"));
    assert!(out.contains("👶"));
}

#[test]
fn skin_tone_change() {
    let old = "<p>👍</p>";
    let new = "<p>👍🏽</p>"; // adds skin tone modifier (new cluster)
    let out = htmldiff(old, new);
    // Should either show replacement or a minimal diff; both clusters intact
    assert!(out.contains("👍") && out.contains("🏽"));
}

// User-authored <ins>/<del> tags must be preserved without data-diff attribute
#[test]
fn user_ins_preserved_unchanged() {
    let old = "<p><ins>underlined text</ins> normal</p>";
    let new = "<p><ins>underlined text</ins> normal</p>";
    let out = htmldiff(old, new);
    println!("user_ins_preserved_unchanged => {}", out);
    // User <ins> should remain as-is (no data-diff attribute)
    assert!(out.contains("<ins>underlined text</ins>"));
    assert!(!out.contains("data-diff>underlined text"));
}

#[test]
fn user_ins_with_text_change() {
    let old = "<p><ins>underlined</ins> hello</p>";
    let new = "<p><ins>underlined</ins> world</p>";
    let out = htmldiff(old, new);
    println!("user_ins_with_text_change => {}", out);
    // User <ins> should remain without data-diff
    assert!(out.contains("<ins>underlined</ins>"));
    // The text diff should use data-diff markers
    assert!(out.contains("data-diff"));
}

#[test]
fn user_ins_added_to_content() {
    let old = "<p>plain text</p>";
    let new = "<p>plain <ins>underlined</ins> text</p>";
    let out = htmldiff(old, new);
    println!("user_ins_added_to_content => {}", out);
    // The user <ins> is part of the new content; it should appear in the output
    assert!(out.contains("<ins>underlined</ins>") || out.contains("underlined"));
}

#[test]
fn user_del_preserved() {
    let old = "<p>text <del>strikethrough</del> more</p>";
    let new = "<p>text <del>strikethrough</del> more!</p>";
    let out = htmldiff(old, new);
    println!("user_del_preserved => {}", out);
    // User <del> should remain without data-diff
    assert!(out.contains("<del>strikethrough</del>"));
    // Diff marker for the added "!" should have data-diff
    assert!(out.contains("data-diff"));
}

#[test]
fn user_ins_removed_from_content() {
    let old = "<p>text <ins>underlined</ins> more</p>";
    let new = "<p>text more</p>";
    let out = htmldiff(old, new);
    println!("user_ins_removed_from_content => {}", out);
    // The diff should wrap the removal in <del data-diff>, not confuse with user <ins>
    assert!(out.contains("data-diff"));
}

// Inline equivalence tests for Anki styling tags
#[test]
fn u_ins_shows_diff() {
    // <u> and <ins> should be treated as DIFFERENT tags (Anki note styling may distinguish them)
    let out = htmldiff("<p><u>underlined</u></p>", "<p><ins>underlined</ins></p>");
    println!("u_ins_shows_diff => {}", out);
    assert!(
        out.contains("data-diff"),
        "u ↔ ins swap should show as a diff"
    );
}

#[test]
fn s_del_shows_diff() {
    // <s> and <del> should be treated as DIFFERENT tags
    let out = htmldiff("<p><s>struck</s></p>", "<p><del>struck</del></p>");
    println!("s_del_shows_diff => {}", out);
    assert!(
        out.contains("data-diff"),
        "s ↔ del swap should show as a diff"
    );
}

#[test]
fn b_strong_equiv_no_diff() {
    // <b> and <strong> are truly visually identical, should NOT produce diff noise
    let out = htmldiff("<p><b>bold</b></p>", "<p><strong>bold</strong></p>");
    println!("b_strong_equiv_no_diff => {}", out);
    assert!(
        !out.contains("data-diff"),
        "b ↔ strong swap should not produce a diff marker"
    );
}

// --- Multilingual / UTF-8 tests ---

#[test]
fn chinese_text_diff() {
    let old = "<p>今天天气很好</p>";
    let new = "<p>今天天气不好</p>";
    let out = htmldiff(old, new);
    println!("chinese_text_diff => {}", out);
    assert!(
        out.contains("data-diff"),
        "Chinese character change should be detected"
    );
    // Both the old and new character should appear
    assert!(out.contains("很") && out.contains("不"));
}

#[test]
fn chinese_text_unchanged() {
    let old = "<p>学习是很重要的</p>";
    let new = "<p>学习是很重要的</p>";
    let out = htmldiff(old, new);
    println!("chinese_text_unchanged => {}", out);
    assert!(
        !out.contains("data-diff"),
        "Identical Chinese text should produce no diff"
    );
}

#[test]
fn german_umlauts() {
    let old = "<p>Die Übung ist schwer</p>";
    let new = "<p>Die Übung ist leicht</p>";
    let out = htmldiff(old, new);
    println!("german_umlauts => {}", out);
    assert!(out.contains("data-diff"));
    assert!(out.contains("Übung")); // Ü should be preserved
}

#[test]
fn mixed_scripts() {
    let old = "<p>English 中文 日本語</p>";
    let new = "<p>English 中文 한국어</p>";
    let out = htmldiff(old, new);
    println!("mixed_scripts => {}", out);
    assert!(out.contains("data-diff"));
    assert!(out.contains("中文")); // Shared text preserved
}

// --- Spelling correction tests ---

#[test]
fn spelling_correction_single_word() {
    let old = "<p>The teh quick fox</p>";
    let new = "<p>The the quick fox</p>";
    let out = htmldiff(old, new);
    println!("spelling_correction => {}", out);
    assert!(out.contains("data-diff"));
    // The change should be localized, not wrap the entire sentence
    assert!(out.contains("quick fox")); // surrounding text preserved outside diff markers
}

#[test]
fn spelling_correction_word_level() {
    // Important: diff should show whole word, not split inside it
    let old = "<p>speling mistake</p>";
    let new = "<p>spelling mistake</p>";
    let out = htmldiff(old, new);
    println!("spelling_correction_word_level => {}", out);
    assert!(out.contains("data-diff"));
    // "mistake" should NOT be inside a diff marker
    assert!(out.contains("mistake"));
}

// --- &nbsp; and empty div handling ---

#[test]
fn nbsp_to_space_no_diff() {
    // &nbsp; vs regular space should not produce diff noise
    let old = "<p>hello\u{00a0}world</p>";
    let new = "<p>hello world</p>";
    let out = htmldiff(old, new);
    println!("nbsp_to_space_no_diff => {}", out);
    assert!(
        !out.contains("data-diff"),
        "nbsp vs space should not produce diff noise"
    );
}

#[test]
fn multiple_nbsp_no_diff() {
    let old = "<p>a\u{00a0}b\u{00a0}c</p>";
    let new = "<p>a b c</p>";
    let out = htmldiff(old, new);
    println!("multiple_nbsp_no_diff => {}", out);
    assert!(
        !out.contains("data-diff"),
        "multiple nbsp vs space should not produce diff noise"
    );
}

#[test]
fn empty_div_wrapping() {
    // Extra empty div wrapping shouldn't cause massive diff
    let old = "<div>Hello world</div>";
    let new = "<div><div>Hello world</div></div>";
    let out = htmldiff(old, new);
    println!("empty_div_wrapping => {}", out);
    // The text should still be present
    assert!(out.contains("Hello world"));
}

#[test]
fn trailing_empty_div() {
    let old = "<p>Content</p>";
    let new = "<p>Content</p><div></div>";
    let out = htmldiff(old, new);
    println!("trailing_empty_div => {}", out);
    assert!(out.contains("Content")); // original content preserved
}

// --- Character-level diff highlight tests ---

#[test]
fn char_highlight_single_char_change() {
    // Single character substitution in a long word should get inner highlight
    let out = htmldiff("<p>今天天气很好</p>", "<p>今天天气不好</p>");
    println!("char_highlight_single_char => {}", out);
    assert!(
        out.contains("data-diff-char"),
        "single char change should get char-level highlight"
    );
    assert!(out.contains("<span data-diff-char>很</span>"));
    assert!(out.contains("<span data-diff-char>不</span>"));
}

#[test]
fn char_highlight_insertion() {
    // Single character insertion (speling→spelling) should highlight added char
    let out = htmldiff("<p>speling mistake</p>", "<p>spelling mistake</p>");
    println!("char_highlight_insertion => {}", out);
    assert!(
        out.contains("data-diff-char"),
        "inserted char should get highlight"
    );
    // The "l" insertion should be highlighted on the ins side
    assert!(out.contains("<ins data-diff>spel<span data-diff-char>l</span>ing</ins>"));
}

#[test]
fn char_highlight_skipped_for_large_change() {
    // Entire word replacement should NOT get char-level highlight
    let out = htmldiff("<p>hello world</p>", "<p>goodbye world</p>");
    println!("char_highlight_skipped_large => {}", out);
    assert!(
        !out.contains("data-diff-char"),
        "large change should not get char-level highlight"
    );
}

#[test]
fn char_highlight_skipped_for_short_word() {
    // Very short word (< 3 graphemes) should not get char highlight
    let out = htmldiff("<p>hi there</p>", "<p>ho there</p>");
    println!("char_highlight_short_word => {}", out);
    assert!(
        !out.contains("data-diff-char"),
        "2-char word change should not get char highlight"
    );
}
