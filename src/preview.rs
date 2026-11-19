//! Previews of the selected file, made on a worker: images (thumbnailed through ImageMagick when
//! Rune can't decode them or they're large), video frames (ffmpegthumbnailer), PDF first pages
//! (pdftoppm), audio tags and cover art (ffprobe, ffmpeg), text and code, Markdown, archive
//! contents and folder sizes. Files on a server download to the cache first (small ones only);
//! archive members are extracted. Generated thumbnails are cached by path, size and date.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::model::{Entry, Location, extension, human_size};
use crate::ssh::Target;

/// Files on a server larger than this aren't downloaded just to preview them.
pub const REMOTE_LIMIT: u64 = 25_000_000;
/// Text read for a preview.
const TEXT_LIMIT: usize = 96 * 1024;

#[derive(Clone, Debug)]
pub enum Body {
    /// A picture: the image itself or a thumbnail, as a file Rune decodes (PNG, JPEG, SVG).
    Image(PathBuf),
    Text {
        text: String,
        lang: Option<&'static str>,
        truncated: bool,
    },
    Markdown(String),
    /// Top-level names inside an archive, and how many entries it has in all.
    Archive {
        names: Vec<(String, bool)>,
        total: usize,
    },
    /// What a folder holds.
    Folder {
        folders: usize,
        files: usize,
        bytes: u64,
    },
    /// No preview, and why.
    Nothing(String),
}

#[derive(Clone, Debug)]
pub struct Preview {
    pub loc: Location,
    pub name: String,
    pub body: Body,
    /// Extra facts: dimensions, duration, pages, tags, lines.
    pub facts: Vec<(&'static str, String)>,
}

/// Where the bytes are.
pub enum Source {
    Local(PathBuf),
    Remote(Target, String),
    /// A member of an archive on this machine, and where extracting it puts it.
    Member {
        archive: PathBuf,
        member: String,
        dir: PathBuf,
        extracted: PathBuf,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Image,
    Video,
    Audio,
    Pdf,
    Markdown,
    Text,
    Archive,
    Other,
}

pub fn kind_of(name: &str) -> Kind {
    let Some(ext) = extension(name) else {
        // Makefile, Dockerfile, LICENSE, README: sniffed as text.
        return Kind::Text;
    };
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "svgz" | "ico" | "tif" | "tiff" | "heic" | "heif" | "avif" | "jxl" | "psd" | "xcf"
        | "tga" => Kind::Image,
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "wmv" | "flv" | "mpg" | "mpeg" | "3gp" => Kind::Video,
        "mp3" | "flac" | "ogg" | "opus" | "m4a" | "aac" | "wav" | "wma" | "alac" | "aiff" | "ape" => Kind::Audio,
        "pdf" => Kind::Pdf,
        "md" | "markdown" | "mdx" => Kind::Markdown,
        _ if crate::archive::is_archive_name(name) => Kind::Archive,
        "txt" | "log" | "csv" | "tsv" | "json" | "jsonc" | "toml" | "yaml" | "yml" | "xml" | "html" | "htm" | "css" | "scss" | "js" | "mjs" | "cjs" | "ts"
        | "tsx" | "jsx" | "rs" | "py" | "go" | "c" | "h" | "cc" | "cpp" | "hpp" | "java" | "kt" | "swift" | "rb" | "php" | "lua" | "sh" | "bash" | "zsh"
        | "fish" | "sql" | "ini" | "conf" | "cfg" | "env" | "lock" | "gitignore" | "dockerfile" | "zig" | "nix" | "vue" | "svelte" | "cs" | "dart" | "ex"
        | "exs" | "erl" | "hs" | "ml" | "scala" | "r" | "pl" | "diff" | "patch" | "srt" | "vtt" | "tex" | "rst" | "org" | "service" | "desktop"
        | "properties" | "gradle" | "cmake" | "mk" => Kind::Text,
        _ => Kind::Other,
    }
}

/// The kind as a word, for the preview's caption.
pub fn kind_word(k: Kind) -> &'static str {
    match k {
        Kind::Image => "Image",
        Kind::Video => "Video",
        Kind::Audio => "Audio",
        Kind::Pdf => "PDF document",
        Kind::Markdown => "Markdown",
        Kind::Text => "Text",
        Kind::Archive => "Archive",
        Kind::Other => "File",
    }
}

