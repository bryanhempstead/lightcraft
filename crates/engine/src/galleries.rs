//! LC-GALLERIES: client galleries (Pic-Time-style, self-hosted) owned by Second Brain, reached
//! from LightCraft. See `docs/galleries.md`.
//!
//! **One source of truth.** Second Brain keeps the galleries (`<userData>/client/galleries.json`),
//! the gallery server and LOT48 publishing. It serves them to LightCraft over a small local JSON
//! API (`127.0.0.1`, a bearer token): [`Api`] finds it through `<userData>/gallery-api.json`
//! (port + token, mode 0600) and talks plain HTTP/1.1 to it. LightCraft **never writes**
//! `galleries.json`: when Second Brain isn't running, [`offline_galleries`] reads it so the
//! galleries tab still shows them, read-only.
//!
//! **What LightCraft adds.** A gallery made from a collection, shoot or selection is exported
//! WITH its LightCraft edits through [`crate::export`] (sized for the web, sRGB, sharpened for
//! screen, optional watermark) into a folder in the shoot (`<shoot>/gallery/<name>/`), then
//! registered with Second Brain. Which library photo each file came from is kept in the catalog
//! ([`GalleryLink`], record `gallery.link.<galleryId>`), so the client's picks map back onto the
//! library (flags / stars / labels, a "client picks" collection), a gallery photo opens its source
//! photo in Develop, and a re-export replaces each photo's own file in place.
//!
//! Creating a gallery is three steps so a front end can export on a worker thread (like
//! [`crate::publish`]): [`prepare_create`] (needs the session), [`CreateJob::run`] (no session:
//! render, write, register over the API) and [`finish_create`] (records the link).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use lightcraft_catalog::{Catalog, Op, PhotoId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::export::{Conflict, Destination, ExportOptions, PreparedExport};

/// Catalog records: `gallery.link.<second brain gallery id>` → [`GalleryLink`].
pub const LINK_PREFIX: &str = "gallery.link.";
/// The discovery file Second Brain writes in its userData folder.
pub const DISCOVERY_FILE: &str = "gallery-api.json";
/// The collection set the per-gallery "client picks" collections live in.
pub const PICKS_SET: &str = "Client Picks";
/// Largest API reply read (a gallery list with a few thousand photos is far below this).
const MAX_REPLY: u64 = 64 << 20;
/// Largest discovery / galleries.json file read.
const MAX_FILE: u64 = 32 << 20;

/// What a gallery made in LightCraft remembers: where its files are and which photo each one is.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GalleryLink {
    /// Second Brain's gallery id.
    pub gallery: String,
    pub name: String,
    /// The gallery folder the photos were exported into.
    pub folder: String,
    /// The `app.export` params the files were made with.
    pub export: Value,
    /// File name in the folder → library photo id.
    pub files: BTreeMap<String, u64>,
    /// Photo id → its fingerprint ([`crate::publish::fingerprint`]) when it was exported.
    pub fingerprints: BTreeMap<u64, u64>,
    /// The "client picks" collection kept in step with the client's hearts.
    pub picks_album: Option<u64>,
    /// When the files were last written (ISO 8601).
    pub at: String,
}

impl GalleryLink {
    /// The file a photo was exported to, when it was.
    pub fn file_of(&self, photo: u64) -> Option<&str> {
        self.files.iter().find(|(_, p)| **p == photo).map(|(f, _)| f.as_str())
    }
}

pub fn link(cat: &Catalog, gallery: &str) -> Option<GalleryLink> {
    cat.record(&format!("{LINK_PREFIX}{gallery}")).and_then(|v| serde_json::from_value(v.clone()).ok())
}

pub fn links(cat: &Catalog) -> Vec<GalleryLink> {
    cat.records_with_prefix(LINK_PREFIX).filter_map(|(_, v)| serde_json::from_value(v.clone()).ok()).collect()
}

pub(crate) fn set_link(l: &GalleryLink) -> Op {
    Op::SetRecord { key: format!("{LINK_PREFIX}{}", l.gallery), value: serde_json::to_value(l).ok() }
}

pub(crate) fn drop_link(gallery: &str) -> Op {
    Op::SetRecord { key: format!("{LINK_PREFIX}{gallery}"), value: None }
}

// ---------- finding Second Brain

/// Second Brain's userData folder: `dir` when set (tests, `LIGHTCRAFT_SECOND_BRAIN_DIR`), else
/// `~/Library/Application Support/second-brain`.
pub fn sb_dir(dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(d) = dir {
        return Some(d.to_path_buf());
    }
    if let Some(d) = std::env::var_os("LIGHTCRAFT_SECOND_BRAIN_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join("Library").join("Application Support").join("second-brain"))
}

fn read_capped(path: &Path) -> Result<Vec<u8>, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buf = Vec::new();
    f.take(MAX_FILE + 1).read_to_end(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
    if buf.len() as u64 > MAX_FILE {
        return Err(format!("{}: larger than {} MB", path.display(), MAX_FILE >> 20));
    }
    Ok(buf)
}

/// Errors talking to Second Brain.
#[derive(Clone, Debug, PartialEq)]
pub enum ApiError {
    /// Not running (no discovery file, or nothing listening on its port).
    Offline(String),
    /// It answered with an error.
    Failed(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Offline(why) => write!(f, "Second Brain isn't running ({why}) — start it to change galleries"),
            ApiError::Failed(why) => write!(f, "{why}"),
        }
    }
}

/// Second Brain's gallery API: where it listens and its token.
#[derive(Clone, PartialEq, Eq)]
pub struct Api {
    pub port: u16,
    token: String,
    /// How long one request may take (publishing copies a whole gallery to LOT48).
    pub timeout: Duration,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Api {{ port: {} }}", self.port)
    }
}

/// A raw reply: status, content type, body.
pub struct Reply {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

impl Api {
    pub fn new(port: u16, token: &str) -> Api {
        Api { port, token: token.to_string(), timeout: Duration::from_secs(20) }
    }

