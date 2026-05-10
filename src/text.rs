use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

pub fn char_display_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

const TRUNCATION_MARKER: &str = "…";

pub fn truncate_with_ellipsis(text: &str, max_width: usize) -> String {
    if display_width(text) <= max_width {
        return text.to_string();
    }

    let marker_width = display_width(TRUNCATION_MARKER);
    if max_width == 0 {
        return String::new();
    }
    if max_width <= marker_width {
        return TRUNCATION_MARKER.to_string();
    }

    let prefix_width = max_width - marker_width;
    let mut output = String::new();
    let mut width = 0;
    for ch in text.chars() {
        let char_width = char_display_width(ch);
        if width + char_width > prefix_width {
            break;
        }
        output.push(ch);
        width += char_width;
    }
    output.push_str(TRUNCATION_MARKER);
    output
}

#[cfg(test)]
mod tests {
    use super::{char_display_width, display_width, truncate_with_ellipsis};

    #[test]
    fn leaves_short_text_unchanged() {
        assert_eq!(truncate_with_ellipsis("hello", 5), "hello");
        assert_eq!(truncate_with_ellipsis("hello", 10), "hello");
    }

    #[test]
    fn measures_terminal_display_width() {
        assert_eq!(display_width("hello"), 5);
        assert_eq!(display_width("好 chat"), 7);
        assert_eq!(char_display_width('好'), 2);
    }

    #[test]
    fn truncates_long_text_with_ellipsis() {
        assert_eq!(truncate_with_ellipsis("hello", 4), "hel…");
        assert_eq!(truncate_with_ellipsis("hello", 1), "…");
        assert_eq!(truncate_with_ellipsis("hello", 0), "");
    }

    #[test]
    fn truncates_on_char_boundaries() {
        assert_eq!(truncate_with_ellipsis("héllo", 4), "hél…");
    }

    #[test]
    fn truncates_by_display_width_for_wide_chars() {
        let truncated = truncate_with_ellipsis("好 chat", 4);

        assert_eq!(truncated, "好 …");
        assert!(display_width(&truncated) <= 4);
    }

    #[test]
    fn omits_wide_char_that_cannot_fit_before_ellipsis() {
        let truncated = truncate_with_ellipsis("好abc", 2);

        assert_eq!(truncated, "…");
        assert!(display_width(&truncated) <= 2);
    }
}
