//! Built-in music library: uploaded audio files and SoundCloud links.
//! Uploaded MP3/audio files are copied into ~/.scoutcode/music/files/ and listed alongside saved SoundCloud links
//! in ~/.scoutcode/music/library.json so the user's library persists even if original files move.

use crate::api::http;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use tauri::http::{Request, Response, header};

const MAX_AUDIO_BYTES: u64 = 150_000_000;
const AUDIO_EXTS: [&str; 5] = ["mp3", "m4a", "wav", "ogg", "flac"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MusicTrack {
    pub id: String,
    pub title: String,
    pub artist: String,
    /// "mp3" for uploaded audio files, "sc" for SoundCloud songs and playlists.
    pub kind: String,
    /// Filename inside ~/.scoutcode/music/files/ for uploaded audio files.
    pub file: Option<String>,
    /// SoundCloud page the entry was added from, linked from the player as SoundCloud asks.
    #[serde(default)]
    pub url: Option<String>,
    /// SoundCloud player source, e.g. https://api.soundcloud.com/tracks/293.
    #[serde(default)]
    pub embed: Option<String>,
    /// Artwork URL.
    pub thumbnail: Option<String>,
    /// Duration in seconds when known.
    pub duration: Option<f64>,
}

pub fn root() -> PathBuf {
    crate::config::home().join("music")
}

fn files_dir() -> PathBuf {
    root().join("files")
}

fn library_path() -> PathBuf {
    root().join("library.json")
}

pub fn list() -> Vec<MusicTrack> {
    let mut tracks = std::fs::read_to_string(library_path())
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<MusicTrack>>(&t).ok())
        .unwrap_or_default();
    // YouTube Music entries saved by an earlier version: YouTube no longer lets the app play them.
    tracks.retain(|t| t.kind != "ytm");
    tracks
}

fn save_list(tracks: &[MusicTrack]) -> Result<(), String> {
    std::fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(tracks).map_err(|e| e.to_string())?;
    std::fs::write(library_path(), text).map_err(|e| e.to_string())
}

/// Checks that a stored filename is a plain single file name without directory traversal.
fn safe_filename(name: &str) -> bool {
    let mut comps = Path::new(name).components();
    matches!(comps.next(), Some(Component::Normal(_))) && comps.next().is_none()
}

/// Parses basic ID3v2 title (`TIT2`) and artist (`TPE1`) frames when present at the start of an MP3 file.
fn parse_id3v2(bytes: &[u8]) -> (Option<String>, Option<String>) {
    if bytes.len() < 10 || &bytes[0..3] != b"ID3" {
        return (None, None);
    }
    let version = bytes[3];
    if version < 2 || version > 4 {
        return (None, None);
    }
    let tag_size = ((bytes[6] as usize & 0x7f) << 21)
        | ((bytes[7] as usize & 0x7f) << 14)
        | ((bytes[8] as usize & 0x7f) << 7)
        | (bytes[9] as usize & 0x7f);
    let end = (10 + tag_size).min(bytes.len());
    let mut pos = 10;
    let mut title = None;
    let mut artist = None;

    while pos + 10 <= end && (title.is_none() || artist.is_none()) {
        let frame_id = &bytes[pos..pos + 4];
        if frame_id.iter().all(|&b| b == 0) {
            break;
        }
        let size = if version == 4 {
            ((bytes[pos + 4] as usize & 0x7f) << 21)
                | ((bytes[pos + 5] as usize & 0x7f) << 14)
                | ((bytes[pos + 6] as usize & 0x7f) << 7)
                | (bytes[pos + 7] as usize & 0x7f)
        } else {
            u32::from_be_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]]) as usize
        };
        pos += 10;
        if size == 0 || pos + size > end {
            break;
        }
        let data = &bytes[pos..pos + size];
        pos += size;
        if frame_id == b"TIT2" {
            title = decode_id3_text(data);
        } else if frame_id == b"TPE1" {
            artist = decode_id3_text(data);
        }
    }
    (title, artist)
}