/// Builds the preview of `e` at `loc`. Runs on a worker: may download, extract and run tools.
pub fn make(loc: Location, e: Entry, source: Source, cache: PathBuf) -> Preview {
    let name = e.name.clone();
    let mut facts = Vec::new();
    let body = build(&e, source, &cache, &mut facts).unwrap_or_else(Body::Nothing);
    Preview { loc, name, body, facts }
}

fn build(e: &Entry, source: Source, cache: &Path, facts: &mut Vec<(&'static str, String)>) -> Result<Body, String> {
    if e.is_dir() {
        return folder(source, facts);
    }
    let kind = kind_of(&e.name);
    if kind == Kind::Other && e.size.is_some_and(|s| s > 1_000_000) {
        return Err("No preview for this kind of file".into());
    }
    let path = fetch(e, source, cache)?;
    let thumbs = cache.join("thumbs");
    let thumb = |tag: &str| {
        let stamp =
            std::fs::metadata(&path).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        let key = crate::app::digest(&(path.clone(), e.size, stamp, tag.to_owned()));
        thumbs.join(format!("{key}.png"))
    };
    let _ = std::fs::create_dir_all(&thumbs);
    match kind {
        Kind::Image => {
            let ext = extension(&e.name).unwrap_or_default();
            let direct = matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "svg" | "svgz") && e.size.is_none_or(|s| s < 12_000_000);
            if let Some([w, h]) = magick_size(&path) {
                facts.push(("Dimensions", format!("{w} × {h}")));
            }
            if direct {
                return Ok(Body::Image(path));
            }
            let out = thumb("image");
            if !out.exists() {
                // `[0]`: the first frame of a GIF or layer of a PSD.
                let input = format!("{}[0]", path.display());
                run(Command::new("magick")
                    .arg(&input)
                    .args(["-auto-orient", "-thumbnail", "1600x1600>", "-background", "none"])
                    .arg(format!("png:{}", out.display())))
                .map_err(|_| "This image format can't be previewed".to_owned())?;
            }
            Ok(Body::Image(out))
        }
        Kind::Video => {
            probe(&path, facts);
            let out = thumb("video");
            if !out.exists() {
                run(Command::new("ffmpegthumbnailer").arg("-i").arg(&path).arg("-o").arg(&out).args(["-s", "960", "-q", "8", "-t", "10%"]))
                    .map_err(|_| "Couldn't read a frame of this video".to_owned())?;
            }
            Ok(Body::Image(out))
        }
        Kind::Audio => {
            probe(&path, facts);
            let out = thumb("cover");
            if !out.exists() {
                let _ = run(Command::new("ffmpeg")
                    .args(["-v", "error", "-y", "-i"])
                    .arg(&path)
                    .args(["-an", "-frames:v", "1", "-vf", "scale='min(800,iw)':-2"])
                    .arg(&out));
            }
            if out.exists() { Ok(Body::Image(out)) } else { Err("No cover art".into()) }
        }
        Kind::Pdf => {
            if let Ok(info) = output(Command::new("pdfinfo").arg(&path)) {
                for line in info.lines() {
                    if let Some(v) = line.strip_prefix("Pages:") {
                        facts.push(("Pages", v.trim().to_owned()));
                    } else if let Some(v) = line.strip_prefix("Title:").map(str::trim).filter(|v| !v.is_empty()) {
                        facts.push(("Title", v.to_owned()));
                    }
                }
            }
            let out = thumb("pdf");
            if !out.exists() {
                let base = out.with_extension("");
                run(Command::new("pdftoppm").args(["-png", "-singlefile", "-f", "1", "-l", "1", "-scale-to", "1200"]).arg(&path).arg(&base))
                    .map_err(|_| "Couldn't render this PDF".to_owned())?;
            }
            Ok(Body::Image(out))
        }
        Kind::Markdown => read_text(&path, facts).map(|(t, _)| Body::Markdown(t)),
        Kind::Text | Kind::Other => {
            let (text, truncated) = read_text(&path, facts)?;
            Ok(Body::Text { lang: rune_kit::syntax::lang_for_path(&e.name), text, truncated })
        }
        Kind::Archive => {
            let a = crate::archive::index(&path, 0, crate::app::now())?;
            let top = a.list("")?;
            let total = a.count();
            let mut names: Vec<(String, bool)> = top.into_iter().map(|e| (e.name.clone(), e.is_dir())).collect();
            names.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| crate::model::natural_cmp(&a.0, &b.0)));
            facts.push(("Contains", format!("{total} item{}", crate::app::plural(total))));
            Ok(Body::Archive { names, total })
        }
    }
}

