//! Files attached to a message. They go in after the text as `<file path="…">` blocks, the same as the old CLI's, so
//! its chats read back the same way. Skills named with @ go in the same way, as `<skill name="…">` blocks; those aren't
//! attachments, since the @name stays in the text and brings the skill back if the message is edited and sent again.
//! Images go in as image parts after the text, with a block of their own so they read back the same way.

use crate::api::Content;
use crate::tools;
use base64::Engine;
use image::{DynamicImage, GenericImageView, ImageFormat, ImageReader, Rgb, RgbImage};
use serde_json::{Value, json};
use std::io::Cursor;
use std::path::{Path, PathBuf};

/// Files this size or larger go in by name only; the model can read them with the view tool.
const MAX_BYTES: u64 = 300_000;
/// Entries listed for an attached folder.
const MAX_ENTRIES: usize = 200;
/// Images larger than this are refused rather than decoded.
const MAX_IMAGE_FILE: u64 = 50_000_000;
/// The longest side sent. Providers shrink to about this anyway, and some refuse much larger.
const IMAGE_EDGE: u32 = 1568;
/// The most sent for one image, which stays under the 5 MB some providers allow once it's base64.
const IMAGE_BYTES: usize = 3_500_000;

fn is_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"))
}

/// A JPEG of the image, on white where it's transparent, at the size it's sent.
fn to_jpeg(img: &DynamicImage) -> Result<Vec<u8>, String> {
    let (w, h) = img.dimensions();
    let small = if w.max(h) > IMAGE_EDGE { img.resize(IMAGE_EDGE, IMAGE_EDGE, image::imageops::FilterType::Triangle) } else { img.clone() };
    let rgba = small.to_rgba8();
    let flat = RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y).0;
        let over = |c: u8| ((c as u32 * p[3] as u32 + 255 * (255 - p[3] as u32)) / 255) as u8;
        Rgb([over(p[0]), over(p[1]), over(p[2])])
    });
    let mut out = vec![];
    for quality in [85, 60, 40] {
        out.clear();
        flat.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)).map_err(|e| e.to_string())?;
        if out.len() <= IMAGE_BYTES {
            break;
        }
    }
    Ok(out)
}

/// The image as a part of the message. Sent as it is when a provider will take it; otherwise shrunk.
fn image_part(path: &Path) -> Result<Value, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_IMAGE_FILE {
        return Err(format!("{} MB is too large", size / 1_000_000));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let reader = ImageReader::new(Cursor::new(&bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let format = reader.format();
    let img = reader.decode().map_err(|e| e.to_string())?;
    let (w, h) = img.dimensions();
    let mime = match format {
        Some(ImageFormat::Png) => Some("image/png"),
        Some(ImageFormat::Jpeg) => Some("image/jpeg"),
        Some(ImageFormat::Gif) => Some("image/gif"),
        Some(ImageFormat::WebP) => Some("image/webp"),
        _ => None,
    };
    let (mime, data) = match mime.filter(|_| w.max(h) <= IMAGE_EDGE && bytes.len() <= IMAGE_BYTES) {
        Some(mime) => (mime, bytes),
        None => ("image/jpeg", to_jpeg(&img)?),
    };
    let data = base64::engine::general_purpose::STANDARD.encode(data);
    Ok(json!({ "type": "image_url", "image_url": { "url": format!("data:{mime};base64,{data}") } }))
}

/// The message to send, and the files the agent may now read and edit without a view first: the ones whose contents it
/// carries, and larger ones it can read in parts, even from outside the project.
pub fn build(cwd: &str, text: &str, files: &[String]) -> (Content, Vec<PathBuf>) {
    let mut blocks = crate::skills::blocks(text);
    if files.is_empty() && blocks.is_empty() {
        return (text.into(), vec![]);
    }
    let mut read = vec![];
    let mut images = vec![];
    for f in files {
        let Ok(abs) = tools::resolve(cwd, f) else { continue };
        if abs.is_dir() {
            let mut names: Vec<String> = std::fs::read_dir(&abs)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    if e.file_type().is_ok_and(|t| t.is_dir()) { format!("{n}/") } else { n }
                })
                .collect();
            names.sort_by_key(|n| n.to_lowercase());
            let more = names.len().saturating_sub(MAX_ENTRIES);
            names.truncate(MAX_ENTRIES);
            if more > 0 {
                names.push(format!("… {more} more"));
            }
            blocks.push(format!("<folder path=\"{f}\">\n{}\n</folder>", names.join("\n")));
            continue;
        }
        if is_image(&abs) {
            let body = match image_part(&abs) {
                Ok(part) => {
                    images.push(part);
                    "(An image, attached to this message.)".to_string()
                }
                Err(e) => format!("(An image that could not be attached: {e})"),
            };
            blocks.push(format!("<file path=\"{f}\">\n{body}\n</file>"));
            continue;
        }
        let size = std::fs::metadata(&abs).map(|m| m.len()).unwrap_or(0);
        let body = if size >= MAX_BYTES {
            read.push(abs);
            format!("(Not included: {} KB is too large to attach. Read the parts you need with the view tool.)", size / 1000)
        } else {
            match std::fs::read(&abs) {
                Err(e) => format!("(Could not read it: {e})"),
                Ok(b) if b.iter().take(8000).any(|&c| c == 0) => "(A binary file, so its contents are not included.)".to_string(),
                Ok(b) => {
                    read.push(abs);
                    // An attached .env or config goes to the provider with its secrets taken out.
                    crate::redact::redact_owned(String::from_utf8_lossy(&b).replace("\r\n", "\n"))
                }
            }
        };
        blocks.push(format!("<file path=\"{f}\">\n{body}\n</file>"));
    }
    let written = format!("{text}\n\n{}", blocks.join("\n\n"));
    if images.is_empty() {
        return (written.into(), read);
    }
    let mut parts = vec![json!({ "type": "text", "text": written })];
    parts.extend(images);
    (Content::Parts(parts), read)
}