fn decode_id3_text(data: &[u8]) -> Option<String> {
    if data.is_empty() {
        return None;
    }
    let enc = data[0];
    let body = &data[1..];
    let s = match enc {
        0 => body.iter().map(|&b| b as char).collect::<String>(),
        1 | 2 => {
            let mut u16s = Vec::with_capacity(body.len() / 2);
            let mut chunks = body.chunks_exact(2);
            let mut le = enc == 1;
            if let Some(first) = chunks.next() {
                if first == [0xff, 0xfe] {
                    le = true;
                } else if first == [0xfe, 0xff] {
                    le = false;
                } else {
                    u16s.push(if le {
                        u16::from_le_bytes([first[0], first[1]])
                    } else {
                        u16::from_be_bytes([first[0], first[1]])
                    });
                }
            }
            for c in chunks {
                u16s.push(if le {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                });
            }
            String::from_utf16_lossy(&u16s)
        }
        _ => String::from_utf8_lossy(body).into_owned(),
    };
    let cleaned = s.trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// Extracts `(title, artist)` from a file path and optional ID3v2 header bytes.
fn metadata_for_file(path: &Path, header: &[u8]) -> (String, String) {
    let (id3_title, id3_artist) = parse_id3v2(header);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Untitled".into());
    let (stem_artist, stem_title) = if let Some((a, t)) = stem.split_once(" - ") {
        (Some(a.trim().to_string()), t.trim().to_string())
    } else {
        (None, stem.trim().to_string())
    };
    let title = id3_title.unwrap_or_else(|| if stem_title.is_empty() { "Untitled".into() } else { stem_title });
    let artist = id3_artist.or(stem_artist).unwrap_or_else(|| "Local MP3".into());
    (title, artist)
}

/// Opens a file picker to upload MP3 / audio files and copies them into ~/.scoutcode/music/files/.
pub fn upload_files() -> Result<Option<Vec<MusicTrack>>, String> {
    let picked = rfd::FileDialog::new()
        .set_title("Upload MP3 or audio files")
        .add_filter("Audio", &AUDIO_EXTS)
        .pick_files();
    let Some(paths) = picked else { return Ok(None) };
    if paths.is_empty() {
        return Ok(None);
    }
    std::fs::create_dir_all(files_dir()).map_err(|e| e.to_string())?;
    let mut tracks = list();
    let now = crate::session::now_ms();

    for (idx, path) in paths.iter().enumerate() {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| "mp3".into());
        if !AUDIO_EXTS.contains(&ext.as_str()) {
            return Err(format!("Unsupported audio format: .{ext}"));
        }
        let meta = std::fs::metadata(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
        if meta.len() > MAX_AUDIO_BYTES {
            return Err(format!("{} is larger than 150 MB.", path.display()));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
        let (title, artist) = metadata_for_file(path, &bytes);
        let id = format!("mp3-{now}-{idx}");
        let filename = format!("{id}.{ext}");
        let dest = files_dir().join(&filename);
        std::fs::write(&dest, &bytes).map_err(|e| format!("Could not save {}: {e}", dest.display()))?;
        tracks.push(MusicTrack {
            id,
            title,
            artist,
            kind: "mp3".into(),
            file: Some(filename),
            url: None,
            embed: None,
            thumbnail: None,
            duration: None,
        });
    }

    save_list(&tracks)?;
    Ok(Some(tracks))
}

/// Removes a track from the library and deletes its local audio file if present.
pub fn remove_track(id: &str) -> Result<Vec<MusicTrack>, String> {
    let mut tracks = list();
    if let Some(pos) = tracks.iter().position(|t| t.id == id) {
        let removed = tracks.remove(pos);
        if let Some(file) = removed.file.as_deref().filter(|f| safe_filename(f)) {
            let _ = std::fs::remove_file(files_dir().join(file));
        }
        save_list(&tracks)?;
    }
    Ok(tracks)
}

// ---------- SoundCloud ----------
// SoundCloud songs and playlists are added by link and play in SoundCloud's own embedded player (see Music.tsx), which
// needs no API key. Its oEmbed endpoint, also keyless, gives the title, uploader, artwork and the player's source.

/// What adding a link returns: the updated library and the entry the link resolved to.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Added {
    pub tracks: Vec<MusicTrack>,
    pub id: String,
    /// False when the link was already in the library.
    pub new: bool,
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?.split(':').next()?;
    Some(host.to_ascii_lowercase())
}

fn is_soundcloud_link(url: &str) -> bool {
    host_of(url).is_some_and(|h| h == "soundcloud.com" || h.ends_with(".soundcloud.com"))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Pulls the player's source (e.g. https://api.soundcloud.com/tracks/293) out of oEmbed's iframe HTML.
fn embed_source(html: &str) -> Option<String> {
    let src = html.split("src=\"").nth(1)?.split('"').next()?;
    let url = src.split(['?', '&']).find_map(|p| p.strip_prefix("url="))?;
    let url = percent_decode(url);
    url.starts_with("https://api.soundcloud.com/").then_some(url)
}

/// A stable library ID from the player source: "sc-tracks-293" or "sc-playlists-1234".
fn soundcloud_id(embed: &str) -> Option<String> {
    let mut parts = embed.trim_start_matches("https://api.soundcloud.com/").split(['/', '?']);
    let (kind, num) = (parts.next()?, parts.next()?);
    (matches!(kind, "tracks" | "playlists") && !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()))
        .then(|| format!("sc-{kind}-{num}"))
}

fn soundcloud_resolve(url: &str) -> Result<MusicTrack, String> {
    let res = http()
        .get("https://soundcloud.com/oembed")
        .query("format", "json")
        .query("url", url)
        .config()
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .call()
        .map_err(|e| format!("Could not reach SoundCloud: {e}"))?;
    match res.status().as_u16() {
        200 => {}
        403 | 404 => return Err("SoundCloud couldn't find that link. Check it's a public song or playlist.".into()),
        s => return Err(format!("SoundCloud returned HTTP {s}.")),
    }
    let bytes = res.into_body().with_config().limit(1_000_000).read_to_vec().map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| format!("Unexpected reply from SoundCloud: {e}"))?;
    let embed = v["html"].as_str().and_then(embed_source).ok_or("SoundCloud can't embed that link.")?;
    let id = soundcloud_id(&embed).ok_or("That link isn't a SoundCloud song or playlist.")?;
    let artist = v["author_name"].as_str().unwrap_or("SoundCloud").trim().to_string();
    let title = v["title"].as_str().unwrap_or("SoundCloud track").trim();
    // oEmbed titles read "Song by Uploader".
    let title = title.strip_suffix(&format!(" by {artist}")).unwrap_or(title).to_string();
    let page = url.split(['?', '#']).next().unwrap_or(url).to_string();
    Ok(MusicTrack {
        id,
        title,
        artist,
        kind: "sc".into(),
        file: None,
        url: Some(page),
        embed: Some(embed),
        thumbnail: v["thumbnail_url"].as_str().map(String::from),
        duration: None,
    })
}

