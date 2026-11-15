//! Free stock video, pictures and sound, searched online for the Stock panel.
//!
//! Four services can be searched (see `Source`), each turned on or off in Settings ▸ Stock:
//!
//! - Wikimedia Commons (video) and Openverse (pictures and sound, found across Flickr,
//!   Freesound, museums and more) need no account. Only work its maker has given to the public
//!   domain (Creative Commons Zero, "CC0") is listed from them: each search asks for CC0 work
//!   only, and every result's own licence is checked again here before it is shown.
//! - Pexels and Pixabay (video and pictures) need the person's own free API key. Their work is
//!   free to use commercially without credit, under each site's own licence rather than CC0.
//!
//! Results are kept safe for work. Each service's own filter is used where it has one
//! (Openverse leaves out work marked mature, Pixabay has a safe search), and on top of that
//! anything whose title, tags or categories mention nudity, sex or gore is dropped here (see
//! `is_unsafe`). Wikimedia Commons has no filter of its own and hosts explicit educational
//! media, so for it this check is the only one: it reads every file's categories, and a file
//! whose categories could not be read is left out rather than trusted. No word list catches
//! everything, so an unsuitable result is still possible, most of all from Commons.
//!
//! Every result says which service it came from, which both Pexels and Pixabay ask of apps
//! that show their results.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// What is being searched for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    #[default]
    Video,
    Image,
    Audio,
}

impl Kind {
    /// In the order of the Stock panel's tabs.
    pub const ALL: [Self; 3] = [Self::Video, Self::Image, Self::Audio];

    /// The tab's title.
    pub fn name(self) -> &'static str {
        match self {
            Self::Video => "Videos",
            Self::Image => "Images",
            Self::Audio => "Audio",
        }
    }

}

/// A service stock is searched on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Source {
    WikimediaCommons,
    Openverse,
    Pexels,
    Pixabay,
}

impl Source {
    /// In the order results are mixed and Settings lists them.
    pub const ALL: [Self; 4] = [Self::WikimediaCommons, Self::Openverse, Self::Pexels, Self::Pixabay];

    pub fn name(self) -> &'static str {
        match self {
            Self::WikimediaCommons => "Wikimedia Commons",
            Self::Openverse => "Openverse",
            Self::Pexels => "Pexels",
            Self::Pixabay => "Pixabay",
        }
    }

    /// What the service offers and on what terms, for Settings.
    pub fn about(self) -> &'static str {
        match self {
            Self::WikimediaCommons => "Video. Public domain (CC0) only. No account needed.",
            Self::Openverse => "Pictures and sound from Flickr, Freesound, museums and more. Public domain (CC0) only. No account needed.",
            Self::Pexels => "Video and pictures, free for commercial use without credit under the Pexels licence. Needs your free API key.",
            Self::Pixabay => "Video and pictures, free for commercial use without credit under the Pixabay licence. Needs your free API key.",
        }
    }

    /// The kinds of stock the service is searched for.
    pub fn kinds(self) -> &'static [Kind] {
        match self {
            Self::WikimediaCommons => &[Kind::Video],
            Self::Openverse => &[Kind::Image, Kind::Audio],
            Self::Pexels | Self::Pixabay => &[Kind::Video, Kind::Image],
        }
    }

    /// Whether the service only answers with an API key.
    pub fn needs_key(self) -> bool {
        matches!(self, Self::Pexels | Self::Pixabay)
    }

    /// Whether everything listed from the service is public domain (CC0).
    pub fn public_domain_only(self) -> bool {
        !self.needs_key()
    }

    /// Where a free API key for the service is made, for Settings.
    pub fn key_page(self) -> Option<&'static str> {
        match self {
            Self::Pexels => Some("pexels.com/api"),
            Self::Pixabay => Some("pixabay.com/api/docs"),
            Self::WikimediaCommons | Self::Openverse => None,
        }
    }
}

/// A service to search, with the person's API key for it (empty for the ones that need none).
pub type Access = (Source, String);

/// An API key as it is safe to send: letters, digits, `-` and `_` only, so nothing typed or
/// pasted into the key field can add to the request.
pub fn clean_key(key: &str) -> String {
    key.trim().chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).collect()
}

/// How many results one search page asks for (the most Openverse gives without an account).
pub const PAGE_SIZE: usize = 20;

/// The largest search reply read, so a broken reply can't fill memory.
const MAX_REPLY_BYTES: u64 = 4 << 20;

/// One thing found: enough to show it, download it and say where it came from.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StockItem {
    /// Unique within its kind; stable across searches. It starts with the service's name, so
    /// two services' numbering can't collide.
    pub id: String,
    pub kind: Kind,
    /// The service it was found on.
    pub source: Source,
    /// The site it came from, as shown on the result: the service's name, or for Openverse the
    /// site Openverse found it on ("Flickr via Openverse").
    pub origin: String,
    pub title: String,
    /// Who made it, when the source says. Shown for interest; no credit is required.
    pub creator: Option<String>,
    /// A small picture of it (none for most sound).
    pub thumbnail_url: Option<String>,
    /// The file itself.
    pub download_url: String,
    /// The file's extension, lowercase ("jpg", "webm", "mp3").
    pub extension: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_seconds: Option<f64>,
    pub bytes: Option<u64>,
}

impl StockItem {
    /// The name the downloaded file is saved under: the title made safe for a file name, plus
    /// the extension.
    pub fn file_name(&self) -> String {
        let cleaned: String = self.title.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { ' ' }).collect();
        let words: Vec<&str> = cleaned.split_whitespace().collect();
        let mut stem: String = words.join(" ").chars().take(60).collect();
        if stem.trim().is_empty() {
            stem = "Stock".into();
        }
        format!("{}.{}", stem.trim(), self.extension)
    }

    /// A short line of facts under the title: size, length and file size, whichever are known.
    pub fn summary(&self) -> String {
        let mut facts = Vec::new();
        if let (Some(width), Some(height)) = (self.width, self.height) {
            facts.push(format!("{width} × {height}"));
        }
        if let Some(seconds) = self.duration_seconds {
            let whole = seconds.round() as u64;
            facts.push(format!("{}:{:02}", whole / 60, whole % 60));
        }
        if let Some(bytes) = self.bytes {
            facts.push(if bytes >= 1 << 20 { format!("{:.1} MB", bytes as f64 / f64::from(1 << 20)) } else { format!("{} KB", (bytes / 1024).max(1)) });
        }
        facts.join(" · ")
    }
}

