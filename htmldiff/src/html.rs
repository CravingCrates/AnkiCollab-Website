enum Mode {
    Char,
    Tag,
    Whitespace,
}

pub fn split(s: &str) -> Vec<&str> {
    let mut words = vec![];
    let mut start = 0;
    let mut mode = Mode::Char;

    for (i, c) in s.char_indices() {
        match mode {
            Mode::Char if is_start_of_tag(c) => {
                if start != i {
                    unsafe {
                        words.push(s.get_unchecked(start..i));
                    }
                }
                start = i;
                mode = Mode::Tag;
            }
            Mode::Char if is_whitespace(c) => {
                if start != i {
                    unsafe {
                        words.push(s.get_unchecked(start..i));
                    }
                }
                start = i;
                mode = Mode::Whitespace;
            }
            Mode::Char => { /* continue */ }
            Mode::Tag if is_end_of_tag(c) => {
                unsafe {
                    words.push(s.get_unchecked(start..=i));
                }
                start = i + 1;
                mode = Mode::Char;
            }
            Mode::Tag => { /* continue */ }
            Mode::Whitespace if is_start_of_tag(c) => {
                if start != i {
                    unsafe {
                        words.push(s.get_unchecked(start..i));
                    }
                }
                start = i;
                mode = Mode::Tag;
            }
            Mode::Whitespace if is_whitespace(c) => { /* continue */ }
            Mode::Whitespace => {
                if start != i {
                    unsafe {
                        words.push(s.get_unchecked(start..i));
                    }
                }
                start = i;
                mode = Mode::Char;
            }
        }
    }

    if start < s.len() {
        words.push(&s[start..]);
    }

    words
}

fn is_end_of_tag(c: char) -> bool {
    c == '>'
}

fn is_start_of_tag(c: char) -> bool {
    c == '<'
}

fn is_whitespace(c: char) -> bool {
    c.is_ascii_whitespace()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_html() {
        let actual = split("<p>Hello, world!</p>");
        let expected = vec!["<p>", "Hello,", " ", "world!", "</p>"];
        assert_eq!(actual, expected);
    }
}
