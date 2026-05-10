use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

pub fn char_display_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

const TRUNCATION_MARKER: &str = "…";

pub fn truncate_with_ellipsis(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }

    let marker_width = display_width(TRUNCATION_MARKER);
    let prefix_width = max_width.saturating_sub(marker_width);
    let mut prefix = String::new();
    let mut prefix_display_width = 0;
    let mut text_display_width = 0;

    for ch in text.chars() {
        let char_width = char_display_width(ch);
        if text_display_width + char_width > max_width {
            if max_width <= marker_width {
                return TRUNCATION_MARKER.to_string();
            }
            prefix.push_str(TRUNCATION_MARKER);
            return prefix;
        }

        text_display_width += char_width;
        if prefix_display_width + char_width <= prefix_width {
            prefix.push(ch);
            prefix_display_width += char_width;
        }
    }

    text.to_string()
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
    fn truncates_long_text_without_scanning_full_content() {
        let text = format!("{}tail", "a".repeat(1_000_000));

        assert_eq!(truncate_with_ellipsis(&text, 4), "aaa…");
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