fn block_start(s: &str) -> Option<usize> {
    [s.find("\n\n<file path=\""), s.find("\n\n<folder path=\""), s.find("\n\n<skill name=\"")].into_iter().flatten().min()
}

/// The typed text and the attached paths of a message made by `build`.
pub fn split(content: &str) -> (String, Vec<String>) {
    let Some(start) = block_start(content) else { return (content.to_string(), vec![]) };
    let text = content[..start].to_string();
    let mut files = vec![];
    let mut rest = &content[start + 2..];
    loop {
        let tag = if rest.starts_with("<file path=\"") {
            "file"
        } else if rest.starts_with("<folder path=\"") {
            "folder"
        } else if rest.starts_with("<skill name=\"") {
            "skill"
        } else {
            break;
        };
        if tag != "skill" {
            let open = tag.len() + 8;
            let Some(q) = rest[open..].find('"') else { break };
            files.push(rest[open..open + q].to_string());
        }
        // A block ends at its closing tag when the next block or the end of the message follows; contents may
        // hold the same tag.
        let close = format!("\n</{tag}>");
        let mut from = 0;
        let mut next = None;
        while let Some(i) = rest[from..].find(&close) {
            let at = from + i + close.len();
            let after = &rest[at..];
            if after.is_empty() || block_start(after) == Some(0) {
                next = Some(at);
                break;
            }
            from = at;
        }
        match next {
            Some(at) if at < rest.len() => rest = &rest[at + 2..],
            _ => break,
        }
    }
    (text, files)
}

/// A path to show for a picked or dropped file: relative inside the project, absolute outside it.
pub fn display_path(cwd: &str, abs: &std::path::Path) -> String {
    tools::rel(cwd, abs)
}

/// The project's files, relative, for the @ picker. Skips the folders search skips and stops at `limit`.
pub fn index(cwd: &str, limit: usize) -> Vec<String> {
    let mut out = vec![];
    let cancel = std::sync::atomic::AtomicBool::new(false);
    tools::walk(std::path::Path::new(cwd), &cancel, |p| {
        out.push(tools::rel(cwd, p));
        out.len() < limit
    });
    out.sort_by_key(|p| (p.matches('/').count(), p.to_lowercase()));
    out
}

