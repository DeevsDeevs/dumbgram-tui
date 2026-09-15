use chrono::{DateTime, Utc};
use color_eyre::Result;
use color_eyre::eyre::{WrapErr, bail, eyre};
use grammers_client::{Client, InvocationError, grammers_tl_types as tl, types};
use serde_json::{Map, Value, json};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const PROGRESS_EVERY: usize = 500;
const PHOTO_PROGRESS_EVERY: usize = 25;
const PHOTO_PLACEHOLDER: &str = "(photo)";
const BOT_API_CHANNEL_ID_OFFSET: i64 = 1_000_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatKind {
    Personal,
    Bot,
    Group,
    Supergroup,
    Channel,
}

impl ChatKind {
    fn of(chat: &types::Chat) -> Self {
        match chat {
            types::Chat::User(user) if user.is_bot() => Self::Bot,
            types::Chat::User(_) => Self::Personal,
            // grammers wraps megagroups (raw TL channels without `broadcast`) as `Chat::Group`.
            types::Chat::Group(group) => match group.raw {
                tl::enums::Chat::Channel(_) | tl::enums::Chat::ChannelForbidden(_) => {
                    Self::Supergroup
                }
                _ => Self::Group,
            },
            types::Chat::Channel(_) => Self::Channel,
        }
    }

    fn export_type(self) -> &'static str {
        match self {
            Self::Personal => "personal_chat",
            Self::Bot => "bot_chat",
            Self::Group => "private_group",
            Self::Supergroup => "private_supergroup",
            Self::Channel => "private_channel",
        }
    }

    fn peer_id(self, id: i64) -> String {
        match self {
            Self::Personal | Self::Bot => format!("user{id}"),
            Self::Group => format!("chat{id}"),
            Self::Supergroup | Self::Channel => format!("channel{id}"),
        }
    }

    fn bot_api_chat_id(self, chat_id: i64, own_user_id: i64) -> i64 {
        match self {
            Self::Personal | Self::Bot => own_user_id,
            Self::Group => -chat_id,
            Self::Supergroup | Self::Channel => -(BOT_API_CHANNEL_ID_OFFSET + chat_id),
        }
    }

    // Every private chat shares the account's own Bot API id, so those are looked up by user id.
    fn lookup_id(self, chat_id: i64) -> i64 {
        self.bot_api_chat_id(chat_id, chat_id)
    }
}

fn display_name(chat: &types::Chat) -> String {
    match chat {
        types::Chat::User(user) => user.full_name(),
        other => other.name().to_string(),
    }
}