/// Adds a SoundCloud song or playlist link to the library.
pub fn soundcloud_add(input: &str) -> Result<Added, String> {
    let input = input.trim();
    let url = if input.contains("://") { input.to_string() } else { format!("https://{input}") };
    if !is_soundcloud_link(&url) {
        return Err("Paste a SoundCloud link, like soundcloud.com/artist/song.".into());
    }
    let track = soundcloud_resolve(&url)?;
    let mut tracks = list();
    let id = track.id.clone();
    let new = !tracks.iter().any(|t| t.id == id);
    if new {
        tracks.push(track);
        save_list(&tracks)?;
    }
    Ok(Added { tracks, id, new })
}

// ---------- Streaming over the `music:` URI scheme ----------
// The player's <audio> element plays uploaded files from music://localhost/file/<trackId> (http://music.localhost/...
// on Windows). Requests are answered with byte ranges, so a long file starts at once and seeking reads only what it needs.

const FILE_CHUNK: u64 = 8 << 20;

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn mime_for_ext(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "m4a" | "mp4" | "aac" => "audio/mp4",
        "webm" => "audio/webm",
        "wav" => "audio/wav",
        "ogg" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        _ => "audio/mpeg",
    }
}

/// Parses `bytes=START-` or `bytes=START-END`.
fn parse_range(h: &str) -> Option<(u64, Option<u64>)> {
    let (start, end) = h.trim().strip_prefix("bytes=")?.split(',').next()?.split_once('-')?;
    let start = start.trim().parse().ok()?;
    let end = end.trim();
    Some((start, if end.is_empty() { None } else { Some(end.parse().ok()?) }))
}

type Served = Result<Response<Vec<u8>>, (u16, String)>;

/// Answers a request on the `music:` scheme.
pub fn serve(req: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let path = req.uri().path().trim_start_matches('/');
    let range = req.headers().get(header::RANGE).and_then(|v| v.to_str().ok()).and_then(parse_range);
    let out = match path.split_once('/') {
        Some(("file", id)) if valid_id(id) => serve_file(id, range),
        _ => Err((404, "Not found".into())),
    };
    out.unwrap_or_else(|(code, msg)| {
        Response::builder()
            .status(code)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(msg.into_bytes())
            .unwrap()
    })
}