    /// Read `<dir>/gallery-api.json` → the API (not yet checked to be up: [`Api::ping`]).
    pub fn discover(dir: &Path) -> Result<Api, ApiError> {
        let file = dir.join(DISCOVERY_FILE);
        let bytes = read_capped(&file).map_err(|_| ApiError::Offline(format!("no {}", file.display())))?;
        let v: Value = serde_json::from_slice(&bytes).map_err(|e| ApiError::Offline(format!("{} is not JSON: {e}", file.display())))?;
        let port = v.get("port").and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()).filter(|p| *p > 0);
        let token =
            v.get("token").and_then(Value::as_str).filter(|t| !t.is_empty() && t.len() <= 256 && t.chars().all(|c| c.is_ascii_alphanumeric()));
        match (port, token) {
            (Some(port), Some(token)) => Ok(Api::new(port, token)),
            _ => Err(ApiError::Offline(format!("{} has no port / token", file.display()))),
        }
    }

    /// One HTTP/1.1 request to `127.0.0.1:<port>` → the raw reply.
    pub fn raw(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Reply, ApiError> {
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(800))
            .map_err(|e| ApiError::Offline(format!("nothing on port {}: {e}", self.port)))?;
        let _ = s.set_read_timeout(Some(self.timeout));
        let _ = s.set_write_timeout(Some(Duration::from_secs(10)));
        let payload = body.map(|b| b.to_string()).unwrap_or_default();
        let mut req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nAccept: application/json\r\nConnection: close\r\n",
            self.port, self.token
        );
        if body.is_some() {
            req.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", payload.len()));
        }
        req.push_str("\r\n");
        req.push_str(&payload);
        s.write_all(req.as_bytes()).map_err(|e| ApiError::Offline(format!("send failed: {e}")))?;
        let mut buf = Vec::new();
        (&mut s).take(MAX_REPLY + 1).read_to_end(&mut buf).map_err(|e| ApiError::Failed(format!("no answer from Second Brain: {e}")))?;
        if buf.len() as u64 > MAX_REPLY {
            return Err(ApiError::Failed("Second Brain's answer is too large".into()));
        }
        parse_reply(&buf).ok_or_else(|| ApiError::Failed("Second Brain's answer isn't HTTP".into()))
    }

    /// A JSON request → the JSON answer (`{ok: false, error}` answers are errors).
    pub fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, ApiError> {
        let r = self.raw(method, path, body)?;
        let v: Value = serde_json::from_slice(&r.body).map_err(|_| ApiError::Failed(format!("Second Brain answered {} without JSON", r.status)))?;
        if r.status == 401 {
            return Err(ApiError::Offline("its token changed — reopen the tab".into()));
        }
        if !(200..300).contains(&r.status) || v.get("ok").and_then(Value::as_bool) == Some(false) {
            let why = v.get("error").and_then(Value::as_str).unwrap_or("failed");
            return Err(ApiError::Failed(why.to_string()));
        }
        Ok(v)
    }

    pub fn get(&self, path: &str) -> Result<Value, ApiError> {
        self.call("GET", path, None)
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        self.call("POST", path, Some(body))
    }

    /// Is it up (`/v1/ping`)?
    pub fn ping(&self) -> Result<(), ApiError> {
        let v = self.get("/v1/ping")?;
        if v.get("app").and_then(Value::as_str) == Some("second-brain") {
            Ok(())
        } else {
            Err(ApiError::Offline("something else answers on its port".into()))
        }
    }

    /// A gallery photo's thumbnail (JPEG bytes).
    pub fn thumb(&self, gallery: &str, name: &str, size: u32) -> Result<Vec<u8>, ApiError> {
        let r = self.raw("GET", &format!("/v1/galleries/{}/thumb?name={}&size={size}", enc(gallery), enc(name)), None)?;
        if r.status == 200 && r.content_type.starts_with("image/") { Ok(r.body) } else { Err(ApiError::Failed(format!("no thumbnail for {name}"))) }
    }
}

/// Percent-encode a path segment / query value.
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn parse_reply(buf: &[u8]) -> Option<Reply> {
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(buf.get(..end)?).ok()?;
    let mut lines = head.split("\r\n");
    let status: u16 = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let mut content_type = String::new();
    let mut chunked = false;
    let mut length: Option<usize> = None;
    for l in lines {
        let Some((k, v)) = l.split_once(':') else { continue };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        match k.as_str() {
            "content-type" => content_type = v.to_ascii_lowercase(),
            "transfer-encoding" => chunked = v.to_ascii_lowercase().contains("chunked"),
            "content-length" => length = v.parse().ok(),
            _ => {}
        }
    }
    let rest = buf.get(end + 4..)?;
    let body = if chunked {
        dechunk(rest)?
    } else {
        match length {
            Some(n) => rest.get(..n.min(rest.len()))?.to_vec(),
            None => rest.to_vec(),
        }
    };
    Some(Reply { status, content_type, body })
}

fn dechunk(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for _ in 0..1_000_000 {
        let nl = b.windows(2).position(|w| w == b"\r\n")?;
        let size = std::str::from_utf8(b.get(..nl)?).ok()?.split(';').next()?.trim().to_string();
        let n = usize::from_str_radix(&size, 16).ok()?;
        b = b.get(nl + 2..)?;
        if n == 0 {
            return Some(out);
        }
        out.extend_from_slice(b.get(..n)?);
        b = b.get(n.checked_add(2)?..).unwrap_or(&[]);
    }
    None
}

/// The API when Second Brain is up, else why not.
pub fn connect(dir: Option<&Path>) -> Result<Api, ApiError> {
    let dir = sb_dir(dir).ok_or_else(|| ApiError::Offline("no home folder".into()))?;
    let api = Api::discover(&dir)?;
    api.ping()?;
    Ok(api)
}

/// The galleries straight from Second Brain's `client/galleries.json` (read-only: never written
/// here), each without its link key or password → `[gallery]`.
pub fn offline_galleries(dir: Option<&Path>) -> Result<Vec<Value>, String> {
    let dir = sb_dir(dir).ok_or("no home folder")?;
    let file = dir.join("client").join("galleries.json");
    let bytes = read_capped(&file).map_err(|e| format!("can't read the galleries ({e})"))?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| format!("{} is not JSON ({e})", file.display()))?;
    let list = v.get("galleries").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(list.into_iter().filter(Value::is_object).map(public_record).collect())
}