/// Word beginnings that mark a result as not safe for work: nudity, sex and gore. A word in a
/// title, tag or category that starts with one of these drops the result ("nud" covers nude,
/// nudity and nudist). Some harmless results are lost too ("breastfeeding"); that is the price
/// of erring on the safe side.
const UNSAFE_WORD_STARTS: [&str; 48] = [
    "nud", "naked", "porn", "erotic", "masturbat", "ejaculat", "genital", "penis", "penile", "vulva", "vagina", "fellatio", "cunnilingus", "coitus", "intercourse", "orgasm",
    "cumshot", "semen", "erection", "fetish", "bdsm", "bondage", "topless", "bottomless", "breast", "nipple", "buttock", "defecat", "urinat", "feces", "striptease", "stripper",
    "lingerie", "nsfw", "hentai", "dildo", "vibrator", "prostitut", "brothel", "scrotum", "testic", "clitoris", "labia", "pubic", "gore", "corpse", "behead", "mutilat",
];

/// Whole words that mark a result as not safe for work. These are matched exactly, because
/// they begin harmless words too ("sex" begins "sextant", "anal" begins "analysis").
const UNSAFE_WORDS: [&str; 12] = ["sex", "sexual", "sexually", "sexuality", "sexy", "anal", "anus", "erect", "xxx", "cum", "autopsy", "suicide"];

/// Whether any of `texts` (titles, tags, category names) mentions nudity, sex or gore.
pub fn is_unsafe<'a>(texts: impl IntoIterator<Item = &'a str>) -> bool {
    texts.into_iter().any(|text| {
        text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty()).any(|word| UNSAFE_WORDS.contains(&word) || UNSAFE_WORD_STARTS.iter().any(|start| word.starts_with(start)))
    })
}

/// What a Commons search is told to leave out, as search words. This thins explicit files out
/// of the results before they are fetched; `is_unsafe` still checks every one that comes back.
const COMMONS_EXCLUDED: &str = "-nude -nudity -naked -porn -sex -sexual -masturbation -ejaculation -penis -genitalia -vulva -erotic";

/// `text` made safe to put in a URL's query: everything but letters, digits and `-_.~` is
/// written as `%XX`.
fn url_encode(text: &str) -> String {
    text.bytes().map(|byte| if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) { char::from(byte).to_string() } else { format!("%{byte:02X}") }).collect()
}

/// The address that asks `source` for `kind` matching `query`, page `page` (the first is 0),
/// or `None` when the service has no such kind. Commons and Openverse are asked for CC0 work
/// only. Pixabay takes its key in the address; `key` is ignored by the others.
pub fn search_url(source: Source, kind: Kind, query: &str, page: usize, key: &str) -> Option<String> {
    if !source.kinds().contains(&kind) {
        return None;
    }
    let query = url_encode(query.trim());
    let number = page + 1;
    Some(match (source, kind) {
        (Source::Openverse, _) => {
            let media = if kind == Kind::Image { "images" } else { "audio" };
            format!("https://api.openverse.org/v1/{media}/?q={query}&license=cc0&mature=false&page_size={PAGE_SIZE}&page={number}")
        }
        // Commons marks each file's licence as a statement (P275); Q6938433 is CC0.
        (Source::WikimediaCommons, _) => format!(
            "https://commons.wikimedia.org/w/api.php?action=query&format=json&generator=search&gsrnamespace=6&gsrlimit={PAGE_SIZE}&gsroffset={}&gsrsearch={query}%20{}%20filetype%3Avideo%20haswbstatement%3AP275%3DQ6938433&prop=imageinfo%7Ccategories&cllimit=max&clshow=!hidden&iiprop=url%7Csize%7Cuser%7Cextmetadata&iiurlwidth=320&iiextmetadatafilter=LicenseShortName%7CObjectName",
            page * PAGE_SIZE,
            url_encode(COMMONS_EXCLUDED)
        ),
        (Source::Pexels, Kind::Video) => format!("https://api.pexels.com/videos/search?query={query}&per_page={PAGE_SIZE}&page={number}"),
        (Source::Pexels, _) => format!("https://api.pexels.com/v1/search?query={query}&per_page={PAGE_SIZE}&page={number}"),
        (Source::Pixabay, Kind::Video) => format!("https://pixabay.com/api/videos/?key={}&q={query}&safesearch=true&per_page={PAGE_SIZE}&page={number}", clean_key(key)),
        (Source::Pixabay, _) => format!("https://pixabay.com/api/?key={}&q={query}&image_type=photo&safesearch=true&per_page={PAGE_SIZE}&page={number}", clean_key(key)),
    })
}

/// The request header that carries the key, for the services that take it that way.
fn key_header(source: Source, key: &str) -> Option<String> {
    (source == Source::Pexels).then(|| format!("Authorization: {}", clean_key(key)))
}

/// A web address without its query ("?…"), which Commons adds for its own counting.
fn without_query(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

/// The lowercase extension of the file a web address points at, if it has a sensible one.
fn extension_of(url: &str) -> Option<String> {
    let name = without_query(url).rsplit('/').next()?;
    let (_, extension) = name.rsplit_once('.')?;
    ((1..=5).contains(&extension.len()) && extension.chars().all(|c| c.is_ascii_alphanumeric())).then(|| extension.to_ascii_lowercase())
}

/// A text field of a JSON object, if it is there and not empty.
fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::trim).filter(|text| !text.is_empty()).map(str::to_owned)
}

/// A whole-number field of a JSON object, if it is there and fits.
fn number<T: TryFrom<u64>>(value: &Value, key: &str) -> Option<T> {
    value.get(key)?.as_u64().and_then(|number| T::try_from(number).ok())
}