/// The file on this machine: itself, a fresh cached download, or an extracted member.
fn fetch(e: &Entry, source: Source, cache: &Path) -> Result<PathBuf, String> {
    match source {
        Source::Local(p) => Ok(p),
        Source::Remote(t, path) => {
            let size = e.size.unwrap_or(0);
            if size > REMOTE_LIMIT {
                return Err(format!("{} on the server: open it to download a preview copy", human_size(size)));
            }
            let dest = crate::ops::open_cache_path(cache, &t, &path);
            if !crate::app::cache_fresh(&dest, e) {
                if let Some(d) = dest.parent() {
                    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
                }
                let mut cmd = t.scp();
                cmd.arg("--").arg(t.remote_arg(&path)).arg(&dest);
                run(&mut cmd).map_err(|err| format!("Couldn't download a preview: {err}"))?;
            }
            Ok(dest)
        }
        Source::Member { archive, member, dir, extracted } => {
            if e.size.is_some_and(|s| s > REMOTE_LIMIT * 4) {
                return Err("Too large to extract for a preview".into());
            }
            if !extracted.exists() {
                crate::jobs::run_steps(crate::archive::extract(&archive, &[member], &dir), &std::sync::atomic::AtomicBool::new(false))?;
            }
            Ok(extracted)
        }
    }
}

fn folder(source: Source, facts: &mut Vec<(&'static str, String)>) -> Result<Body, String> {
    let entries = match source {
        Source::Local(p) => crate::local::list(&p)?,
        Source::Remote(t, path) => crate::ssh::list(&t, &path).map_err(|e| e.message())?,
        Source::Member { .. } => return Err("Folder".into()),
    };
    let folders = entries.iter().filter(|e| e.is_dir()).count();
    let files = entries.len() - folders;
    let bytes = entries.iter().filter_map(|e| e.size).sum();
    facts.push(("Contains", format!("{} item{}", entries.len(), crate::app::plural(entries.len()))));
    Ok(Body::Folder { folders, files, bytes })
}

/// Up to `TEXT_LIMIT` bytes as text; an error for binary files.
fn read_text(path: &Path, facts: &mut Vec<(&'static str, String)>) -> Result<(String, bool), String> {
    use std::io::Read;
    let f = std::fs::File::open(path).map_err(|e| crate::local::plain_io(&e))?;
    let mut buf = Vec::with_capacity(TEXT_LIMIT);
    f.take(TEXT_LIMIT as u64 + 1).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let truncated = buf.len() > TEXT_LIMIT;
    buf.truncate(TEXT_LIMIT);
    if buf.contains(&0) {
        return Err("Binary file: no preview".into());
    }
    // A cut may split a character; drop the partial one rather than fail.
    let text = match String::from_utf8(buf) {
        Ok(t) => t,
        Err(e) => {
            let valid = e.utf8_error().valid_up_to();
            let bytes = e.into_bytes();
            if valid + 4 < bytes.len() {
                return Err("Not UTF-8 text: no preview".into());
            }
            String::from_utf8_lossy(&bytes[..valid]).into_owned()
        }
    };
    let lines = text.lines().count();
    facts.push(("Lines", if truncated { format!("{lines}+") } else { lines.to_string() }));
    Ok((text, truncated))
}

/// Duration, resolution and tags through ffprobe.
fn probe(path: &Path, facts: &mut Vec<(&'static str, String)>) {
    let Ok(out) = output(
        Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration,bit_rate:format_tags=title,artist,album,date:stream=codec_type,width,height,codec_name",
                "-of",
                "default=nw=1",
            ])
            .arg(path),
    ) else {
        return;
    };
    let get = |k: &str| out.lines().find_map(|l| l.strip_prefix(k)).map(str::trim).filter(|v| !v.is_empty() && *v != "N/A").map(str::to_owned);
    if let Some(d) = get("duration=").and_then(|d| d.parse::<f64>().ok()) {
        let s = d.round() as u64;
        facts.push(("Duration", if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }));
    }
    if let (Some(w), Some(h)) = (get("width="), get("height=")) {
        facts.push(("Resolution", format!("{w} × {h}")));
    }
    if let Some(c) = get("codec_name=") {
        facts.push(("Codec", c));
    }
    for (key, label) in [("TAG:title=", "Title"), ("TAG:artist=", "Artist"), ("TAG:album=", "Album"), ("TAG:date=", "Year")] {
        if let Some(v) = get(key) {
            facts.push((label, v));
        }
    }
    if let Some(b) = get("bit_rate=").and_then(|b| b.parse::<u64>().ok()) {
        facts.push(("Bit rate", format!("{} kb/s", b / 1000)));
    }
}

