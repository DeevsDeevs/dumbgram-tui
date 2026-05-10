use crate::{app::App, diagnostics};
use base64::Engine;
use image::ImageFormat;
use std::{env, fs, io::Cursor, io::Write, path::Path};

const KITTY_ESCAPE_START: &str = "\x1b_G";
const KITTY_ESCAPE_END: &str = "\x1b\\";
const MAX_IMAGE_COLUMNS: u16 = 40;
const MAX_IMAGE_ROWS: u16 = 12;
const MIN_IMAGE_COLUMNS: u16 = 8;
const MIN_IMAGE_ROWS: u16 = 4;

pub(crate) fn terminal_supports_kitty_graphics() -> bool {
    terminal_env_supports_kitty_graphics(
        env::var("TERM_PROGRAM").ok().as_deref(),
        env::var("TERM").ok().as_deref(),
    )
}

pub(crate) fn terminal_env_supports_kitty_graphics(
    term_program: Option<&str>,
    term: Option<&str>,
) -> bool {
    let term_program = term_program.unwrap_or_default().to_ascii_lowercase();
    let term = term.unwrap_or_default().to_ascii_lowercase();

    term_program.contains("ghostty")
        || term_program.contains("kitty")
        || term.contains("ghostty")
        || term.contains("kitty")
}

pub(crate) fn clear_kitty_images_sequence() -> String {
    format!("{KITTY_ESCAPE_START}a=d,d=A,q=2;{KITTY_ESCAPE_END}")
}

pub(crate) fn clear_terminal_images<W: Write>(writer: &mut W) -> std::io::Result<()> {
    clear_terminal_images_for_support(writer, terminal_supports_kitty_graphics())
}

fn clear_terminal_images_for_support<W: Write>(
    writer: &mut W,
    supports_kitty_graphics: bool,
) -> std::io::Result<()> {
    if supports_kitty_graphics {
        writer.write_all(clear_kitty_images_sequence().as_bytes())?;
        writer.flush()?;
    }
    Ok(())
}

pub(crate) fn kitty_png_image_sequence(bytes: &[u8], columns: u16, rows: u16) -> String {
    let encoded_image = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!(
        "{KITTY_ESCAPE_START}a=T,f=100,q=2,z=1,c={columns},r={rows};{encoded_image}{KITTY_ESCAPE_END}"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImagePayloadSourceFormat {
    Png,
    Jpeg,
}

impl ImagePayloadSourceFormat {
    fn diagnostic_label(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
        }
    }
}

pub(crate) fn kitty_png_file_image_at_sequence(
    path: &Path,
    column: u16,
    row: u16,
    columns: u16,
    rows: u16,
) -> std::io::Result<(String, usize, ImagePayloadSourceFormat)> {
    let (bytes, source_format) = kitty_png_payload_bytes(path)?;
    let byte_len = bytes.len();
    Ok((
        format!(
            "\x1b[{};{}H{}",
            row.saturating_add(1),
            column.saturating_add(1),
            kitty_png_image_sequence(&bytes, columns, rows)
        ),
        byte_len,
        source_format,
    ))
}

fn kitty_png_payload_bytes(path: &Path) -> std::io::Result<(Vec<u8>, ImagePayloadSourceFormat)> {
    let bytes = fs::read(path)?;
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok((bytes, ImagePayloadSourceFormat::Png));
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        let image = image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let mut png = Cursor::new(Vec::new());
        image
            .write_to(&mut png, ImageFormat::Png)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        return Ok((png.into_inner(), ImagePayloadSourceFormat::Jpeg));
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "unsupported image preview format",
    ))
}