/// One Openverse result as a stock item; `None` unless it is CC0 and has a usable file.
fn openverse_item(kind: Kind, result: &Value) -> Option<StockItem> {
    if text(result, "license")?.to_ascii_lowercase() != "cc0" {
        return None;
    }
    // Openverse marks work its sources or its own checks found sensitive.
    let marked = result.get("mature").and_then(Value::as_bool).unwrap_or(false) || result.get("unstable__sensitivity").and_then(Value::as_array).is_some_and(|marks| !marks.is_empty());
    let tags = result.get("tags").and_then(Value::as_array).map(|tags| tags.iter().filter_map(|tag| tag.get("name")?.as_str()).collect::<Vec<_>>()).unwrap_or_default();
    let title = text(result, "title");
    if marked || is_unsafe(tags.into_iter().chain(title.as_deref())) {
        return None;
    }
    let download_url = text(result, "url")?;
    let extension = text(result, "filetype").map(|kind| kind.to_ascii_lowercase()).or_else(|| extension_of(&download_url))?;
    // Openverse names the site it found the work on ("flickr"); that is where it came from.
    let origin = match text(result, "source").or_else(|| text(result, "provider")) {
        Some(site) => format!("{} via Openverse", capitalized(&site)),
        None => Source::Openverse.name().to_owned(),
    };
    Some(StockItem {
        id: format!("openverse:{}", text(result, "id")?),
        kind,
        source: Source::Openverse,
        origin,
        title: title.unwrap_or_else(|| "Untitled".into()),
        creator: text(result, "creator"),
        thumbnail_url: text(result, "thumbnail"),
        download_url,
        extension: tidy_extension(extension),
        width: number(result, "width"),
        height: number(result, "height"),
        // Openverse gives a sound's length in milliseconds.
        duration_seconds: result.get("duration").and_then(Value::as_f64).map(|milliseconds| milliseconds / 1000.0),
        bytes: number(result, "filesize"),
    })
}

/// One Wikimedia Commons page as a stock item; `None` unless it is CC0, has a usable file and
/// is safe for work going by its title and categories. `categories_complete` says whether the
/// reply carried every page's categories; when it didn't, a page without any is left out,
/// since nothing vouches for it.
fn commons_item(page: &Value, categories_complete: bool) -> Option<StockItem> {
    let categories: Vec<&str> = page.get("categories").and_then(Value::as_array).map(|list| list.iter().filter_map(|category| category.get("title")?.as_str()).collect()).unwrap_or_default();
    if (categories.is_empty() && !categories_complete) || is_unsafe(categories.into_iter().chain(page.get("title").and_then(Value::as_str))) {
        return None;
    }
    let info = page.get("imageinfo")?.get(0)?;
    let metadata = info.get("extmetadata")?;
    let licence = metadata.get("LicenseShortName").and_then(|field| text(field, "value"))?;
    if !matches!(licence.to_ascii_lowercase().as_str(), "cc0" | "cc0 1.0") {
        return None;
    }
    let download_url = without_query(&text(info, "url")?).to_owned();
    let extension = extension_of(&download_url)?;
    // The page title is "File:Name.webm"; the name without those ends reads better.
    let fallback = text(page, "title").map(|title| title.trim_start_matches("File:").rsplit_once('.').map_or(title.clone(), |(name, _)| name.to_owned()));
    let title = metadata.get("ObjectName").and_then(|field| text(field, "value")).or(fallback).unwrap_or_else(|| "Untitled".into());
    if is_unsafe([title.as_str()]) {
        return None;
    }
    Some(StockItem {
        id: format!("commons:{}", page.get("pageid")?.as_u64()?),
        kind: Kind::Video,
        source: Source::WikimediaCommons,
        origin: Source::WikimediaCommons.name().to_owned(),
        title,
        creator: text(info, "user"),
        thumbnail_url: text(info, "thumburl"),
        download_url,
        extension,
        width: number(info, "width"),
        height: number(info, "height"),
        duration_seconds: info.get("duration").and_then(Value::as_f64),
        bytes: number(info, "size"),
    })
}

/// `text` with its first letter made a capital ("flickr" reads "Flickr").
fn capitalized(text: &str) -> String {
    let mut letters = text.chars();
    letters.next().map_or_else(String::new, |first| first.to_uppercase().chain(letters).collect())
}

/// A file's extension as files are usually named: lowercase, with "jpeg" written "jpg".
fn tidy_extension(extension: String) -> String {
    if extension == "jpeg" { "jpg".into() } else { extension }
}

/// One Pexels photo as a stock item; `None` when it has no usable file.
fn pexels_photo(photo: &Value) -> Option<StockItem> {
    let files = photo.get("src")?;
    let download_url = text(files, "original")?;
    let creator = text(photo, "photographer");
    if is_unsafe(text(photo, "alt").as_deref()) {
        return None;
    }
    Some(StockItem {
        id: format!("pexels:{}", photo.get("id")?.as_u64()?),
        kind: Kind::Image,
        source: Source::Pexels,
        origin: Source::Pexels.name().to_owned(),
        // Pexels describes a photo in "alt"; some have none.
        title: text(photo, "alt").or_else(|| creator.as_ref().map(|name| format!("Photo by {name}"))).unwrap_or_else(|| "Untitled".into()),
        creator,
        thumbnail_url: text(files, "medium").or_else(|| text(files, "tiny")),
        extension: tidy_extension(extension_of(&download_url)?),
        download_url,
        width: number(photo, "width"),
        height: number(photo, "height"),
        duration_seconds: None,
        bytes: None,
    })
}

/// The widest picture a downloaded stock video should be: full HD. Larger files (4K) are many
/// times the size for little gain in most edits.
const PREFERRED_VIDEO_WIDTH: u64 = 1920;

/// Of a video's files, the widest one no wider than full HD, or failing that the narrowest
/// of the larger ones.
fn preferred_video_file<'a>(files: &[&'a Value]) -> Option<&'a Value> {
    let width = |file: &Value| number::<u64>(file, "width").unwrap_or(0);
    files.iter().copied().filter(|file| width(file) <= PREFERRED_VIDEO_WIDTH).max_by_key(|file| width(file)).or_else(|| files.iter().copied().min_by_key(|file| width(file)))
}