/// A gallery record as LightCraft shows it: no link key, the password only as "set".
pub fn public_record(mut g: Value) -> Value {
    if let Some(o) = g.as_object_mut() {
        o.remove("token");
        let has = o.remove("password").and_then(|p| p.as_str().map(|s| !s.is_empty())).unwrap_or(false);
        o.entry("hasPassword").or_insert(Value::Bool(has));
        let picks = o.get("selected").and_then(Value::as_array).map_or(0, Vec::len);
        o.entry("picks").or_insert(json!(picks));
    }
    g
}

/// The galleries, from the API when Second Brain is up, else read-only from its file.
#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub online: bool,
    pub galleries: Vec<Value>,
    /// Why it is read-only (offline), in words for the notice.
    pub notice: Option<String>,
}

/// [`Listing`] (no session needed: the app runs it on a worker thread).
pub fn fetch_list(dir: Option<&Path>) -> Result<Listing, String> {
    match connect(dir) {
        Ok(a) => {
            let v = a.get("/v1/galleries").map_err(|e| e.to_string())?;
            let galleries = v.get("galleries").and_then(Value::as_array).cloned().unwrap_or_default().into_iter().filter(Value::is_object).collect();
            Ok(Listing { online: true, galleries, notice: None })
        }
        Err(why) => {
            log::info!("galleries: {why}");
            Ok(match offline_galleries(dir) {
                Ok(l) => Listing {
                    online: false,
                    galleries: l,
                    notice: Some("Second Brain isn't running: its galleries are shown read-only. start. opens it.".into()),
                },
                Err(e) => Listing { online: false, galleries: Vec::new(), notice: Some(format!("Second Brain isn't running, and {e}.")) },
            })
        }
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

fn is_photo_name(n: &str) -> bool {
    let ext = n.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "heic" | "heif" | "gif" | "tif" | "tiff" | "avif")
}

/// One gallery with its photos → `{online, readOnly, notice?, gallery, photos: [{name, picked,
/// highlight, hidden}], videos, files}` (no session needed). Offline: the record from Second
/// Brain's file and the photos read from its folder.
pub fn fetch_gallery(dir: Option<&Path>, id: &str) -> Result<Value, String> {
    let marks = |names: Vec<String>, g: &Value| -> Vec<Value> {
        let (sel, hi, hid) = (strings(g.get("selected")), strings(g.get("highlights")), strings(g.get("hidden")));
        names.into_iter().map(|n| json!({"picked": sel.contains(&n), "highlight": hi.contains(&n), "hidden": hid.contains(&n), "name": n})).collect()
    };
    match connect(dir) {
        Ok(a) => {
            let v = a.get(&format!("/v1/galleries/{}", enc(id))).map_err(|e| e.to_string())?;
            let g = v.get("gallery").cloned().unwrap_or(Value::Null);
            let names: Vec<String> = v
                .get("photos")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|x| x.get("name").and_then(Value::as_str)).map(str::to_string).collect())
                .unwrap_or_default();
            let photos = marks(names, &g);
            Ok(json!({"online": true, "readOnly": false, "gallery": g, "photos": photos, "videos": v.get("videos"), "files": v.get("files")}))
        }
        Err(why) => {
            let list = offline_galleries(dir).map_err(|e| format!("{why}, and {e}"))?;
            let g = list.into_iter().find(|g| g.get("id").and_then(Value::as_str) == Some(id)).ok_or_else(|| format!("no gallery {id}"))?;
            let folder = g.get("folder").and_then(Value::as_str).unwrap_or("").to_string();
            let mut names: Vec<String> = std::fs::read_dir(&folder)
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                        .map(|e| e.file_name().to_string_lossy().to_string())
                        .filter(|n| !n.starts_with('.') && is_photo_name(n))
                        .take(20_000)
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            let photos = marks(names, &g);
            Ok(json!({"online": false, "readOnly": true, "notice": format!("{why}. Read-only."), "gallery": g, "photos": photos}))
        }
    }
}

/// What LightCraft knows about a gallery record: made here, how many photos map back, its
/// "client picks" collection.
pub fn annotate(cat: &Catalog, mut g: Value) -> Value {
    let id = g.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let l = link(cat, &id);
    if let Some(o) = g.as_object_mut() {
        o.insert("linked".into(), json!(l.is_some()));
        if let Some(l) = l {
            o.insert("linkedPhotos".into(), json!(l.files.len()));
            o.insert("picksAlbum".into(), json!(l.picks_album));
        }
    }
    g
}

/// [`annotate`] a [`fetch_gallery`] answer, and give each photo its library `photo` id (or null).
pub fn annotate_detail(cat: &Catalog, id: &str, v: &mut Value) {
    let l = link(cat, id);
    if let Some(g) = v.get_mut("gallery") {
        *g = annotate(cat, g.take());
    }
    if let Some(photos) = v.get_mut("photos").and_then(Value::as_array_mut) {
        for p in photos.iter_mut() {
            let name = p.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            let photo = l.as_ref().and_then(|l| l.files.get(&name)).copied().filter(|ph| cat.photo(PhotoId(*ph)).is_some_and(|x| x.in_library()));
            if let Some(o) = p.as_object_mut() {
                o.insert("photo".into(), json!(photo));
            }
        }
    }
}

// ---------- making a gallery from photos

/// The folder a gallery of these photos goes in: `<shoot>/gallery/<name>`, the shoot being the
/// folder the photos share (a camera folder `…/raw/<camera>` or `…/raw` counts as its shoot).
pub fn gallery_folder_for(paths: &[String], name: &str, fallback: &Path) -> PathBuf {
    let safe = safe_name(name);
    let dirs: Vec<PathBuf> = paths.iter().filter_map(|p| Path::new(p).parent().map(Path::to_path_buf)).collect();
    let Some(first) = dirs.first() else { return fallback.join(&safe) };
    let mut common: PathBuf = first.clone();
    for d in dirs.iter().skip(1) {
        let mut c = PathBuf::new();
        for (a, b) in common.components().zip(d.components()) {
            if a != b {
                break;
            }
            c.push(a);
        }
        common = c;
    }
    // a camera folder in raw/, raw/ itself, a format folder: the shoot is above it
    let generic = |n: &str| matches!(n.to_ascii_lowercase().as_str(), "raw" | "raws" | "jpg" | "jpeg" | "jpgs" | "dng" | "originals" | "selects");
    for _ in 0..3 {
        let name_here = common.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let parent_name = common.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if (generic(&name_here) || generic(&parent_name))
            && let Some(p) = common.parent()
        {
            common = p.to_path_buf();
            continue;
        }
        break;
    }
    // the root or a disk's root alone is no shoot
    if common.components().count() <= 2 {
        return fallback.join(&safe);
    }
    common.join("gallery").join(safe)
}