/// Files whose path contains every character of `q` in order, best first: name matches, then shorter paths.
pub fn matches<'a>(files: &'a [String], q: &str, n: usize) -> Vec<&'a String> {
    let q = q.to_lowercase();
    let mut hits: Vec<(u32, &String)> = files
        .iter()
        .filter_map(|f| {
            let lower = f.to_lowercase();
            let name = lower.rsplit('/').next().unwrap_or(&lower);
            let score = if q.is_empty() {
                0
            } else if name.starts_with(&q) {
                0
            } else if name.contains(&q) {
                1
            } else if lower.contains(&q) {
                2
            } else {
                let mut it = lower.chars();
                if !q.chars().all(|c| it.any(|x| x == c)) {
                    return None;
                }
                3
            };
            Some((score, f))
        })
        .collect();
    hits.sort_by_key(|(s, f)| (*s, f.len()));
    hits.into_iter().take(n).map(|(_, f)| f).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_and_splits() {
        let dir = std::env::temp_dir().join(format!("scout-attach-{}", crate::session::now_ms()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        // A line that looks like the closing tag must not end the block.
        std::fs::write(dir.join("src/a.rs"), "fn a() {}\r\n</file>\nmore\n").unwrap();
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let files = vec!["src/a.rs".to_string(), "b.txt".to_string(), "src".to_string()];
        let (msg, read) = build(&cwd, "look at these", &files);
        let msg = msg.text();
        assert_eq!(read.len(), 2);
        assert!(msg.contains("<file path=\"src/a.rs\">\nfn a() {}\n</file>\nmore\n\n</file>"));
        assert!(msg.contains("<folder path=\"src\">\na.rs\n</folder>"));
        assert_eq!(split(&msg), ("look at these".to_string(), files));
        assert_eq!(split("plain"), ("plain".to_string(), vec![]));
        // A skill's block isn't an attachment, and may hold what looks like its closing tag.
        let with_skill = "use @x\n\n<skill name=\"x\" path=\"p\">\nbody\n</skill>\nmore\n</skill>\n\n<file path=\"a\">\nb\n</file>";
        assert_eq!(split(with_skill), ("use @x".to_string(), vec!["a".to_string()]));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn decoded(content: &Content, at: usize) -> (String, DynamicImage) {
        let (mime, data) = content.images().remove(at);
        let bytes = base64::engine::general_purpose::STANDARD.decode(data).unwrap();
        (mime, image::load_from_memory(&bytes).unwrap())
    }

    #[test]
    fn images_go_in_as_images_and_read_back_as_attachments() {
        let dir = std::env::temp_dir().join(format!("scout-attach-img-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let cwd = dir.to_string_lossy().to_string();
        RgbImage::from_pixel(40, 20, Rgb([200, 10, 10])).save(dir.join("small.png")).unwrap();
        // A big one, and a format that isn't sent as it is.
        RgbImage::from_pixel(3200, 1600, Rgb([10, 200, 10])).save(dir.join("big.png")).unwrap();
        RgbImage::from_pixel(30, 30, Rgb([10, 10, 200])).save(dir.join("old.bmp")).unwrap();
        std::fs::write(dir.join("fake.png"), "not an image").unwrap();
        let files: Vec<String> = ["small.png", "big.png", "old.bmp", "fake.png"].iter().map(|s| s.to_string()).collect();
        let (content, read) = build(&cwd, "what's this?", &files);
        assert!(read.is_empty());
        assert_eq!(content.images().len(), 3);
        let (mime, small) = decoded(&content, 0);
        assert_eq!((mime.as_str(), small.dimensions()), ("image/png", (40, 20)));
        let (mime, big) = decoded(&content, 1);
        assert_eq!((mime.as_str(), big.dimensions()), ("image/jpeg", (1568, 784)));
        assert_eq!(decoded(&content, 2).0, "image/jpeg");
        let text = content.text();
        assert!(text.contains("<file path=\"fake.png\">\n(An image that could not be attached"), "{text}");
        assert!(!text.contains("base64"));
        assert_eq!(split(&text), ("what's this?".to_string(), files));
        // Sent as the OpenAI format has it, and saved so that it comes back the same.
        let saved: Content = serde_json::from_str(&serde_json::to_string(&content).unwrap()).unwrap();
        assert_eq!(saved.images().len(), 3);
        // A model that can't see them gets a note instead.
        assert!(content.without_images().images().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn large_files_can_be_read_in_parts_even_from_outside_the_project() {
        let base = std::env::temp_dir().join(format!("scout-attach-big-{}", crate::session::now_ms()));
        let (project, elsewhere) = (base.join("project"), base.join("elsewhere"));
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let big = elsewhere.join("big.log");
        std::fs::write(&big, "a line\n".repeat(60_000)).unwrap();
        let cwd = project.to_string_lossy().to_string();
        let path = big.to_string_lossy().to_string();
        let (cancel, mut read) = (std::sync::atomic::AtomicBool::new(false), std::collections::HashSet::new());
        let mut noop = |_: &str| {};
        let args = json!({ "path": path });
        // Not until it's attached.
        let mut ctx = tools::Ctx { cwd: &cwd, cancel: &cancel, read_files: &mut read, on_progress: &mut noop };
        assert!(tools::run("view", &args, &mut ctx).is_err());
        let (_, attached) = build(&cwd, "", std::slice::from_ref(&path));
        ctx.read_files.extend(attached);
        assert!(tools::run("view", &args, &mut ctx).unwrap().output.contains("a line"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn ranks_name_matches_first() {
        let files: Vec<String> = ["src/app.rs", "src/dock/apply.rs", "README.md", "gui/src/main.rs"].iter().map(|s| s.to_string()).collect();
        let m = matches(&files, "app", 10);
        assert_eq!(m[0], "src/app.rs");
        assert!(matches(&files, "rdm", 10).iter().any(|f| *f == "README.md"));
    }
}