pub(crate) fn render_selected_image<W: Write>(
    writer: &mut W,
    app: &mut App,
) -> std::io::Result<()> {
    let term_program = env::var("TERM_PROGRAM").ok();
    let term = env::var("TERM").ok();
    if !terminal_env_supports_kitty_graphics(term_program.as_deref(), term.as_deref()) {
        log_terminal_image_once(
            app,
            format!(
                "skip:unsupported:{}:{}",
                terminal_kind(term_program.as_deref()),
                terminal_kind(term.as_deref())
            ),
            "terminal_image_skip",
            format!(
                "reason=unsupported_terminal term_program={} term={}",
                terminal_kind(term_program.as_deref()),
                terminal_kind(term.as_deref())
            ),
        );
        return Ok(());
    }

    writer.write_all(clear_kitty_images_sequence().as_bytes())?;

    let selected_message = app.state.selected_message();
    let selected = selected_message.is_some();
    let message_id = selected_message.map(|message| message.id);
    let media = selected_message.and_then(|message| message.media.as_ref());
    let has_media = media.is_some();
    let path = media
        .and_then(|media| media.local_image_path())
        .map(Path::to_path_buf);
    let Some(path) = path else {
        log_terminal_image_once(
            app,
            format!("skip:no-image:{selected}:{has_media}"),
            "terminal_image_skip",
            format!(
                "reason=no_selected_image selected={selected} media={has_media} local_path=false"
            ),
        );
        writer.flush()?;
        return Ok(());
    };

    let area = app.state.terminal_image_area;
    let columns = area.width.saturating_sub(2).min(MAX_IMAGE_COLUMNS);
    let rows = area.height.saturating_sub(2).min(MAX_IMAGE_ROWS);
    if columns < MIN_IMAGE_COLUMNS || rows < MIN_IMAGE_ROWS {
        log_terminal_image_once(
            app,
            format!(
                "skip:area:{}:{}:{}:{}",
                area.width, area.height, columns, rows
            ),
            "terminal_image_skip",
            format!(
                "reason=area_too_small area_width={} area_height={} columns={columns} rows={rows}",
                area.width, area.height
            ),
        );
        writer.flush()?;
        return Ok(());
    }

    let column = area.x.saturating_add(1);
    let row = area.y.saturating_add(1);
    let (sequence, byte_len, source_format) =
        match kitty_png_file_image_at_sequence(&path, column, row, columns, rows) {
            Ok(sequence) => sequence,
            Err(error) => {
                log_terminal_image_once(
                    app,
                    format!("error:read:{:?}", error.kind()),
                    "terminal_image_error",
                    format!("stage=read_image error_kind={:?}", error.kind()),
                );
                writer.flush()?;
                return Ok(());
            }
        };
    writer.write_all(sequence.as_bytes())?;
    writer.flush()?;
    log_terminal_image_once(
        app,
        format!(
            "render:{:?}:{}:{}:{}:{}:{}",
            message_id,
            byte_len,
            source_format.diagnostic_label(),
            columns,
            rows,
            area.width
        ),
        "terminal_image_render",
        format!(
            "message_id={} bytes={byte_len} source_format={} columns={columns} rows={rows} area_width={} area_height={}",
            message_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            source_format.diagnostic_label(),
            area.width,
            area.height
        ),
    );
    Ok(())
}

fn terminal_kind(value: Option<&str>) -> &'static str {
    let value = value.unwrap_or_default().to_ascii_lowercase();
    if value.is_empty() {
        "empty"
    } else if value.contains("ghostty") {
        "ghostty"
    } else if value.contains("kitty") {
        "kitty"
    } else {
        "other"
    }
}

fn log_terminal_image_once(app: &mut App, key: String, name: &str, details: String) {
    if app.terminal_image_diagnostic_key.as_deref() == Some(key.as_str()) {
        return;
    }
    app.terminal_image_diagnostic_key = Some(key);
    diagnostics::event(name, details);
}

#[cfg(test)]
mod tests {
    use super::{
        ImagePayloadSourceFormat, clear_kitty_images_sequence, clear_terminal_images_for_support,
        kitty_png_file_image_at_sequence, kitty_png_image_sequence,
        terminal_env_supports_kitty_graphics,
    };
    use base64::Engine;
    use image::{DynamicImage, ImageFormat, RgbImage};
    use std::{fs, io::Cursor, path::Path};