fn safe_name(s: &str) -> String {
    let t: String = s.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '\0') || c.is_control() { '-' } else { c }).collect();
    let t = t.trim().trim_matches('.').trim().to_string();
    if t.is_empty() { "Gallery".into() } else { t.chars().take(120).collect() }
}

/// The export settings a gallery's files are made with: JPEG for the web, sRGB, sharpened for
/// screen, location removed; `over` (app.export keys) wins.
pub fn web_export(over: &Value) -> Value {
    let mut p = json!({
        "format": "jpeg",
        "quality": 85,
        "longEdge": 2048,
        "dontEnlarge": true,
        "colorSpace": "sRGB",
        "sharpen": "screen",
        "sharpenAmount": "standard",
        "metadata": "copyrightContact",
        "removeLocation": true,
    });
    if let (Some(o), Some(m)) = (p.as_object_mut(), over.as_object()) {
        for (k, v) in m {
            o.insert(k.clone(), v.clone());
        }
    }
    p
}

fn export_options(params: &Value) -> Result<ExportOptions, String> {
    let mut o = ExportOptions::from_params(&ExportOptions::known_keys_only(params)).map_err(|e| e.to_string())?;
    o.export_to = crate::export::ExportTo::SpecificFolder;
    o.add_to_catalog = false;
    o.add_to_stack = None;
    o.subfolder.clear();
    o.after = crate::export::AfterExport::default();
    o.conflict = Conflict::Unique;
    Ok(o)
}

/// What a new gallery is: the record fields Second Brain gets.
#[derive(Clone, Debug, Default)]
pub struct NewGallery {
    pub name: String,
    pub client_name: String,
    pub client_email: String,
    pub password: String,
    pub expires: u64,
    pub allow_download: bool,
    pub allow_prints: bool,
}

/// An export into a gallery folder, ready to run off the UI thread.
pub struct CreateJob {
    /// The gallery being re-exported (None: a new one, registered after the export).
    pub existing: Option<String>,
    pub new: NewGallery,
    pub folder: PathBuf,
    items: Vec<PreparedExport>,
    opts: ExportOptions,
    params: Value,
    /// Photo id → (fingerprint, its current file name in the folder).
    facts: HashMap<u64, (u64, Option<String>)>,
    /// File names other photos already have there.
    others: HashSet<String>,
    api: Option<Api>,
    at: String,
}

impl CreateJob {
    pub fn photos(&self) -> usize {
        self.items.len()
    }
}

/// What a run did.
#[derive(Debug, Default)]
pub struct CreateOutcome {
    pub existing: Option<String>,
    pub name: String,
    pub folder: String,
    pub params: Value,
    /// (photo id, fingerprint, file name in the folder).
    pub written: Vec<(u64, u64, String)>,
    pub errors: Vec<Value>,
    /// The gallery record Second Brain registered (new galleries), or why it didn't.
    pub registered: Option<Result<Value, String>>,
    pub cancelled: bool,
    pub at: String,
}

fn str_of<'a>(p: &'a Value, k: &str) -> &'a str {
    p.get(k).and_then(Value::as_str).unwrap_or("")
}

/// The photos a gallery is made of: `ids`, else `source`: `selection` | `view` (what the grid
/// shows: a collection, a shoot, a camera folder part, a filter) | `album` (`album: id`).
pub fn source_photos(s: &mut crate::Session, p: &Value) -> Result<Vec<PhotoId>, String> {
    if let Some(ids) = p.get("ids").and_then(Value::as_array) {
        let v: Vec<PhotoId> =
            ids.iter().filter_map(Value::as_u64).map(PhotoId).filter(|id| s.catalog.photo(*id).is_some_and(|ph| ph.in_library())).collect();
        return if v.is_empty() { Err("none of those ids is a library photo".into()) } else { Ok(v) };
    }
    let v: Vec<PhotoId> = match str_of(p, "source") {
        "" | "selection" => {
            let t = s.targets(&Value::Null);
            if t.is_empty() && str_of(p, "source").is_empty() { s.visible().to_vec() } else { t }
        }
        "view" => s.visible().to_vec(),
        "album" => {
            let id = p.get("album").and_then(Value::as_u64).ok_or("`source: album` needs `album: id`")?;
            let a = s.catalog.album(lightcraft_catalog::AlbumId(id)).ok_or_else(|| format!("no collection {id}"))?;
            a.photos.clone()
        }
        other => return Err(format!("source is selection | view | album, not “{other}”")),
    };
    let v: Vec<PhotoId> = v
        .into_iter()
        .filter(|id| s.catalog.photo(*id).is_some_and(|ph| ph.in_library() && ph.kind != lightcraft_catalog::MediaKind::Video))
        .collect();
    if v.is_empty() { Err("no photos to put in the gallery (select some, or show a collection)".into()) } else { Ok(v) }
}

fn file_paths(s: &crate::Session, ids: &[PhotoId]) -> Vec<String> {
    ids.iter()
        .filter_map(|id| s.catalog.photo(*id))
        .filter_map(|p| match &p.source {
            lightcraft_catalog::Source::File { path } => Some(path.clone()),
            _ => None,
        })
        .collect()
}