fn audio_response(mime: &str) -> tauri::http::response::Builder {
    Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::CACHE_CONTROL, "no-store")
}

fn serve_file(id: &str, range: Option<(u64, Option<u64>)>) -> Served {
    let tracks = list();
    let track = tracks.iter().find(|t| t.id == id).ok_or((404, "Track not found.".to_string()))?;
    let file = track.file.as_deref().filter(|f| safe_filename(f)).ok_or((404, "This track has no local audio file.".to_string()))?;
    let mime = mime_for_ext(file.rsplit('.').next().unwrap_or(""));
    let mut f = std::fs::File::open(files_dir().join(file)).map_err(|e| (404, format!("Could not read audio file: {e}")))?;
    let len = f.metadata().map_err(|e| (500, e.to_string()))?.len();
    let Some((start, end)) = range else {
        let mut body = Vec::with_capacity(len as usize);
        f.read_to_end(&mut body).map_err(|e| (500, e.to_string()))?;
        return Ok(audio_response(mime).status(200).body(body).unwrap());
    };
    if start >= len {
        return Ok(audio_response(mime).status(416).header(header::CONTENT_RANGE, format!("bytes */{len}")).body(Vec::new()).unwrap());
    }
    let end = end.unwrap_or(u64::MAX).min(len - 1).min(start + FILE_CHUNK - 1);
    let mut body = vec![0; (end - start + 1) as usize];
    f.seek(SeekFrom::Start(start)).and_then(|_| f.read_exact(&mut body)).map_err(|e| (500, e.to_string()))?;
    Ok(audio_response(mime)
        .status(206)
        .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"))
        .body(body)
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_soundcloud_embed_source_and_id() {
        let html = r#"<iframe width="100%" src="https://w.soundcloud.com/player/?visual=true&url=https%3A%2F%2Fapi.soundcloud.com%2Ftracks%2F293&show_artwork=true"></iframe>"#;
        let embed = embed_source(html).unwrap();
        assert_eq!(embed, "https://api.soundcloud.com/tracks/293");
        assert_eq!(soundcloud_id(&embed).as_deref(), Some("sc-tracks-293"));
        assert_eq!(soundcloud_id("https://api.soundcloud.com/users/5"), None);
        assert!(is_soundcloud_link("https://on.soundcloud.com/AbC"));
        assert!(is_soundcloud_link("https://soundcloud.com/forss/flickermood"));
        assert!(!is_soundcloud_link("https://soundcloud.com.evil.example/x"));
    }

    #[test]
    fn parses_media_range_headers() {
        assert_eq!(parse_range("bytes=0-"), Some((0, None)));
        assert_eq!(parse_range("bytes=1048576-2097151"), Some((1048576, Some(2097151))));
        assert_eq!(parse_range("bytes=-500"), None);
        assert_eq!(parse_range("items=0-1"), None);
    }

    #[test]
    fn splits_artist_and_title_from_filename_when_no_id3() {
        let (title, artist) = metadata_for_file(Path::new("Daft Punk - Digital Love.mp3"), b"not-id3");
        assert_eq!(title, "Digital Love");
        assert_eq!(artist, "Daft Punk");

        let (title2, artist2) = metadata_for_file(Path::new("lofi_beat.mp3"), b"");
        assert_eq!(title2, "lofi_beat");
        assert_eq!(artist2, "Local MP3");
    }

    #[test]
    fn parses_id3v2_title_and_artist_frames() {
        // Construct a minimal ID3v2.3 tag with TIT2 ("Nightcall") and TPE1 ("Kavinsky").
        let mut tag = Vec::new();
        tag.extend_from_slice(b"ID3\x03\x00\x00\x00\x00\x00\x27");
        // TIT2 frame: 10 byte header + 1 encoding byte (0) + 9 bytes "Nightcall" = 20 bytes
        tag.extend_from_slice(b"TIT2\x00\x00\x00\x0a\x00\x00\x00Nightcall");
        // TPE1 frame: 10 byte header + 1 encoding byte (0) + 8 bytes "Kavinsky" = 19 bytes
        tag.extend_from_slice(b"TPE1\x00\x00\x00\x09\x00\x00\x00Kavinsky");

        let (title, artist) = metadata_for_file(Path::new("track.mp3"), &tag);
        assert_eq!(title, "Nightcall");
        assert_eq!(artist, "Kavinsky");
    }
}