fn magick_size(path: &Path) -> Option<[u32; 2]> {
    let out = output(Command::new("magick").args(["identify", "-ping", "-format", "%w %h\n"]).arg(format!("{}[0]", path.display()))).ok()?;
    let mut it = out.lines().next()?.split(' ').filter_map(|v| v.parse().ok());
    Some([it.next()?, it.next()?])
}

fn run(cmd: &mut Command) -> Result<(), String> {
    output(cmd).map(|_| ())
}

fn output(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("failed").to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, size: u64) -> Entry {
        Entry { name: name.into(), kind: crate::model::Kind::File, link: false, size: Some(size), modified: None, mode: None }
    }

    #[test]
    fn kinds() {
        assert_eq!(kind_of("a.PNG"), Kind::Image);
        assert_eq!(kind_of("clip.mkv"), Kind::Video);
        assert_eq!(kind_of("song.flac"), Kind::Audio);
        assert_eq!(kind_of("main.rs"), Kind::Text);
        assert_eq!(kind_of("Makefile"), Kind::Text);
        assert_eq!(kind_of("x.tar.gz"), Kind::Archive);
        assert_eq!(kind_of("README.md"), Kind::Markdown);
        assert_eq!(kind_of("a.bin"), Kind::Other);
    }

    #[test]
    fn real_previews() {
        let d = std::env::temp_dir().join(format!("files-preview-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let cache = d.join("cache");
        let make_at = |name: &str| {
            let p = d.join(name);
            let size = std::fs::metadata(&p).unwrap().len();
            make(Location::Local(p.clone()), entry(name, size), Source::Local(p), cache.clone())
        };
        std::fs::write(d.join("main.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
        let p = make_at("main.rs");
        assert!(matches!(&p.body, Body::Text { lang: Some("rust"), truncated: false, .. }), "{:?}", p.body);
        std::fs::write(d.join("blob.bin"), [0u8, 1, 2, 3]).unwrap();
        assert!(matches!(make_at("blob.bin").body, Body::Nothing(_)));
        // Formats Rune can't decode become PNG thumbnails.
        if Command::new("magick").arg("-version").output().is_ok() {
            assert!(Command::new("magick").args(["-size", "64x48", "xc:red"]).arg(d.join("red.webp")).status().unwrap().success());
            let p = make_at("red.webp");
            let Body::Image(thumb) = &p.body else { panic!("{:?}", p.body) };
            assert!(thumb.extension().is_some_and(|e| e == "png") && thumb.exists());
            assert!(p.facts.contains(&("Dimensions", "64 × 48".into())));
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