/// Step 1 of a new gallery (`gallery.create`): the photos, the folder, the export settings.
pub fn prepare_create(s: &mut crate::Session, p: &Value) -> Result<CreateJob, String> {
    let ids = source_photos(s, p)?;
    if ids.len() > 5000 {
        return Err(format!("{} photos is more than a gallery takes (5000)", ids.len()));
    }
    let name = str_of(p, "name").trim().to_string();
    let name = if name.is_empty() { default_name(s, &ids) } else { name };
    let folder = match str_of(p, "folder").trim() {
        "" => {
            // photos that share no shoot folder (demo photos, several disks): ~/Pictures
            let fallback = std::env::var_os("HOME")
                .filter(|h| !h.is_empty())
                .map(|h| PathBuf::from(h).join("Pictures").join("LightCraft Galleries"))
                .unwrap_or_else(|| std::env::temp_dir().join("LightCraft Galleries"));
            gallery_folder_for(&file_paths(s, &ids), &name, &fallback)
        }
        f => PathBuf::from(f),
    };
    if !folder.is_absolute() {
        return Err("`folder` is an absolute path".into());
    }
    let mut over = p.get("export").cloned().unwrap_or_else(|| json!({}));
    if let Some(w) = p.get("watermark").filter(|w| !w.is_null()) {
        let wm = match w.as_str().and_then(|n| s.watermark_presets.iter().find(|x| x.name == n)) {
            Some(preset) => serde_json::to_value(&preset.watermark).unwrap_or(Value::Null),
            None => w.clone(),
        };
        if let Some(o) = over.as_object_mut() {
            o.insert("watermark".into(), wm);
        }
    }
    let params = web_export(&over);
    let opts = export_options(&params)?;
    let new = NewGallery {
        name: name.clone(),
        client_name: str_of(p, "clientName").to_string(),
        client_email: str_of(p, "clientEmail").to_string(),
        password: str_of(p, "password").to_string(),
        expires: p.get("expires").and_then(Value::as_u64).unwrap_or(0),
        allow_download: p.get("allowDownload").and_then(Value::as_bool).unwrap_or(true),
        allow_prints: p.get("allowPrints").and_then(Value::as_bool).unwrap_or(false),
    };
    let api = connect(s.second_brain_dir.as_deref()).ok();
    if api.is_none() && !p.get("exportOnly").and_then(Value::as_bool).unwrap_or(false) {
        return Err(ApiError::Offline("no answer from its gallery API".into()).to_string());
    }
    let facts = ids.iter().filter_map(|id| s.catalog.photo(*id).map(|ph| (id.0, (crate::publish::fingerprint(ph), None)))).collect();
    let items = crate::export::prepare_batch(s, &ids, &opts)?;
    Ok(CreateJob { existing: None, new, folder, items, opts, params, facts, others: HashSet::new(), api, at: (s.clock)() })
}

/// Step 1 of a re-export (`gallery.reexport`): the gallery's photos whose edits changed since
/// (all of them with `all`), plus `ids` to add.
pub fn prepare_reexport(s: &mut crate::Session, gallery: &str, add: &[PhotoId], all: bool) -> Result<CreateJob, String> {
    let l = link(&s.catalog, gallery).ok_or_else(|| format!("gallery {gallery} wasn't made in LightCraft (no photos to re-export)"))?;
    let mut ids: Vec<PhotoId> = Vec::new();
    let mut facts = HashMap::new();
    let mut members: Vec<u64> = l.files.values().copied().collect();
    members.extend(add.iter().map(|a| a.0));
    members.sort_unstable();
    members.dedup();
    for id in members {
        let Some(ph) = s.catalog.photo(PhotoId(id)).filter(|p| p.in_library()) else { continue };
        let fp = crate::publish::fingerprint(ph);
        let own = l.file_of(id).map(str::to_string);
        let changed = l.fingerprints.get(&id) != Some(&fp) || own.is_none();
        if all || changed {
            ids.push(PhotoId(id));
            facts.insert(id, (fp, own));
        }
    }
    let others: HashSet<String> = l.files.iter().filter(|(_, p)| !facts.contains_key(p)).map(|(f, _)| f.clone()).collect();
    let opts = export_options(&l.export)?;
    let items = crate::export::prepare_batch(s, &ids, &opts)?;
    let api = connect(s.second_brain_dir.as_deref()).ok();
    Ok(CreateJob {
        existing: Some(gallery.to_string()),
        new: NewGallery { name: l.name.clone(), ..Default::default() },
        folder: PathBuf::from(&l.folder),
        items,
        opts,
        params: l.export.clone(),
        facts,
        others,
        api,
        at: (s.clock)(),
    })
}

fn default_name(s: &crate::Session, ids: &[PhotoId]) -> String {
    if let crate::LibrarySource::Album(a) = s.source
        && let Some(al) = s.catalog.album(a)
    {
        return al.name.clone();
    }
    let paths = file_paths(s, ids);
    let f = gallery_folder_for(&paths, "x", Path::new("/"));
    f.parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Gallery".into())
}

impl CreateJob {
    /// Render and write the photos into the folder (each photo's own earlier file is replaced in
    /// place), then register a new gallery with Second Brain (or tell it the folder changed).
    /// Never fails as a whole: each photo's error is in the outcome. `progress(done, file)` before
    /// each photo; false cancels the rest.
    pub fn run(self, write: &mut dyn FnMut(&str, &[u8]) -> Result<(), String>, progress: &mut dyn FnMut(usize, &str) -> bool) -> CreateOutcome {
        let folder = self.folder.to_string_lossy().to_string();
        let mut out = CreateOutcome {
            existing: self.existing.clone(),
            name: self.new.name.clone(),
            folder: folder.clone(),
            params: self.params.clone(),
            at: self.at.clone(),
            ..Default::default()
        };
        if let Err(e) = std::fs::create_dir_all(&self.folder) {
            out.errors.push(json!({"error": format!("{folder}: {e}")}));
            return out;
        }
        let to = Destination { dir: folder.clone(), exact: None };
        let join = |n: &str| self.folder.join(n).to_string_lossy().to_string();
        let mut taken: HashSet<String> = self.others.iter().map(|n| join(n)).collect();
        for (i, item) in self.items.into_iter().enumerate() {
            let photo = item.photo.0;
            let (fp, own) = self.facts.get(&photo).cloned().unwrap_or((0, None));
            let own = own.map(|n| join(&n));
            let exists = |p: &str| own.as_deref() != Some(p) && (taken.contains(p) || Path::new(p).exists());
            let mut prog = |_: usize, f: &str| progress(i, f);
            match crate::export::run_batch(vec![item], &self.opts, &to, write, &exists, false, &mut prog) {
                Ok(r) if r.is_empty() => {
                    out.cancelled = true;
                    break;
                }
                Ok(r) => {
                    for v in r {
                        match v.get("path").and_then(Value::as_str) {
                            Some(p) => {
                                taken.insert(p.to_string());
                                let name = Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                                // re-exported under another name (a format change): the old file goes
                                if let Some(old) = own.as_deref().filter(|o| *o != p) {
                                    let _ = std::fs::remove_file(old);
                                }
                                out.written.push((photo, fp, name));
                            }
                            None => out.errors.push(v),
                        }
                    }
                }
                Err(e) => out.errors.push(json!({"photo": photo, "error": e})),
            }
        }
        let Some(api) = self.api else { return out };
        out.registered = Some(match &self.existing {
            // the folder's contents changed: Second Brain re-counts it
            Some(id) => api.post(&format!("/v1/galleries/{}/folder", enc(id)), &json!({"folder": folder})).map_err(|e| e.to_string()),
            None if out.written.is_empty() => Err("no photo could be exported, so no gallery was made".into()),
            None => {
                let body = json!({
                    "folder": folder,
                    "name": self.new.name,
                    "clientName": self.new.client_name,
                    "clientEmail": self.new.client_email,
                    "password": self.new.password,
                    "expires": self.new.expires,
                    "allowDownload": self.new.allow_download,
                    "allowPrints": self.new.allow_prints,
                    "lightcraft": {"photos": out.written.len(), "at": out.at},
                });
                api.post("/v1/galleries", &body).map(|v| v.get("gallery").cloned().unwrap_or(Value::Null)).map_err(|e| e.to_string())
            }
        });
        out
    }
}