/// One Pexels video as a stock item; `None` when it has no MP4 file.
fn pexels_video(video: &Value) -> Option<StockItem> {
    // Each video comes in several sizes, and as a streaming playlist that can't be imported.
    let files: Vec<&Value> = video.get("video_files")?.as_array()?.iter().filter(|file| text(file, "file_type").as_deref() == Some("video/mp4") && text(file, "link").is_some()).collect();
    let file = preferred_video_file(&files)?;
    let creator = video.get("user").and_then(|user| text(user, "name"));
    // Pexels gives videos no title; the page address ends in one ("…/video/waves-on-a-beach-1234/").
    let from_address = text(video, "url").and_then(|url| {
        let slug = url.trim_end_matches('/').rsplit('/').next()?.to_owned();
        let words: Vec<&str> = slug.split('-').filter(|word| !word.is_empty() && !word.chars().all(|c| c.is_ascii_digit())).collect();
        (!words.is_empty()).then(|| capitalized(&words.join(" ")))
    });
    if is_unsafe(from_address.as_deref()) {
        return None;
    }
    Some(StockItem {
        id: format!("pexels:{}", video.get("id")?.as_u64()?),
        kind: Kind::Video,
        source: Source::Pexels,
        origin: Source::Pexels.name().to_owned(),
        title: from_address.or_else(|| creator.as_ref().map(|name| format!("Video by {name}"))).unwrap_or_else(|| "Untitled".into()),
        creator,
        thumbnail_url: text(video, "image"),
        download_url: text(file, "link")?,
        extension: "mp4".into(),
        width: number(file, "width"),
        height: number(file, "height"),
        duration_seconds: video.get("duration").and_then(Value::as_f64),
        bytes: None,
    })
}

/// A Pixabay result's title: its first few tags ("flower, yellow, nature" reads "Flower, yellow").
fn pixabay_title(hit: &Value) -> String {
    let tags = text(hit, "tags").unwrap_or_default();
    let first: Vec<&str> = tags.split(',').map(str::trim).filter(|tag| !tag.is_empty()).take(2).collect();
    if first.is_empty() { "Untitled".into() } else { capitalized(&first.join(", ")) }
}

/// The longest side of the picture Pixabay gives without special approval.
const PIXABAY_LARGE_SIDE: f64 = 1280.0;

/// One Pixabay picture as a stock item; `None` when it has no usable file.
fn pixabay_image(hit: &Value) -> Option<StockItem> {
    if is_unsafe(text(hit, "tags").as_deref()) {
        return None;
    }
    let download_url = text(hit, "largeImageURL")?;
    // The file is the original scaled to fit 1280 pixels; its size follows from the original's.
    let scaled = match (number::<u32>(hit, "imageWidth"), number::<u32>(hit, "imageHeight")) {
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            let scale = (PIXABAY_LARGE_SIDE / f64::from(width.max(height))).min(1.0);
            Some(((f64::from(width) * scale).round() as u32, (f64::from(height) * scale).round() as u32))
        }
        _ => None,
    };
    Some(StockItem {
        id: format!("pixabay:{}", hit.get("id")?.as_u64()?),
        kind: Kind::Image,
        source: Source::Pixabay,
        origin: Source::Pixabay.name().to_owned(),
        title: pixabay_title(hit),
        creator: text(hit, "user"),
        thumbnail_url: text(hit, "webformatURL").or_else(|| text(hit, "previewURL")),
        extension: tidy_extension(extension_of(&download_url)?),
        download_url,
        width: scaled.map(|(width, _)| width),
        height: scaled.map(|(_, height)| height),
        duration_seconds: None,
        bytes: None,
    })
}

/// One Pixabay video as a stock item; `None` when it has no usable file.
fn pixabay_video(hit: &Value) -> Option<StockItem> {
    if is_unsafe(text(hit, "tags").as_deref()) {
        return None;
    }
    // Each video comes in up to four sizes; a size Pixabay doesn't have has an empty address.
    let sizes = hit.get("videos")?;
    let files: Vec<&Value> = ["large", "medium", "small", "tiny"].iter().filter_map(|size| sizes.get(size)).filter(|file| text(file, "url").is_some()).collect();
    let file = preferred_video_file(&files)?;
    let download_url = text(file, "url")?;
    Some(StockItem {
        id: format!("pixabay:{}", hit.get("id")?.as_u64()?),
        kind: Kind::Video,
        source: Source::Pixabay,
        origin: Source::Pixabay.name().to_owned(),
        title: pixabay_title(hit),
        creator: text(hit, "user"),
        thumbnail_url: text(file, "thumbnail"),
        extension: extension_of(&download_url).unwrap_or_else(|| "mp4".into()),
        download_url,
        width: number(file, "width"),
        height: number(file, "height"),
        duration_seconds: hit.get("duration").and_then(Value::as_f64),
        bytes: number(file, "size"),
    })
}

/// The stock items in one service's search reply, in the service's order, and whether it has
/// more pages after `page` (the first is 0).
pub fn parse(source: Source, kind: Kind, reply: &str, page: usize) -> Result<(Vec<StockItem>, bool), String> {
    let unreadable = || format!("{} sent a reply that could not be read.", source.name());
    let reply: Value = serde_json::from_str(reply).map_err(|_| unreadable())?;
    let list = |key: &str| reply.get(key).and_then(Value::as_array).ok_or_else(unreadable);
    match source {
        Source::Openverse => {
            let more = match (number::<u64>(&reply, "page"), number::<u64>(&reply, "page_count")) {
                (Some(page), Some(pages)) => page < pages,
                _ => false,
            };
            Ok((list("results")?.iter().filter_map(|result| openverse_item(kind, result)).collect(), more))
        }
        Source::WikimediaCommons => {
            // A search that finds nothing has no "query" at all.
            let Some(pages) = reply.get("query").and_then(|query| query.get("pages")).and_then(Value::as_object) else {
                return Ok((Vec::new(), false));
            };
            // Pages come keyed by id; "index" is each one's place in the results.
            // When the categories didn't all fit in one reply, Commons says where they would go on.
            let categories_complete = reply.get("continue").and_then(|more| more.get("clcontinue")).is_none();
            let mut found: Vec<(u64, StockItem)> = pages.values().filter_map(|page| Some((page.get("index").and_then(Value::as_u64).unwrap_or(u64::MAX), commons_item(page, categories_complete)?))).collect();
            found.sort_by_key(|(index, _)| *index);
            let more = reply.get("continue").and_then(|more| more.get("gsroffset")).is_some();
            Ok((found.into_iter().map(|(_, item)| item).collect(), more))
        }
        Source::Pexels => {
            let items = match kind {
                Kind::Video => list("videos")?.iter().filter_map(pexels_video).collect(),
                _ => list("photos")?.iter().filter_map(pexels_photo).collect(),
            };
            // Pexels names the next page only when there is one.
            Ok((items, text(&reply, "next_page").is_some()))
        }
        Source::Pixabay => {
            let items = match kind {
                Kind::Video => list("hits")?.iter().filter_map(pixabay_video).collect(),
                _ => list("hits")?.iter().filter_map(pixabay_image).collect(),
            };
            let reachable = number::<u64>(&reply, "totalHits").unwrap_or(0);
            Ok((items, reachable > ((page + 1) * PAGE_SIZE) as u64))
        }
    }
}