fn sender_fields(chat: &types::Chat) -> (String, String) {
    (display_name(chat), ChatKind::of(chat).peer_id(chat.id()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExportMedia {
    Photo(String),
    Document { mime_type: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InlineButton {
    kind: &'static str,
    text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ExportMessage {
    id: i32,
    service: bool,
    date: DateTime<Utc>,
    edited: Option<DateTime<Utc>>,
    from: Option<(String, String)>,
    reply_to_message_id: Option<i32>,
    media: Option<ExportMedia>,
    text: String,
    inline_buttons: Vec<Vec<InlineButton>>,
}

impl ExportMessage {
    fn from_grammers(
        message: &types::Message,
        me: &(String, String),
        media_dir: Option<&MediaDir>,
    ) -> Self {
        // Channel posts and outgoing private-chat messages carry no `from_id`, so grammers
        // reports no sender; attribute them to the channel or to the exporting account.
        let from = match message.sender() {
            Some(sender) => Some(sender_fields(&sender)),
            None if message.post() => Some(sender_fields(&message.chat())),
            None if message.outgoing() => Some(me.clone()),
            None => None,
        };
        Self {
            id: message.id(),
            service: message.action().is_some(),
            date: message.date(),
            edited: message.edit_date(),
            from,
            reply_to_message_id: message.reply_to_message_id(),
            media: match message.media() {
                Some(types::Media::Photo(photo)) => Some(ExportMedia::Photo(photo_field(
                    message.id(),
                    media_dir.filter(|_| photo.to_raw_input_location().is_some()),
                ))),
                Some(types::Media::Document(document)) => Some(ExportMedia::Document {
                    mime_type: document.mime_type().map(str::to_string),
                }),
                Some(types::Media::Sticker(sticker)) => Some(ExportMedia::Document {
                    mime_type: sticker.document.mime_type().map(str::to_string),
                }),
                _ => None,
            },
            text: message.text().to_string(),
            inline_buttons: inline_buttons(message.reply_markup()),
        }
    }

    fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("id".into(), json!(self.id));
        out.insert(
            "type".into(),
            json!(if self.service { "service" } else { "message" }),
        );
        out.insert("date".into(), json!(export_date(self.date)));
        out.insert(
            "date_unixtime".into(),
            json!(self.date.timestamp().to_string()),
        );
        if self.service {
            return Value::Object(out);
        }
        if let Some(edited) = self.edited {
            out.insert("edited".into(), json!(export_date(edited)));
            out.insert(
                "edited_unixtime".into(),
                json!(edited.timestamp().to_string()),
            );
        }
        if let Some((name, peer_id)) = &self.from {
            out.insert("from".into(), json!(name));
            out.insert("from_id".into(), json!(peer_id));
        }
        if let Some(reply_to) = self.reply_to_message_id {
            out.insert("reply_to_message_id".into(), json!(reply_to));
        }
        match &self.media {
            Some(ExportMedia::Photo(photo)) => {
                out.insert("photo".into(), json!(photo));
            }
            Some(ExportMedia::Document { mime_type }) => {
                out.insert("file".into(), json!("(file)"));
                if let Some(mime_type) = mime_type {
                    out.insert("mime_type".into(), json!(mime_type));
                }
            }
            None => {}
        }
        out.insert("text".into(), json!(self.text));
        let entities = if self.text.is_empty() {
            json!([])
        } else {
            json!([{"type": "plain", "text": self.text}])
        };
        out.insert("text_entities".into(), entities);
        if !self.inline_buttons.is_empty() {
            let rows: Vec<Value> = self
                .inline_buttons
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|button| json!({"type": button.kind, "text": button.text}))
                        .collect()
                })
                .collect();
            out.insert("inline_bot_buttons".into(), Value::Array(rows));
        }
        Value::Object(out)
    }
}

fn export_date(date: DateTime<Utc>) -> String {
    date.format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn inline_buttons(markup: Option<tl::enums::ReplyMarkup>) -> Vec<Vec<InlineButton>> {
    let Some(tl::enums::ReplyMarkup::ReplyInlineMarkup(markup)) = markup else {
        return Vec::new();
    };
    markup
        .rows
        .into_iter()
        .map(|tl::enums::KeyboardButtonRow::Row(row)| {
            row.buttons
                .into_iter()
                .map(|button| InlineButton {
                    kind: match button {
                        tl::enums::KeyboardButton::Callback(_) => "callback",
                        tl::enums::KeyboardButton::Url(_) => "url",
                        _ => "other",
                    },
                    text: button.text(),
                })
                .collect()
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChatCandidate {
    id: i64,
    lookup_id: i64,
    name: String,
}

fn resolve_chat_query(query: &str, chats: &[ChatCandidate]) -> Result<usize> {
    let find = |predicate: &dyn Fn(&ChatCandidate) -> bool| -> Vec<usize> {
        (0..chats.len()).filter(|&i| predicate(&chats[i])).collect()
    };
    let matches = match query.parse::<i64>() {
        Ok(id) => {
            let by_lookup_id = find(&|chat| chat.lookup_id == id);
            if by_lookup_id.is_empty() {
                find(&|chat| chat.id == id)
            } else {
                by_lookup_id
            }
        }
        Err(_) => {
            let needle = query.to_lowercase();
            let exact = find(&|chat| chat.name.to_lowercase() == needle);
            if exact.is_empty() {
                find(&|chat| chat.name.to_lowercase().contains(&needle))
            } else {
                exact
            }
        }
    };
    match matches.as_slice() {
        [index] => Ok(*index),
        [] => bail!("no chat matches {query:?}"),
        _ => {
            let candidates: Vec<String> = matches
                .iter()
                .map(|&i| format!("{}\t{}", chats[i].lookup_id, chats[i].name))
                .collect();
            bail!(
                "{query:?} matches {} chats; re-run with a chat id:\n{}",
                matches.len(),
                candidates.join("\n")
            )
        }
    }
}

fn flood_wait(error: &InvocationError) -> Option<Duration> {
    match error {
        InvocationError::Rpc(rpc) if rpc.code == 420 => {
            Some(Duration::from_secs(rpc.value?.into()))
        }
        _ => None,
    }
}

// grammers sleeps through a FLOOD_WAIT only once per request and only up to
// `flood_sleep_threshold`; a whole-history export must wait out longer ones.
async fn wait_or_fail<E>(delay: Option<Duration>, error: E) -> Result<()>
where
    E: std::error::Error + Send + Sync + 'static,
{
    let Some(delay) = delay else {
        return Err(error.into());
    };
    eprintln!("Telegram FLOOD_WAIT: sleeping {}s", delay.as_secs());
    tokio::time::sleep(delay).await;
    Ok(())
}

pub struct ExportSummary {
    pub message_count: usize,
    pub chat_id: i64,
    pub bot_api_chat_id: i64,
}

/// Creates `path` (never overwriting, owner-only on unix), hands it to `run`, and removes it
/// again if `run` fails so a retry is not blocked by a partial file.
pub async fn with_output_file<T>(
    path: &Path,
    run: impl AsyncFnOnce(File) -> Result<T>,
) -> Result<T> {
    let file = create_private_file(path)?;
    let result = run(file).await;
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}

fn create_private_file(path: &Path) -> Result<File> {
    private_file_options()
        .create_new(true)
        .open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                eyre!("refusing to overwrite existing file: {}", path.display())
            } else {
                eyre!("failed to create {}: {error}", path.display())
            }
        })
}

fn private_file_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

struct MediaDir {
    dir: PathBuf,
    json_dir: PathBuf,
}

impl MediaDir {
    fn photo_path(&self, message_id: i32) -> PathBuf {
        self.dir.join(format!("photo_{message_id}.jpg"))
    }
}

fn photo_field(message_id: i32, media_dir: Option<&MediaDir>) -> String {
    let Some(media_dir) = media_dir else {
        return PHOTO_PLACEHOLDER.to_string();
    };
    let path = media_dir.photo_path(message_id);
    path.strip_prefix(&media_dir.json_dir)
        .unwrap_or(&path)
        .to_string_lossy()
        .into_owned()
}

fn create_media_dir(dir: &Path) -> Result<PathBuf> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir).map_err(|error| {
        eyre!(
            "failed to create media directory {}: {error}",
            dir.display()
        )
    })?;
    Ok(dir.canonicalize()?)
}

async fn download_photos(
    client: &Client,
    photos: Vec<(i32, types::Media)>,
    media_dir: &MediaDir,
) -> Result<()> {
    let total = photos.len();
    for (index, (message_id, media)) in photos.into_iter().enumerate() {
        let path = media_dir.photo_path(message_id);
        if !path.exists() {
            let partial = path.with_extension("jpg.part");
            private_file_options()
                .create(true)
                .truncate(true)
                .open(&partial)?;
            let downloadable = types::Downloadable::Media(media);
            while let Err(error) = client.download_media(&downloadable, &partial).await {
                let delay = error
                    .get_ref()
                    .and_then(|inner| inner.downcast_ref::<InvocationError>())
                    .and_then(flood_wait);
                wait_or_fail(delay, error).await.wrap_err_with(|| {
                    format!("failed to download photo of message {message_id}")
                })?;
            }
            std::fs::rename(&partial, &path)?;
        }
        let done = index + 1;
        if done % PHOTO_PROGRESS_EVERY == 0 || done == total {
            eprintln!("Downloaded {done}/{total} photos…");
        }
    }
    Ok(())
}

pub async fn export_chat(
    client: &Client,
    query: &str,
    out_path: &Path,
    file: File,
    media_dir: Option<&Path>,
) -> Result<ExportSummary> {
    let media_dir = match media_dir {
        Some(dir) => Some(MediaDir {
            dir: create_media_dir(dir)?,
            json_dir: out_path
                .canonicalize()?
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
        }),
        None => None,
    };
    let me = client.get_me().await?;
    let me_id = me.id();
    let me_sender = sender_fields(&types::Chat::User(me));

    let mut chats = Vec::new();
    let mut dialogs = client.iter_dialogs();
    loop {
        match dialogs.next().await {
            Ok(Some(dialog)) => {
                if !matches!(dialog.raw, tl::enums::Dialog::Folder(_)) {
                    chats.push(dialog.chat().clone());
                }
            }
            Ok(None) => break,
            Err(error) => wait_or_fail(flood_wait(&error), error).await?,
        }
    }
    let candidates: Vec<ChatCandidate> = chats
        .iter()
        .map(|chat| ChatCandidate {
            id: chat.id(),
            lookup_id: ChatKind::of(chat).lookup_id(chat.id()),
            name: display_name(chat),
        })
        .collect();
    let chat = &chats[resolve_chat_query(query, &candidates)?];
    let kind = ChatKind::of(chat);

    let mut messages = Vec::new();
    let mut photos = Vec::new();
    let mut history = client.iter_messages(chat);
    loop {
        match history.next().await {
            Ok(Some(message)) => {
                messages.push(ExportMessage::from_grammers(
                    &message,
                    &me_sender,
                    media_dir.as_ref(),
                ));
                if media_dir.is_some()
                    && let Some(media @ types::Media::Photo(_)) = message.media()
                    && media.to_raw_input_location().is_some()
                {
                    photos.push((message.id(), media));
                }
                if messages.len() % PROGRESS_EVERY == 0 {
                    eprintln!("Fetched {} messages…", messages.len());
                }
            }
            Ok(None) => break,
            Err(error) => wait_or_fail(flood_wait(&error), error).await?,
        }
    }
    messages.reverse();
    if let Some(media_dir) = &media_dir {
        photos.reverse();
        download_photos(client, photos, media_dir).await?;
    }

    let bot_api_chat_id = kind.bot_api_chat_id(chat.id(), me_id);
    let document = json!({
        "name": display_name(chat),
        "type": kind.export_type(),
        "id": chat.id(),
        "bot_api_chat_id": bot_api_chat_id,
        "messages": messages.iter().map(ExportMessage::to_json).collect::<Vec<_>>(),
    });
    write_json(file, &document)?;

    Ok(ExportSummary {
        message_count: messages.len(),
        chat_id: chat.id(),
        bot_api_chat_id,
    })
}

fn write_json(file: File, document: &Value) -> Result<()> {
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, document)?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(unix: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(unix, 0).unwrap()
    }

    fn message(text: &str) -> ExportMessage {
        ExportMessage {
            id: 7,
            date: at(1_772_359_200),
            from: Some(("Alice Doe".into(), "user42".into())),
            text: text.into(),
            ..Default::default()
        }
    }

    #[test]
    fn unedited_text_message_matches_desktop_fields() {
        assert_eq!(
            message("hi").to_json(),
            json!({
                "id": 7,
                "type": "message",
                "date": "2026-03-01T10:00:00",
                "date_unixtime": "1772359200",
                "from": "Alice Doe",
                "from_id": "user42",
                "text": "hi",
                "text_entities": [{"type": "plain", "text": "hi"}],
            })
        );
    }

    #[test]
    fn edited_message_includes_edit_dates_as_strings() {
        let value = ExportMessage {
            edited: Some(at(1_772_359_260)),
            ..message("hi")
        }
        .to_json();

        assert_eq!(value["edited"], "2026-03-01T10:01:00");
        assert_eq!(value["edited_unixtime"], "1772359260");
    }

    #[test]
    fn optional_fields_are_omitted_when_absent() {
        let value = ExportMessage {
            from: None,
            ..message("hi")
        }
        .to_json();

        for key in [
            "edited",
            "edited_unixtime",
            "from",
            "from_id",
            "reply_to_message_id",
            "photo",
            "file",
            "mime_type",
            "inline_bot_buttons",
        ] {
            assert!(value.get(key).is_none(), "{key} should be omitted");
        }
    }

    #[test]
    fn reply_id_is_a_number() {
        let value = ExportMessage {
            reply_to_message_id: Some(118),
            ..message("hi")
        }
        .to_json();

        assert_eq!(value["reply_to_message_id"], 118);
    }

    #[test]
    fn photo_and_document_media_use_placeholders() {
        let photo = ExportMessage {
            media: Some(ExportMedia::Photo(PHOTO_PLACEHOLDER.into())),
            ..message("")
        }
        .to_json();
        assert_eq!(photo["photo"], "(photo)");
        assert!(photo.get("file").is_none());

        let document = ExportMessage {
            media: Some(ExportMedia::Document {
                mime_type: Some("application/pdf".into()),
            }),
            ..message("")
        }
        .to_json();
        assert_eq!(document["file"], "(file)");
        assert_eq!(document["mime_type"], "application/pdf");

        let unknown_mime = ExportMessage {
            media: Some(ExportMedia::Document { mime_type: None }),
            ..message("")
        }
        .to_json();
        assert_eq!(unknown_mime["file"], "(file)");
        assert!(unknown_mime.get("mime_type").is_none());
    }

    #[test]
    fn empty_text_has_no_entities() {
        let value = message("").to_json();

        assert_eq!(value["text"], "");
        assert_eq!(value["text_entities"], json!([]));
    }

    #[test]
    fn inline_buttons_keep_rows() {
        let button = |kind, text: &str| InlineButton {
            kind,
            text: text.into(),
        };
        let value = ExportMessage {
            inline_buttons: vec![
                vec![button("callback", "Confirm"), button("callback", "Cancel")],
                vec![button("url", "Docs"), button("other", "Share")],
            ],
            ..message("Pick")
        }
        .to_json();

        assert_eq!(
            value["inline_bot_buttons"],
            json!([
                [{"type": "callback", "text": "Confirm"}, {"type": "callback", "text": "Cancel"}],
                [{"type": "url", "text": "Docs"}, {"type": "other", "text": "Share"}],
            ])
        );
    }

    #[test]
    fn inline_markup_is_converted_from_tl() {
        let markup = tl::enums::ReplyMarkup::ReplyInlineMarkup(tl::types::ReplyInlineMarkup {
            rows: vec![tl::enums::KeyboardButtonRow::Row(
                tl::types::KeyboardButtonRow {
                    buttons: vec![
                        tl::enums::KeyboardButton::Callback(tl::types::KeyboardButtonCallback {
                            requires_password: false,
                            text: "Confirm".into(),
                            data: b"ok".to_vec(),
                        }),
                        tl::enums::KeyboardButton::Url(tl::types::KeyboardButtonUrl {
                            text: "Docs".into(),
                            url: "https://example.com".into(),
                        }),
                        tl::enums::KeyboardButton::Game(tl::types::KeyboardButtonGame {
                            text: "Play".into(),
                        }),
                    ],
                },
            )],
        });

        assert_eq!(
            inline_buttons(Some(markup)),
            vec![vec![
                InlineButton {
                    kind: "callback",
                    text: "Confirm".into()
                },
                InlineButton {
                    kind: "url",
                    text: "Docs".into()
                },
                InlineButton {
                    kind: "other",
                    text: "Play".into()
                },
            ]]
        );
        assert!(inline_buttons(None).is_empty());
    }

    #[test]
    fn service_message_has_only_identity_and_date() {
        let value = ExportMessage {
            service: true,
            edited: Some(at(1_772_359_260)),
            reply_to_message_id: Some(1),
            ..message("joined")
        }
        .to_json();

        assert_eq!(
            value,
            json!({
                "id": 7,
                "type": "service",
                "date": "2026-03-01T10:00:00",
                "date_unixtime": "1772359200",
            })
        );
    }

    #[test]
    fn bot_api_chat_id_per_chat_kind() {
        let own_user_id = 555;
        assert_eq!(ChatKind::Personal.bot_api_chat_id(42, own_user_id), 555);
        assert_eq!(ChatKind::Bot.bot_api_chat_id(42, own_user_id), 555);
        assert_eq!(ChatKind::Group.bot_api_chat_id(42, own_user_id), -42);
        assert_eq!(
            ChatKind::Supergroup.bot_api_chat_id(1234567890, own_user_id),
            -1001234567890
        );
        assert_eq!(
            ChatKind::Channel.bot_api_chat_id(1234567890, own_user_id),
            -1001234567890
        );
    }

    #[test]
    fn lookup_id_is_user_id_for_private_chats_and_bot_api_id_otherwise() {
        assert_eq!(ChatKind::Bot.lookup_id(42), 42);
        assert_eq!(ChatKind::Group.lookup_id(42), -42);
        assert_eq!(ChatKind::Channel.lookup_id(42), -1000000000042);
    }

    #[test]
    fn megagroup_wrapped_as_group_is_a_supergroup() {
        let megagroup = types::Chat::from_raw(tl::enums::Chat::ChannelForbidden(
            tl::types::ChannelForbidden {
                broadcast: false,
                megagroup: true,
                id: 9,
                access_hash: 0,
                title: "Finance".into(),
                until_date: None,
            },
        ));
        assert!(matches!(megagroup, types::Chat::Group(_)));
        assert_eq!(ChatKind::of(&megagroup), ChatKind::Supergroup);
        assert_eq!(
            sender_fields(&megagroup),
            ("Finance".to_string(), "channel9".to_string())
        );

        let broadcast = types::Chat::from_raw(tl::enums::Chat::ChannelForbidden(
            tl::types::ChannelForbidden {
                broadcast: true,
                megagroup: false,
                id: 9,
                access_hash: 0,
                title: "News".into(),
                until_date: None,
            },
        ));
        assert_eq!(ChatKind::of(&broadcast), ChatKind::Channel);

        let basic = types::Chat::from_raw(tl::enums::Chat::Forbidden(tl::types::ChatForbidden {
            id: 9,
            title: "Family".into(),
        }));
        assert_eq!(ChatKind::of(&basic), ChatKind::Group);
    }

    #[test]
    fn export_type_and_peer_id_per_chat_kind() {
        assert_eq!(ChatKind::Personal.export_type(), "personal_chat");
        assert_eq!(ChatKind::Bot.export_type(), "bot_chat");
        assert_eq!(ChatKind::Group.export_type(), "private_group");
        assert_eq!(ChatKind::Supergroup.export_type(), "private_supergroup");
        assert_eq!(ChatKind::Channel.export_type(), "private_channel");
        assert_eq!(ChatKind::Bot.peer_id(1), "user1");
        assert_eq!(ChatKind::Group.peer_id(1), "chat1");
        assert_eq!(ChatKind::Supergroup.peer_id(1), "channel1");
    }

    fn chats() -> Vec<ChatCandidate> {
        let chat = |id, lookup_id, name: &str| ChatCandidate {
            id,
            lookup_id,
            name: name.into(),
        };
        vec![
            chat(100, 100, "Finance Bot"),
            chat(4242424242, -4242424242, "Expenses"),
            chat(300, -1000000000300, "Expenses family"),
            chat(400, -400, "Fitness"),
        ]
    }

    #[test]
    fn chat_query_matches_chat_id() {
        assert_eq!(resolve_chat_query("100", &chats()).unwrap(), 0);
        assert_eq!(resolve_chat_query("300", &chats()).unwrap(), 2);
    }

    #[test]
    fn chat_query_matches_negative_bot_api_id() {
        assert_eq!(resolve_chat_query("-4242424242", &chats()).unwrap(), 1);
        assert_eq!(resolve_chat_query("-1000000000300", &chats()).unwrap(), 2);
    }

    #[test]
    fn chat_query_exact_name_beats_substring() {
        assert_eq!(resolve_chat_query("expenses", &chats()).unwrap(), 1);
        assert_eq!(resolve_chat_query("family", &chats()).unwrap(), 2);
    }

    #[test]
    fn chat_query_lists_ambiguous_candidates_by_lookup_id() {
        let error = resolve_chat_query("fi", &chats()).unwrap_err().to_string();

        assert!(error.contains("100\tFinance Bot"), "{error}");
        assert!(error.contains("-400\tFitness"), "{error}");
        assert!(!error.contains("Expenses"), "{error}");
    }

    #[test]
    fn chat_query_without_match_fails() {
        let error = resolve_chat_query("nope", &chats())
            .unwrap_err()
            .to_string();

        assert_eq!(error, "no chat matches \"nope\"");
    }

    fn temp_output_path(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "dumbgram-export-test-{}-{name}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[tokio::test]
    async fn existing_output_file_is_never_overwritten() {
        let path = temp_output_path("overwrite");

        with_output_file(&path, async |file| write_json(file, &json!({"a": 1})))
            .await
            .unwrap();
        let second = with_output_file(&path, async |file| write_json(file, &json!({"a": 2})))
            .await
            .unwrap_err()
            .to_string();
        assert!(second.contains("refusing to overwrite"), "{second}");
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_file(&path).unwrap();

        assert_eq!(written, json!({"a": 1}));
    }

    #[test]
    fn photo_field_without_media_dir_is_placeholder() {
        assert_eq!(photo_field(5, None), "(photo)");
    }

    #[test]
    fn photo_field_is_relative_inside_json_dir_and_absolute_outside() {
        let media_dir = |dir: &str| MediaDir {
            dir: dir.into(),
            json_dir: "/exports".into(),
        };

        assert_eq!(
            photo_field(5, Some(&media_dir("/exports/photos"))),
            "photos/photo_5.jpg"
        );
        assert_eq!(
            photo_field(5, Some(&media_dir("/media"))),
            "/media/photo_5.jpg"
        );
        assert_eq!(
            photo_field(5, Some(&media_dir("/exports-photos"))),
            "/exports-photos/photo_5.jpg"
        );
    }

    #[test]
    fn media_dir_is_created_private_and_must_be_a_directory() {
        let dir = temp_output_path("media-dir");
        let _ = std::fs::remove_dir(&dir);

        let created = create_media_dir(&dir).unwrap();
        assert!(created.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        assert_eq!(create_media_dir(&dir).unwrap(), created);
        std::fs::remove_dir(&dir).unwrap();

        std::fs::write(&dir, b"not a dir").unwrap();
        assert!(create_media_dir(&dir).is_err());
        std::fs::remove_file(&dir).unwrap();
    }

    #[tokio::test]
    async fn error_after_creation_removes_output_file() {
        let path = temp_output_path("cleanup");

        let result: Result<()> = with_output_file(&path, async |_file| {
            assert!(path.exists());
            bail!("export failed")
        })
        .await;

        assert!(result.is_err());
        assert!(!path.exists());
    }
}