/// Step 3: record which photo each file is (bookkeeping, not an undo step) →
/// `{gallery, folder, exported, errors, registered}`.
pub fn finish_create(s: &mut crate::Session, out: CreateOutcome) -> crate::Result<Value> {
    let gallery_id = match (&out.existing, &out.registered) {
        (Some(id), _) => Some(id.clone()),
        (None, Some(Ok(g))) => g.get("id").and_then(Value::as_str).map(str::to_string),
        _ => None,
    };
    let registered_error = match &out.registered {
        Some(Err(e)) => Some(e.clone()),
        _ => None,
    };
    let Some(gid) = gallery_id else {
        return Ok(json!({
            "gallery": Value::Null,
            "folder": out.folder,
            "exported": out.written.len(),
            "errors": out.errors,
            "cancelled": out.cancelled,
            "registered": false,
            "note": registered_error.unwrap_or_else(|| "exported only: Second Brain isn't running, so no gallery was registered".into()),
        }));
    };
    let mut l = link(&s.catalog, &gid).unwrap_or_else(|| GalleryLink {
        gallery: gid.clone(),
        name: out.name.clone(),
        folder: out.folder.clone(),
        export: out.params.clone(),
        ..Default::default()
    });
    for (photo, fp, name) in &out.written {
        l.files.retain(|_, p| p != photo);
        l.files.insert(name.clone(), *photo);
        l.fingerprints.insert(*photo, *fp);
    }
    l.at = out.at.clone();
    s.apply_bookkeeping(set_link(&l))?;
    let record = match &out.registered {
        Some(Ok(g)) if g.is_object() => g.clone(),
        _ => Value::Null,
    };
    Ok(json!({
        "gallery": gid,
        "record": record,
        "folder": out.folder,
        "exported": out.written.len(),
        "errors": out.errors,
        "cancelled": out.cancelled,
        "registered": registered_error.is_none(),
        "note": registered_error,
    }))
}

// ---------- client picks back into the library

/// The library photos behind gallery file names (only for galleries made in LightCraft).
pub fn photos_for(l: &GalleryLink, names: &[String], cat: &Catalog) -> Vec<PhotoId> {
    let mut v: Vec<PhotoId> =
        names.iter().filter_map(|n| l.files.get(n)).copied().map(PhotoId).filter(|id| cat.photo(*id).is_some_and(|p| p.in_library())).collect();
    v.dedup();
    v
}

/// The "client picks" collection for a gallery: made on first use under [`PICKS_SET`], then its
/// photos set to `picks` (an undo step) → its id.
pub fn sync_picks_album(s: &mut crate::Session, l: &mut GalleryLink, picks: &[PhotoId]) -> crate::Result<u64> {
    use lightcraft_catalog::{Album, AlbumId};
    let alive = l.picks_album.filter(|a| s.catalog.album(AlbumId(*a)).is_some_and(|al| !al.folder && !al.is_smart()));
    let id = match alive {
        Some(a) => AlbumId(a),
        None => {
            let found = s.catalog.albums().find(|a| a.folder && a.name == PICKS_SET && a.parent.is_none()).map(|a| a.id);
            let set = match found {
                Some(set) => set,
                None => {
                    let set = s.catalog.alloc_album_id();
                    s.commit("New Collection Set", Op::AddAlbum { album: Album { folder: true, ..Album::new(set, PICKS_SET) } })?;
                    set
                }
            };
            let id = s.catalog.alloc_album_id();
            let name = format!("{} · client picks", l.name);
            s.commit("New Collection", Op::AddAlbum { album: Album { parent: Some(set), ..Album::new(id, &name) } })?;
            id
        }
    };
    let same = s.catalog.album(id).is_some_and(|a| a.photos == picks);
    if !same {
        let cover = picks.first().copied();
        s.commit("Client Picks", Op::Batch { ops: vec![Op::SetAlbumPhotos { id, photos: picks.to_vec() }, Op::SetAlbumCover { id, cover }] })?;
    }
    if l.picks_album != Some(id.0) {
        l.picks_album = Some(id.0);
        s.apply_bookkeeping(set_link(l))?;
    }
    Ok(id.0)
}