/// One page of a search across several services.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    /// What was found, the services' results taken in turn so no one service fills the top.
    pub items: Vec<StockItem>,
    /// Whether any service has more after this page.
    pub more: bool,
    /// A line for each service that could not be searched, to show under the results.
    pub problems: Vec<String>,
}

/// Asks one service for a page of `kind` matching `query`.
fn search_one(source: Source, key: &str, kind: Kind, query: &str, page: usize) -> Result<(Vec<StockItem>, bool), String> {
    let Some(url) = search_url(source, kind, query, page, key) else { return Ok((Vec::new(), false)) };
    let header = key_header(source, key);
    let reply = crate::download::fetch_text_privately(&url, header.as_deref(), MAX_REPLY_BYTES).map_err(|error| {
        // A refused key is the usual reason a service that needs one says no.
        if source.needs_key() && ["400", "401", "403"].iter().any(|code| error.contains(code)) {
            format!("{} did not accept the API key. Check it in Settings ▸ Stock.", source.name())
        } else if error.contains("429") {
            format!("{} is asking to slow down. Wait a minute and search again.", source.name())
        } else {
            format!("{} could not be searched. Check the internet connection and try again.", source.name())
        }
    })?;
    parse(source, kind, &reply, page)
}

/// Searches every service in `access` that has `kind`, all at once, and mixes what they
/// found. Run it off the main thread: it waits on the network.
pub fn search(kind: Kind, query: &str, page: usize, access: &[Access]) -> Page {
    let wanted: Vec<&Access> = access.iter().filter(|(source, _)| source.kinds().contains(&kind)).collect();
    let replies: Vec<Result<(Vec<StockItem>, bool), String>> = std::thread::scope(|scope| {
        let asked: Vec<_> = wanted.iter().map(|(source, key)| scope.spawn(move || search_one(*source, key, kind, query, page))).collect();
        asked.into_iter().map(|reply| reply.join().unwrap_or_else(|_| Err("A stock search stopped unexpectedly.".into()))).collect()
    });
    let mut found = Page::default();
    let mut lists = Vec::new();
    for reply in replies {
        match reply {
            Ok((items, more)) => {
                found.more |= more;
                lists.push(items.into_iter());
            }
            Err(problem) => found.problems.push(problem),
        }
    }
    // One from each service in turn, until all are used up.
    loop {
        let before = found.items.len();
        found.items.extend(lists.iter_mut().filter_map(Iterator::next));
        if found.items.len() == before {
            break;
        }
    }
    found
}

/// Where a thumbnail is kept once fetched: the system's temporary folder, which the system
/// clears by itself, under a name made from its address.
pub fn thumbnail_path(url: &str) -> PathBuf {
    use sha2::Digest as _;
    let digest: String = sha2::Sha256::digest(url.as_bytes()).iter().take(12).map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join("studio-stock-thumbnails").join(format!("{digest}.jpg"))
}

/// Fetches an item's thumbnail if it isn't already on disk, and says where it is.
pub fn fetch_thumbnail(url: &str) -> Result<PathBuf, String> {
    let path = thumbnail_path(url);
    if path.exists() {
        return Ok(path);
    }
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    crate::download::fetch_file(url, &path, "the preview picture", &|| false).map(|()| path)
}

/// The stock a person keeps to hand: the items they starred, and the ones they added to a
/// project most recently (newest first). Each keeps everything needed to add it again and to
/// say which site it came from, which is the record to check if a licence is ever questioned.
/// Kept in a file of its own in Studio's settings folder.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Shelf {
    pub favorites: Vec<StockItem>,
    pub recents: Vec<StockItem>,
}

impl Shelf {
    /// The most recently added items remembered.
    pub const MAX_RECENTS: usize = 40;

    fn path() -> PathBuf {
        crate::app_settings::folder().join("stock-shelf.json")
    }

    /// The saved shelf, or an empty one when there is none or it can't be read.
    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    /// Saves the shelf. Failing to only loses what is remembered, so it is reported on the
    /// terminal rather than interrupting work.
    pub fn save(&self) {
        let path = Self::path();
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| serde_json::to_string_pretty(self).map_err(std::io::Error::other))
            .and_then(|text| crate::app_settings::write_atomically(&path, &text));
        if let Err(error) = written {
            eprintln!("could not save the stock favourites: {error}");
        }
    }

    pub fn is_favorite(&self, id: &str) -> bool {
        self.favorites.iter().any(|item| item.id == id)
    }

    /// Stars `item`, or takes its star away. New favourites go first.
    pub fn toggle_favorite(&mut self, item: &StockItem) {
        if self.is_favorite(&item.id) {
            self.favorites.retain(|kept| kept.id != item.id);
        } else {
            self.favorites.insert(0, item.clone());
        }
    }

    /// Notes that `item` was just added to a project: it moves to the front of the recents,
    /// and the oldest drop off past `MAX_RECENTS`.
    pub fn note_added(&mut self, item: &StockItem) {
        self.recents.retain(|kept| kept.id != item.id);
        self.recents.insert(0, item.clone());
        self.recents.truncate(Self::MAX_RECENTS);
    }
}

/// Where an item's file is kept while it is only being previewed: in the temporary folder,
/// named after its address, so a later download of the same item can reuse it.
pub fn preview_path(item: &StockItem) -> PathBuf {
    use sha2::Digest as _;
    let digest: String = sha2::Sha256::digest(item.download_url.as_bytes()).iter().take(12).map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join("studio-stock-previews").join(format!("{digest}.{}", item.extension))
}

/// Fetches an item's file for previewing if it isn't already on disk, and says where it is.
pub fn fetch_preview(item: &StockItem, cancelled: &dyn Fn() -> bool) -> Result<PathBuf, String> {
    let path = preview_path(item);
    if path.exists() {
        return Ok(path);
    }
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    crate::download::fetch_file(&item.download_url, &path, &format!("“{}”", item.title), cancelled).map(|()| path)
}

