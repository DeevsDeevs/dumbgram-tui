#[cfg(test)]
use crate::text::display_width;
use color_eyre::{Result, eyre::eyre};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkSpan {
    pub(crate) url: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

pub(crate) fn links_in_text(text: &str) -> Vec<LinkSpan> {
    let mut starts: Vec<usize> = text
        .match_indices("https://")
        .chain(text.match_indices("http://"))
        .map(|(start, _)| start)
        .collect();
    starts.sort_unstable();

    let mut links = Vec::new();
    let mut covered_until = 0;
    for start in starts {
        if start < covered_until {
            continue;
        }

        let mut end = text.len();
        for (relative_index, character) in text[start..].char_indices() {
            if character.is_whitespace() || character.is_control() {
                end = start + relative_index;
                break;
            }
        }

        end = trim_url_end(text, start, end);
        if end > start {
            links.push(LinkSpan {
                url: text[start..end].to_string(),
                start,
                end,
            });
            covered_until = end;
        }
    }

    links
}

pub(crate) fn first_url(text: &str) -> Option<String> {
    links_in_text(text).into_iter().next().map(|link| link.url)
}

#[cfg(test)]
pub(crate) fn link_at_display_column(text: &str, column: usize) -> Option<String> {
    links_in_text(text).into_iter().find_map(|link| {
        let start_column = display_width(&text[..link.start]);
        let end_column = display_width(&text[..link.end]);
        (column >= start_column && column < end_column).then_some(link.url)
    })
}

pub(crate) fn open_url(url: &str) -> Result<()> {
    let status = opener_command(url).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(eyre!("browser command exited with status {status}"))
    }
}

#[cfg(target_os = "macos")]
fn opener_command(url: &str) -> Command {
    let mut command = Command::new("open");
    command.arg(url);
    command
}

#[cfg(target_os = "windows")]
fn opener_command(url: &str) -> Command {
    let mut command = Command::new("cmd");
    command.args(["/C", "start", "", url]);
    command
}

#[cfg(all(unix, not(target_os = "macos")))]
fn opener_command(url: &str) -> Command {
    let mut command = Command::new("xdg-open");
    command.arg(url);
    command
}

fn trim_url_end(text: &str, start: usize, mut end: usize) -> usize {
    while end > start {
        let Some(character) = text[start..end].chars().next_back() else {
            break;
        };
        if matches!(
            character,
            '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '>' | '"' | '\''
        ) {
            end -= character.len_utf8();
        } else {
            break;
        }
    }
    end
}

#[cfg(test)]
mod tests {
    use super::{first_url, link_at_display_column, links_in_text};

    #[test]
    fn links_in_text_finds_http_and_https_urls() {
        let links = links_in_text("see http://example.com and https://example.org/a?q=1");

        assert_eq!(links.len(), 2);
        assert_eq!(links[0].url, "http://example.com");
        assert_eq!(links[1].url, "https://example.org/a?q=1");
    }

    #[test]
    fn links_in_text_trims_common_sentence_punctuation() {
        assert_eq!(
            first_url("Open (https://example.org/path)."),
            Some("https://example.org/path".to_string())
        );
    }

    #[test]
    fn link_at_display_column_accounts_for_wide_prefix_text() {
        let text = "好 https://example.org";

        assert_eq!(link_at_display_column(text, 1), None);
        assert_eq!(
            link_at_display_column(text, 3),
            Some("https://example.org".to_string())
        );
    }
}
