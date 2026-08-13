use htmldiff::htmldiff;

#[test]
fn style_tag_replacement() {
    let old = "<p>Hello, <u>world</u>!</p>";
    let new = "<p>Hello, <b>world</b>!</p>";
    let diff = htmldiff(old, new);
    assert_eq!(
        diff,
        "<p>Hello, <del data-diff><u>world</u></del><ins data-diff><b>world</b></ins>!</p>"
    );
}