/// The folder downloaded stock is saved in: a Stock folder beside the saved project, or in
/// Movies (Videos) for a project not saved yet.
pub fn folder(project_file: Option<&Path>) -> PathBuf {
    match project_file.and_then(Path::parent) {
        Some(folder) => folder.join("Stock"),
        // Tests never write into the person's Movies folder.
        None if cfg!(test) => std::env::temp_dir().join(format!("studio-test-stock-{}", std::process::id())),
        None => crate::panels::export::starting_folder(None).join("Studio Stock"),
    }
}

/// Downloads `item` into `folder` and says where it was saved. A file of the same name that
/// is already there is used as it is, so adding something twice downloads it once.
pub fn download(item: &StockItem, folder: &Path, cancelled: &dyn Fn() -> bool) -> Result<PathBuf, String> {
    std::fs::create_dir_all(folder).map_err(|error| format!("{} could not be created ({error}).", folder.display()))?;
    let path = folder.join(item.file_name());
    if path.exists() {
        return Ok(path);
    }
    // An item that was previewed is already on disk, so it is copied, not fetched again.
    let previewed = preview_path(item);
    if previewed.exists() && std::fs::copy(&previewed, &path).is_ok() {
        return Ok(path);
    }
    crate::download::fetch_file(&item.download_url, &path, &format!("“{}”", item.title), cancelled).map(|()| path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPENVERSE_IMAGES: &str = r#"{"result_count":240,"page_count":120,"page_size":2,"page":1,"results":[
        {"id":"eaed97d9","title":"Mountain","url":"https://live.staticflickr.com/8221/8339296677_2e0534488d_b.jpg","creator":"Pat","license":"cc0","source":"flickr","filesize":null,"filetype":null,"height":683,"width":1024,"thumbnail":"https://api.openverse.org/v1/images/eaed97d9/thumb/"},
        {"id":"not-free","title":"Credit me","url":"https://example.org/a.jpg","creator":"Sam","license":"by","filetype":"jpeg","height":10,"width":10,"thumbnail":null},
        {"id":"no-file","title":"Nothing to download","license":"cc0"}]}"#;

    const OPENVERSE_AUDIO: &str = r#"{"result_count":1,"page_count":1,"page_size":20,"page":1,"results":[
        {"id":"9df463a7","title":"Rain Sound.mp3","url":"https://cdn.freesound.org/previews/457/457447_9518146-hq.mp3","creator":"INNORECORDS","license":"cc0","provider":"freesound","filesize":5468620,"filetype":"mp3","duration":341000,"thumbnail":null}]}"#;

    const COMMONS_VIDEO: &str = r#"{"batchcomplete":"","continue":{"gsroffset":2,"continue":"gsroffset||"},"query":{"pages":{
        "92787551":{"pageid":92787551,"title":"File:Ocean surface waves.ogv","index":2,"imageinfo":[{"user":"Mostafa","size":63424136,"width":1920,"height":1080,"duration":21.32,"thumburl":"https://thumb.wikimedia.org/a.jpg?utm_source=x","url":"https://upload.wikimedia.org/wikipedia/commons/8/8a/Ocean_surface_waves.ogv?utm_source=x&utm_content=original","extmetadata":{"LicenseShortName":{"value":"CC0"}}}]},
        "81916645":{"pageid":81916645,"title":"File:Ocean in Sommaroy.webm","index":1,"imageinfo":[{"user":"Netha","size":2928602,"width":1920,"height":1080,"duration":15.472,"thumburl":"https://thumb.wikimedia.org/b.jpg","url":"https://upload.wikimedia.org/wikipedia/commons/5/5d/Ocean_in_Sommaroy.webm","extmetadata":{"ObjectName":{"value":"Ocean in Sommaroy"},"LicenseShortName":{"value":"CC0"}}}]},
        "7":{"pageid":7,"title":"File:Credited.webm","index":3,"imageinfo":[{"url":"https://upload.wikimedia.org/c.webm","extmetadata":{"LicenseShortName":{"value":"CC BY-SA 4.0"}}}]}}}}"#;

    // Shaped as the Pexels and Pixabay documentation describes their replies.
    const PEXELS_PHOTOS: &str = r#"{"page":1,"per_page":20,"total_results":2,"next_page":"https://api.pexels.com/v1/search/?page=2","photos":[
        {"id":2014422,"width":3024,"height":4032,"url":"https://www.pexels.com/photo/brown-rocks-2014422/","photographer":"Joey","alt":"Brown rocks during golden hour","src":{"original":"https://images.pexels.com/photos/2014422/pexels-photo-2014422.jpeg","medium":"https://images.pexels.com/photos/2014422/pexels-photo-2014422.jpeg?h=350","tiny":"https://images.pexels.com/photos/2014422/tiny.jpeg"}}]}"#;

    const PEXELS_VIDEOS: &str = r#"{"page":1,"per_page":20,"total_results":1,"videos":[
        {"id":1448735,"width":4096,"height":2160,"url":"https://www.pexels.com/video/video-of-forest-1448735/","image":"https://images.pexels.com/videos/1448735/free-video-1448735.jpg","duration":32,"user":{"id":574687,"name":"Ruvim"},"video_files":[
            {"id":1,"quality":"uhd","file_type":"video/mp4","width":4096,"height":2160,"link":"https://player.vimeo.com/external/uhd.mp4"},
            {"id":2,"quality":"hd","file_type":"video/mp4","width":1920,"height":1080,"link":"https://player.vimeo.com/external/hd.mp4"},
            {"id":3,"quality":"sd","file_type":"video/mp4","width":960,"height":540,"link":"https://player.vimeo.com/external/sd.mp4"},
            {"id":4,"quality":"hls","file_type":"application/x-mpegURL","width":null,"height":null,"link":"https://player.vimeo.com/external/master.m3u8"}]}]}"#;

    const PIXABAY_IMAGES: &str = r#"{"total":4692,"totalHits":500,"hits":[
        {"id":195893,"tags":"blossom, bloom, flower","previewURL":"https://cdn.pixabay.com/photo/2013/10/15/09/12/flower-195893_150.jpg","webformatURL":"https://pixabay.com/get/35bbf209e13e39d2_640.jpg","largeImageURL":"https://pixabay.com/get/ed6a99fd0a76647_1280.jpg","imageWidth":4000,"imageHeight":2250,"imageSize":4731420,"user":"Josch13"}]}"#;

    const PIXABAY_VIDEOS: &str = r#"{"total":42,"totalHits":15,"hits":[
        {"id":125,"tags":"flowers, yellow, blossom","duration":12,"user":"Coverr-Free-Footage","videos":{
            "large":{"url":"","width":0,"height":0,"size":0,"thumbnail":""},
            "medium":{"url":"https://cdn.pixabay.com/video/2015/08/08/125-135736646_medium.mp4","width":1280,"height":720,"size":3562083,"thumbnail":"https://cdn.pixabay.com/video/125_medium.jpg"},
            "small":{"url":"https://cdn.pixabay.com/video/2015/08/08/125-135736646_small.mp4","width":640,"height":360,"size":1030736,"thumbnail":"https://cdn.pixabay.com/video/125_small.jpg"}}}]}"#;

    #[test]
    fn only_public_domain_results_with_a_file_are_listed() {
        let (images, more) = parse(Source::Openverse, Kind::Image, OPENVERSE_IMAGES, 0).unwrap();
        assert_eq!(images.len(), 1, "the credited picture and the one without a file are left out");
        assert!(more, "page 1 of 120");
        let mountain = &images[0];
        assert_eq!((mountain.title.as_str(), mountain.extension.as_str(), mountain.width, mountain.height), ("Mountain", "jpg", Some(1024), Some(683)));
        assert_eq!(mountain.summary(), "1024 × 683");
        assert_eq!((mountain.source, mountain.origin.as_str()), (Source::Openverse, "Flickr via Openverse"), "the site it was found on is named");

        let (audio, more) = parse(Source::Openverse, Kind::Audio, OPENVERSE_AUDIO, 0).unwrap();
        assert!(!more);
        assert_eq!((audio[0].extension.as_str(), audio[0].duration_seconds, audio[0].summary().as_str()), ("mp3", Some(341.0), "5:41 · 5.2 MB"));
        assert_eq!(audio[0].file_name(), "Rain Sound mp3.mp3");
        assert_eq!(audio[0].origin, "Freesound via Openverse");
    }

    #[test]
    fn videos_come_in_search_order_without_tracking_in_their_addresses() {
        let (videos, more) = parse(Source::WikimediaCommons, Kind::Video, COMMONS_VIDEO, 0).unwrap();
        assert!(more);
        assert_eq!(videos.iter().map(|video| video.title.as_str()).collect::<Vec<_>>(), ["Ocean in Sommaroy", "Ocean surface waves"], "CC BY-SA is left out");
        assert_eq!(videos[1].download_url, "https://upload.wikimedia.org/wikipedia/commons/8/8a/Ocean_surface_waves.ogv");
        assert_eq!((videos[1].extension.as_str(), videos[1].summary().as_str()), ("ogv", "1920 × 1080 · 0:21 · 60.5 MB"));
        assert_eq!(videos[1].origin, "Wikimedia Commons");
        // A search with no matches has no "query" in its reply.
        assert_eq!(parse(Source::WikimediaCommons, Kind::Video, r#"{"batchcomplete":""}"#, 0).unwrap(), (Vec::new(), false));
        assert!(parse(Source::WikimediaCommons, Kind::Video, "<html>", 0).is_err());
    }

    #[test]
    fn results_that_mention_nudity_sex_or_gore_are_left_out() {
        assert!(is_unsafe(["Videos of sitting males masturbating"]));
        assert!(is_unsafe(["A calm lake", "Nude women in 2021"]));
        assert!(is_unsafe(["Human Defecation Process"]) && is_unsafe(["SEX education"]) && is_unsafe(["fellatio.webm"]));
        // Words that merely contain or begin like a blocked one are fine.
        for harmless in ["Sextant on a ship", "Essex coast", "Data analysis", "Sitting on a bench", "Cucumber salad", "Erecting a tent is not blocked"] {
            assert!(!is_unsafe([harmless]), "{harmless}");
        }
        assert!(!is_unsafe(std::iter::empty()));

        // Commons: a file is judged by its categories as well as its name.
        let page = |title: &str, categories: &str| format!(r#"{{"batchcomplete":"","query":{{"pages":{{"1":{{"pageid":1,"title":"File:{title}.webm","index":1,{categories}"imageinfo":[{{"url":"https://upload.wikimedia.org/a.webm","extmetadata":{{"LicenseShortName":{{"value":"CC0"}}}}}}]}}}}}}}}"#);
        let count = |reply: String| parse(Source::WikimediaCommons, Kind::Video, &reply, 0).unwrap().0.len();
        assert_eq!(count(page("Park bench", r#""categories":[{"title":"Category:Benches"}],"#)), 1);
        assert_eq!(count(page("Park bench", r#""categories":[{"title":"Category:Videos of nude women"}],"#)), 0, "an innocent name with an explicit category");
        assert_eq!(count(page("Adult male masturbating", "")), 0, "an explicit name");
        assert_eq!(count(page("Park bench", "")), 1, "no categories at all is fine when the reply was complete");
        // When the reply ran out of room for categories, a file without any is not trusted.
        let cut_short = page("Park bench", "").replacen(r#"{"batchcomplete":"","#, r#"{"continue":{"clcontinue":"1|x","continue":"||"},"#, 1);
        assert_eq!(count(cut_short), 0);

        // Openverse: work marked mature or sensitive, or tagged explicitly, is dropped.
        let result = |extra: &str| format!(r#"{{"page":1,"page_count":1,"results":[{{"id":"a","title":"Portrait","url":"https://example.org/a.jpg","license":"cc0"{extra}}}]}}"#);
        let count = |reply: String| parse(Source::Openverse, Kind::Image, &reply, 0).unwrap().0.len();
        assert_eq!(count(result("")), 1);
        assert_eq!(count(result(r#","mature":true"#)), 0);
        assert_eq!(count(result(r#","unstable__sensitivity":["sensitive_text"]"#)), 0);
        assert_eq!(count(result(r#","tags":[{"name":"studio"},{"name":"nude"}]"#)), 0);
    }

    #[test]
    fn pexels_results_take_the_full_hd_file_and_a_title_from_what_there_is() {
        let (photos, more) = parse(Source::Pexels, Kind::Image, PEXELS_PHOTOS, 0).unwrap();
        assert!(more, "a next page is named");
        let photo = &photos[0];
        assert_eq!((photo.id.as_str(), photo.title.as_str(), photo.extension.as_str(), photo.origin.as_str()), ("pexels:2014422", "Brown rocks during golden hour", "jpg", "Pexels"));
        assert_eq!(photo.download_url, "https://images.pexels.com/photos/2014422/pexels-photo-2014422.jpeg");

        let (videos, more) = parse(Source::Pexels, Kind::Video, PEXELS_VIDEOS, 0).unwrap();
        assert!(!more);
        let video = &videos[0];
        assert_eq!(video.download_url, "https://player.vimeo.com/external/hd.mp4", "full HD, not 4K, and never the streaming playlist");
        assert_eq!((video.title.as_str(), video.width, video.duration_seconds, video.creator.as_deref()), ("Video of forest", Some(1920), Some(32.0), Some("Ruvim")));
    }

    #[test]
    fn pixabay_results_skip_sizes_it_does_not_have() {
        let (images, more) = parse(Source::Pixabay, Kind::Image, PIXABAY_IMAGES, 0).unwrap();
        assert!(more, "500 reachable results, 20 a page");
        let image = &images[0];
        assert_eq!((image.title.as_str(), image.extension.as_str(), image.width, image.height), ("Blossom, bloom", "jpg", Some(1280), Some(720)), "sized as the file that is downloaded");
        assert_eq!(image.thumbnail_url.as_deref(), Some("https://pixabay.com/get/35bbf209e13e39d2_640.jpg"));

        let (videos, more) = parse(Source::Pixabay, Kind::Video, PIXABAY_VIDEOS, 0).unwrap();
        assert!(!more, "15 results fit on one page");
        let video = &videos[0];
        assert_eq!((video.download_url.as_str(), video.bytes, video.extension.as_str()), ("https://cdn.pixabay.com/video/2015/08/08/125-135736646_medium.mp4", Some(3_562_083), "mp4"), "the empty large size is passed over");
        assert_eq!(video.id, "pixabay:125");
    }

    #[test]
    fn searches_ask_for_public_domain_work_only_and_escape_what_was_typed() {
        let images = search_url(Source::Openverse, Kind::Image, " city & night ", 0, "").unwrap();
        assert!(images.contains("q=city%20%26%20night") && images.contains("license=cc0") && images.contains("mature=false") && images.ends_with("page=1"), "{images}");
        let audio = search_url(Source::Openverse, Kind::Audio, "rain", 2, "").unwrap();
        assert!(audio.contains("/audio/") && audio.ends_with("page=3"));
        let videos = search_url(Source::WikimediaCommons, Kind::Video, "ocean", 1, "").unwrap();
        assert!(videos.contains("gsrsearch=ocean%20-nude%20-nudity") && videos.contains("%20filetype%3Avideo%20haswbstatement%3AP275%3DQ6938433") && videos.contains("gsroffset=20"), "{videos}");
        assert!(videos.contains("prop=imageinfo%7Ccategories"), "categories are asked for, to judge each file by");
        // A service is only asked for the kinds it has.
        assert_eq!(search_url(Source::WikimediaCommons, Kind::Audio, "rain", 0, ""), None);
        assert_eq!(search_url(Source::Pexels, Kind::Audio, "rain", 0, "key"), None);
    }

    #[test]
    fn keys_are_sent_where_each_service_wants_them_and_nothing_else_rides_along() {
        // Pexels takes its key in a header, so it stays out of the address.
        let pexels = search_url(Source::Pexels, Kind::Video, "sea", 0, "abc123").unwrap();
        assert!(pexels.starts_with("https://api.pexels.com/videos/search?query=sea") && !pexels.contains("abc123"), "{pexels}");
        assert_eq!(key_header(Source::Pexels, " abc123 ").as_deref(), Some("Authorization: abc123"));
        assert_eq!(key_header(Source::Pixabay, "abc123"), None);
        let pixabay = search_url(Source::Pixabay, Kind::Image, "sea", 0, "123-abc").unwrap();
        assert!(pixabay.starts_with("https://pixabay.com/api/?key=123-abc&q=sea&image_type=photo&safesearch=true"), "{pixabay}");
        // Anything that isn't part of a key is dropped, so a pasted key can't add to the request.
        assert_eq!(clean_key(" ab\"c\n&q=x "), "abcqx");
    }

    #[test]
    fn every_kind_has_a_source_that_needs_no_key() {
        for kind in Kind::ALL {
            assert!(Source::ALL.iter().any(|source| !source.needs_key() && source.kinds().contains(&kind)), "{kind:?}");
        }
    }

    #[test]
    fn file_names_are_safe_and_never_empty() {
        let item = |title: &str| StockItem { id: "1".into(), kind: Kind::Image, source: Source::Openverse, origin: "Openverse".into(), title: title.into(), creator: None, thumbnail_url: None, download_url: String::new(), extension: "png".into(), width: None, height: None, duration_seconds: None, bytes: None };
        assert_eq!(item("Sun/set: over  the *sea*").file_name(), "Sun set over the sea.png");
        assert_eq!(item("../..").file_name(), "Stock.png");
    }

    /// Asks the real services that need no key. Run by hand with
    /// `cargo test -p studio stock -- --ignored`; left out of the normal run so the tests never
    /// depend on the network.
    #[test]
    #[ignore = "uses the network"]
    fn the_live_services_answer_with_public_domain_results() {
        let access: Vec<Access> = [Source::WikimediaCommons, Source::Openverse].map(|source| (source, String::new())).to_vec();
        for kind in Kind::ALL {
            let found = search(kind, "ocean", 0, &access);
            assert!(found.problems.is_empty(), "{:?}", found.problems);
            assert!(!found.items.is_empty(), "{kind:?} found nothing");
            assert!(found.items.iter().all(|item| !item.download_url.is_empty() && !item.extension.is_empty()));
        }
        // The search that once brought explicit video back from Commons. The titles are printed
        // so a person running this can read them over.
        for page in 0..2 {
            let found = search(Kind::Video, "sitting", page, &access);
            assert!(found.problems.is_empty(), "{:?}", found.problems);
            for item in &found.items {
                println!("sitting, page {page}: {}", item.title);
            }
        }
    }
}