    #[test]
    fn terminal_support_detection_accepts_ghostty_or_kitty() {
        assert!(terminal_env_supports_kitty_graphics(
            Some("ghostty"),
            Some("xterm-256color")
        ));
        assert!(terminal_env_supports_kitty_graphics(
            Some("Apple_Terminal"),
            Some("xterm-kitty")
        ));
        assert!(terminal_env_supports_kitty_graphics(
            Some(""),
            Some("xterm-ghostty")
        ));
        assert!(!terminal_env_supports_kitty_graphics(
            Some("Apple_Terminal"),
            Some("xterm-256color")
        ));
    }

    #[test]
    fn kitty_png_image_sequence_encodes_png_bytes_and_cell_size() {
        let sequence = kitty_png_image_sequence(b"png-bytes", 20, 8);

        assert!(sequence.starts_with("\x1b_G"));
        assert!(sequence.ends_with("\x1b\\"));
        assert!(sequence.contains("a=T,f=100,q=2,z=1,c=20,r=8;"));
        assert!(sequence.contains("cG5nLWJ5dGVz"));
        assert!(!sequence.contains("png-bytes"));
    }

    #[test]
    fn kitty_png_file_image_at_sequence_moves_cursor_before_image() {
        let path = std::env::temp_dir().join(format!(
            "dumbgram-terminal-image-test-{}.png",
            std::process::id()
        ));
        let png_bytes = base64::engine::general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMB/axK6JkAAAAASUVORK5CYII=")
            .unwrap();
        fs::write(&path, &png_bytes).unwrap();

        let (sequence, byte_len, source_format) =
            kitty_png_file_image_at_sequence(Path::new(&path), 2, 3, 10, 4).unwrap();

        assert_eq!(byte_len, png_bytes.len());
        assert_eq!(source_format, ImagePayloadSourceFormat::Png);
        assert!(sequence.starts_with("\x1b[4;3H\x1b_G"));
        assert!(!sequence.contains(path.to_string_lossy().as_ref()));
        fs::remove_file(path).ok();
    }

    #[test]
    fn kitty_png_file_image_at_sequence_converts_jpeg_to_png_payload() {
        let path = std::env::temp_dir().join(format!(
            "dumbgram-terminal-image-test-{}.jpg",
            std::process::id()
        ));
        let image = RgbImage::from_fn(2, 2, |x, y| image::Rgb([x as u8 * 120, y as u8 * 120, 80]));
        let mut jpeg = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image)
            .write_to(&mut jpeg, ImageFormat::Jpeg)
            .unwrap();
        fs::write(&path, jpeg.into_inner()).unwrap();

        let (sequence, byte_len, source_format) =
            kitty_png_file_image_at_sequence(Path::new(&path), 2, 3, 10, 4).unwrap();

        assert_eq!(source_format, ImagePayloadSourceFormat::Jpeg);
        assert!(byte_len > 0);
        assert!(sequence.starts_with("\x1b[4;3H\x1b_G"));
        assert!(sequence.contains("a=T,f=100,q=2,z=1,c=10,r=4;"));
        fs::remove_file(path).ok();
    }

    #[test]
    fn clear_kitty_images_sequence_deletes_visible_images_quietly() {
        assert_eq!(clear_kitty_images_sequence(), "\x1b_Ga=d,d=A,q=2;\x1b\\");
    }

    #[test]
    fn terminal_kind_keeps_diagnostics_non_sensitive() {
        assert_eq!(super::terminal_kind(None), "empty");
        assert_eq!(super::terminal_kind(Some("Ghostty")), "ghostty");
        assert_eq!(super::terminal_kind(Some("xterm-kitty")), "kitty");
        assert_eq!(super::terminal_kind(Some("xterm-256color")), "other");
    }

    #[test]
    fn clear_terminal_images_only_writes_for_supported_terminals() {
        let mut supported = Vec::new();
        clear_terminal_images_for_support(&mut supported, true).unwrap();
        assert_eq!(supported, clear_kitty_images_sequence().as_bytes());

        let mut unsupported = Vec::new();
        clear_terminal_images_for_support(&mut unsupported, false).unwrap();
        assert!(unsupported.is_empty());
    }
}