/// A stand-in for Second Brain's gallery API, for tests (engine, app, MCP): an in-memory list of
/// galleries over the same routes, on a free 127.0.0.1 port, announced through a discovery file
/// in `dir` like the real one. Never reads or writes the real Second Brain.
#[doc(hidden)]
pub mod fake {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};

    use serde_json::{Value, json};

    pub const TOKEN: &str = "faketoken0123456789";

    #[derive(Default)]
    pub struct State {
        pub galleries: Vec<Value>,
        /// Every request: (method, path, body).
        pub requests: Vec<(String, String, Value)>,
        next: u32,
    }

    pub struct FakeSecondBrain {
        pub port: u16,
        pub dir: PathBuf,
        pub state: Arc<Mutex<State>>,
        stop: Arc<AtomicBool>,
    }

    fn photos_in(folder: &str) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(folder)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|n| !n.starts_with('.') && n.contains('.'))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    impl FakeSecondBrain {
        /// Start it; `dir` gets `gallery-api.json` (and `client/galleries.json` on [`Self::stop`]).
        pub fn start(dir: &Path) -> std::io::Result<FakeSecondBrain> {
            std::fs::create_dir_all(dir)?;
            let l = TcpListener::bind("127.0.0.1:0")?;
            let port = l.local_addr()?.port();
            std::fs::write(dir.join(super::DISCOVERY_FILE), json!({"api": 1, "port": port, "token": TOKEN}).to_string())?;
            let state = Arc::new(Mutex::new(State::default()));
            let stop = Arc::new(AtomicBool::new(false));
            let (st, sp) = (state.clone(), stop.clone());
            std::thread::Builder::new().name("fake-second-brain".into()).spawn(move || {
                for conn in l.incoming() {
                    if sp.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Ok(c) = conn {
                        let st = st.clone();
                        let _ = std::thread::Builder::new().name("fake-sb-conn".into()).spawn(move || serve(c, &st));
                    }
                }
            })?;
            Ok(FakeSecondBrain { port, dir: dir.to_path_buf(), state, stop })
        }

        pub fn lock(&self) -> std::sync::MutexGuard<'_, State> {
            self.state.lock().unwrap_or_else(PoisonError::into_inner)
        }

        /// A client hearts these files in gallery `id`.
        pub fn set_selected(&self, id: &str, names: &[&str]) {
            let mut s = self.lock();
            if let Some(g) = s.galleries.iter_mut().find(|g| g["id"] == id) {
                g["selected"] = json!(names);
            }
        }

        /// Add a gallery that wasn't made in LightCraft → its id.
        pub fn add(&self, name: &str, folder: &str) -> String {
            let mut s = self.lock();
            s.next += 1;
            let id = format!("sb{}", s.next);
            let count = photos_in(folder).len();
            s.galleries.push(json!({"id": id, "name": name, "folder": folder, "count": count, "token": "secret", "password": "", "selected": [], "published": false, "views": 0, "orders": 0}));
            id
        }

        /// Stop answering, and leave the list behind as `client/galleries.json` (what Second Brain
        /// keeps on disk), so the app falls back to reading it.
        pub fn stop(&self) {
            self.stop.store(true, Ordering::Relaxed);
            let _ = TcpStream::connect(("127.0.0.1", self.port));
            let gl = self.lock().galleries.clone();
            let _ = std::fs::create_dir_all(self.dir.join("client"));
            let _ = std::fs::write(self.dir.join("client").join("galleries.json"), json!({"galleries": gl}).to_string());
            let _ = std::fs::remove_file(self.dir.join(super::DISCOVERY_FILE));
        }
    }

    impl Drop for FakeSecondBrain {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            let _ = TcpStream::connect(("127.0.0.1", self.port));
        }
    }

    fn reply(c: &mut TcpStream, code: u16, v: &Value) {
        let body = v.to_string();
        let _ = write!(c, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    }

    fn serve(mut c: TcpStream, st: &Mutex<State>) {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        let head_end = loop {
            match c.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(chunk.get(..n).unwrap_or(&[])),
            }
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i;
            }
            if buf.len() > 1 << 20 {
                return;
            }
        };
        let head = String::from_utf8_lossy(buf.get(..head_end).unwrap_or(&[])).to_string();
        let len: usize =
            head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0))).unwrap_or(0);
        while buf.len() < head_end + 4 + len {
            match c.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(chunk.get(..n).unwrap_or(&[])),
            }
        }
        let body: Value = serde_json::from_slice(buf.get(head_end + 4..).unwrap_or(&[])).unwrap_or(json!({}));
        let mut first = head.lines().next().unwrap_or("").split_whitespace();
        let (method, path) = (first.next().unwrap_or("").to_string(), first.next().unwrap_or("").to_string());
        if !head.contains(&format!("Bearer {TOKEN}")) {
            return reply(&mut c, 401, &json!({"ok": false, "error": "bad token"}));
        }
        let mut s = st.lock().unwrap_or_else(PoisonError::into_inner);
        s.requests.push((method.clone(), path.clone(), body.clone()));
        let (code, v) = route(&mut s, &method, &path, &body);
        drop(s);
        reply(&mut c, code, &v);
    }

    fn public(g: &Value) -> Value {
        let mut g = g.clone();
        if let Some(o) = g.as_object_mut() {
            o.remove("token");
            let has = o.remove("password").is_some_and(|p| p.as_str().is_some_and(|x| !x.is_empty()));
            o.insert("hasPassword".into(), json!(has));
            let n = o.get("selected").and_then(Value::as_array).map_or(0, Vec::len);
            o.insert("picks".into(), json!(n));
        }
        g
    }

    fn set(s: &mut State, i: usize, k: &str, v: Value) {
        if let Some(t) = s.galleries.get_mut(i).and_then(Value::as_object_mut) {
            t.insert(k.into(), v);
        }
    }

    fn route(s: &mut State, method: &str, path: &str, b: &Value) -> (u16, Value) {
        let path = path.split_once('?').map_or(path, |(p, _)| p);
        let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
        match (method, parts.as_slice()) {
            ("GET", ["v1", "ping"]) => (200, json!({"ok": true, "app": "second-brain", "api": 1})),
            ("GET", ["v1", "galleries"]) => (200, json!({"ok": true, "galleries": s.galleries.iter().map(public).collect::<Vec<_>>()})),
            ("POST", ["v1", "galleries"]) => {
                let folder = b["folder"].as_str().unwrap_or("").to_string();
                if !Path::new(&folder).is_dir() {
                    return (422, json!({"ok": false, "error": "folder not found"}));
                }
                s.next += 1;
                let mut g = b.clone();
                if let Some(o) = g.as_object_mut() {
                    o.insert("id".into(), json!(format!("g{}", s.next)));
                    o.insert("token".into(), json!("secret"));
                    o.insert("count".into(), json!(photos_in(&folder).len()));
                    o.insert("selected".into(), json!([]));
                    o.insert("published".into(), json!(false));
                    o.insert("views".into(), json!(0));
                    o.insert("orders".into(), json!(0));
                }
                s.galleries.push(g.clone());
                (200, json!({"ok": true, "gallery": public(&g)}))
            }
            (m, ["v1", "galleries", id, rest @ ..]) => {
                let Some(i) = s.galleries.iter().position(|g| g["id"] == *id) else {
                    return (404, json!({"ok": false, "error": format!("no gallery {id}")}));
                };
                let g = s.galleries.get(i).cloned().unwrap_or(Value::Null);
                match (m, rest) {
                    ("GET", []) => {
                        let photos: Vec<Value> =
                            photos_in(g["folder"].as_str().unwrap_or("")).into_iter().map(|n| json!({"name": n, "size": 1})).collect();
                        let mut full = public(&g);
                        if let Some(o) = full.as_object_mut() {
                            o.insert("password".into(), g["password"].clone());
                        }
                        (200, json!({"ok": true, "gallery": full, "photos": photos, "videos": [], "files": []}))
                    }
                    ("PATCH", []) => {
                        for (k, v) in b.as_object().cloned().unwrap_or_default() {
                            set(s, i, &k, v);
                        }
                        (200, json!({"ok": true, "gallery": s.galleries.get(i).map(public)}))
                    }
                    ("DELETE", []) => {
                        s.galleries.remove(i);
                        (200, json!({"ok": true}))
                    }
                    ("POST", ["folder"]) => {
                        let f = b["folder"].as_str().unwrap_or("").to_string();
                        set(s, i, "count", json!(photos_in(&f).len()));
                        set(s, i, "folder", json!(f));
                        (200, json!({"ok": true, "gallery": s.galleries.get(i).map(public)}))
                    }
                    ("GET", ["picks"]) => (200, json!({"ok": true, "selected": g["selected"], "highlights": [], "hidden": [], "comments": []})),
                    ("POST", ["picks", "pull"]) => (200, json!({"ok": true, "pulled": true, "selected": g["selected"]})),
                    ("GET", ["link"]) => (
                        200,
                        json!({"ok": true, "url": format!("https://gallery.test/{id}?k=secret"), "public": true, "permanent": true, "links": []}),
                    ),
                    ("POST", ["publish"]) => {
                        set(s, i, "published", json!(true));
                        (200, json!({"ok": true}))
                    }
                    ("POST", ["unpublish"]) => {
                        set(s, i, "published", json!(false));
                        (200, json!({"ok": true}))
                    }
                    ("POST", ["hidden"]) => (200, json!({"ok": true, "hidden": b["names"]})),
                    ("POST", ["slideshow"]) => (200, json!({"ok": true, "renderId": "r1"})),
                    _ => (404, json!({"ok": false, "error": "no route"})),
                }
            }
            ("GET", ["v1", "slideshows", r]) => (200, json!({"ok": true, "renderId": r, "state": "done", "path": "/tmp/show.mp4"})),
            ("GET", ["v1", "clientbook"]) => {
                (200, json!({"ok": true, "clients": [{"id": "c1", "name": "Stand-in Client", "shoots": 1}], "pending": []}))
            }
            _ => (404, json!({"ok": false, "error": "no route"})),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shoot_folder_from_photo_paths() {
        let fb = Path::new("/fallback");
        let p = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            gallery_folder_for(&p(&["/Volumes/X/2026/Erika/raw/GR3/a.dng", "/Volumes/X/2026/Erika/raw/X-T2/b.raf"]), "Erika", fb),
            PathBuf::from("/Volumes/X/2026/Erika/gallery/Erika")
        );
        assert_eq!(
            gallery_folder_for(&p(&["/Volumes/X/2026/Erika/raw/GR3/a.dng"]), "Picks", fb),
            PathBuf::from("/Volumes/X/2026/Erika/gallery/Picks")
        );
        assert_eq!(gallery_folder_for(&p(&["/Users/b/Shoots/Ann/a.jpg"]), "A/B", fb), PathBuf::from("/Users/b/Shoots/Ann/gallery/A-B"));
        assert_eq!(gallery_folder_for(&p(&["/a.jpg", "/Volumes/Y/b.jpg"]), "G", fb), PathBuf::from("/fallback/G"));
        assert_eq!(gallery_folder_for(&[], "", fb), PathBuf::from("/fallback/Gallery"));
    }

    #[test]
    fn replies_parse_plain_chunked_and_junk() {
        let r = parse_reply(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\n\r\n{\"ok\":true}").unwrap();
        assert_eq!((r.status, r.body.as_slice()), (200, b"{\"ok\":true}".as_slice()));
        let r = parse_reply(b"HTTP/1.1 422 X\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"ok\r\n6\r\n\":true\r\n1\r\n}\r\n0\r\n\r\n").unwrap();
        assert_eq!((r.status, r.body.as_slice()), (422, b"{\"ok\":true}".as_slice()));
        assert!(parse_reply(b"garbage").is_none());
        assert!(parse_reply(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n").is_none());
        // a length past the end is cut at the end, never a panic
        assert_eq!(parse_reply(b"HTTP/1.1 200 OK\r\nContent-Length: 99\r\n\r\nab").unwrap().body, b"ab");
    }

    #[test]
    fn discovery_needs_a_port_and_a_clean_token() {
        let dir = std::env::temp_dir().join(format!("lc-gal-disc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(matches!(Api::discover(&dir), Err(ApiError::Offline(_))));
        std::fs::write(dir.join(DISCOVERY_FILE), "{not json").unwrap();
        assert!(matches!(Api::discover(&dir), Err(ApiError::Offline(_))));
        std::fs::write(dir.join(DISCOVERY_FILE), r#"{"port": 70000, "token": "abc"}"#).unwrap();
        assert!(Api::discover(&dir).is_err());
        std::fs::write(dir.join(DISCOVERY_FILE), r#"{"port": 8796, "token": "ab\r\nX: y"}"#).unwrap();
        assert!(Api::discover(&dir).is_err(), "a token that would inject a header is refused");
        std::fs::write(dir.join(DISCOVERY_FILE), r#"{"port": 8796, "token": "abc123"}"#).unwrap();
        assert_eq!(Api::discover(&dir).unwrap().port, 8796);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn offline_records_drop_the_key_and_password() {
        let g = public_record(json!({"id": "a", "token": "secret", "password": "pw", "selected": ["x.jpg"]}));
        assert!(g.get("token").is_none() && g.get("password").is_none());
        assert_eq!(g["hasPassword"], true);
        assert_eq!(g["picks"], 1);
        assert_eq!(enc("a b/ç"), "a%20b%2F%C3%A7");
    }
}
