//! LR-MAP-PLACES: places and suggested keywords for photos, albums and folders.
//!
//! **Place search** ([`lookup`]): typed coordinates (`47.6062, -122.3321`, `47°36'22"N 122°19'55"W`)
//! are read locally; names of businesses, landmarks, towns and street addresses go to a
//! [`Geocoder`]. The app's geocoder is OpenStreetMap's Nominatim ([`Nominatim`]) over HTTPS: the
//! request carries the typed text and nothing else about the user or the library (the HTTP client
//! sends `User-Agent: LightCraft/<version>` and no cookies), at most one request a second (the
//! Nominatim usage policy), and every answer is kept on disk ([`cache_path`]) so a place looked up
//! once is found again offline. Typed coordinates are looked up the other way round (reverse
//! geocoding) to fill in the city and country; offline they are used as they are.
//!
//! **A place for an album or folder** ([`ScopeInfo`], kept in the catalog under [`SCOPES_KEY`],
//! so it is undoable and saved with the library): `location.apply` with an album or folder
//! remembers the place, and photos later imported into that album or folder get it ([`after_import`];
//! Settings ▸ Import ▸ Places and keywords).
//!
//! **Suggested keywords** ([`suggestions`]): from the place (venue, city, state, country, kind of
//! place), from the album's or folder's name ("*C&J WEDDING" → wedding), and from what the photos
//! show: the AI search index's CLIP embeddings ([`crate::vision`]) compared with a fixed list of
//! subjects ([`CONTENT_LABELS`]). Nothing leaves the machine for these. Suggestions the user drops
//! are remembered per album / folder and not offered again.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use lightcraft_catalog::{AlbumId, Catalog, Filter, Meta, Op, PhotoId, Sort, Source};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

/// Where the places of albums and folders (and their dropped suggestions) are kept in the catalog.
pub const SCOPES_KEY: &str = "places.scopes";
/// The geocoding service (OpenStreetMap Nominatim).
pub const NOMINATIM: &str = "https://nominatim.openstreetmap.org";
/// Shown with search results (required by the OpenStreetMap licence).
pub const ATTRIBUTION: &str = "Search © OpenStreetMap contributors (Nominatim)";
/// Cached answers older than this are asked again (when online).
const CACHE_MAX_AGE_S: u64 = 180 * 24 * 3600;
/// Answers kept on disk (the oldest go first).
const CACHE_MAX: usize = 4000;
/// Largest answer read from the geocoder.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const MAX_BODY: usize = 2 << 20;
/// Longest query accepted.
const MAX_QUERY: usize = 300;

// ------------------------------------------------------------------------------------- places

/// A place a photo can be given: its position and the IPTC location fields.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Place {
    /// What the list shows first: "Pike Place Market", "Seattle", "47.60620, -122.33210".
    pub name: String,
    /// The whole address line, for telling candidates apart.
    pub address: String,
    pub lat: f64,
    pub lon: f64,
    /// Sublocation (IPTC Location): a venue, landmark or street address; empty for a town or region.
    pub location: String,
    pub city: String,
    pub state: String,
    pub country: String,
    /// What kind of place it is (OpenStreetMap's type: `restaurant`, `park`, `stadium`…).
    pub kind: String,
}

impl Place {
    /// Bare coordinates (no address known).
    pub fn at(lat: f64, lon: f64) -> Place {
        Place { name: format!("{lat:.5}, {lon:.5}"), lat, lon, ..Default::default() }
    }

    /// A place from command params (`{name?, lat, lon, location?, city?, state?, country?, kind?}`).
    pub fn from_json(v: &Value) -> std::result::Result<Place, String> {
        let mut p: Place =
            serde_json::from_value(v.clone()).map_err(|e| format!("a place is {{name, lat, lon, location?, city?, state?, country?}}: {e}"))?;
        if crate::map::located(Some((p.lat, p.lon))).is_none() {
            return Err(format!("{}, {} isn't a place on Earth (latitude ±90°, longitude ±180°, not 0, 0)", p.lat, p.lon));
        }
        for f in [&mut p.name, &mut p.address, &mut p.location, &mut p.city, &mut p.state, &mut p.country, &mut p.kind] {
            *f = f.trim().chars().take(MAX_QUERY).collect();
        }
        if p.name.is_empty() {
            p.name = [&p.location, &p.city, &p.state, &p.country]
                .iter()
                .find(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| Place::at(p.lat, p.lon).name);
        }
        Ok(p)
    }

    /// "Pike Place Market, Seattle, Washington, United States" (the parts it has).
    pub fn summary(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for s in [&self.location, &self.city, &self.state, &self.country] {
            if !s.is_empty() && !parts.iter().any(|x| x.eq_ignore_ascii_case(s)) {
                parts.push(s);
            }
        }
        match parts.is_empty() {
            false => parts.join(", "),
            true if self.name.is_empty() && crate::map::located(Some((self.lat, self.lon))).is_some() => Place::at(self.lat, self.lon).name,
            true => self.name.clone(),
        }
    }

    /// Keywords the place gives a photo: the venue, the city, the state, the country and what kind
    /// of place it is (when that says something: `restaurant`, `beach`, not `house` or `yes`).
    pub fn keywords(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut add = |k: &str| {
            let k = lightcraft_catalog::keywords::clean(k);
            if !k.is_empty() && k.chars().count() <= 80 && !out.iter().any(|x| x.eq_ignore_ascii_case(&k)) {
                out.push(k);
            }
        };
        // a street address is no keyword: nobody searches for a house number
        if !self.location.is_empty() && !self.location.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            add(&self.location);
        }
        add(&self.city);
        add(&self.state);
        add(&self.country);
        if let Some(k) = kind_keyword(&self.kind) {
            add(k);
        }
        out
    }

    /// The photo's metadata with this place: GPS and the four location fields.
    pub fn apply_to(&self, m: &mut Meta) {
        m.gps = Some((self.lat, self.lon));
        m.location = self.location.clone();
        m.city = self.city.clone();
        m.state = self.state.clone();
        m.country = self.country.clone();
    }

    /// The place a photo's own metadata names (its fields and GPS), if any.
    pub fn of_meta(m: &Meta) -> Option<Place> {
        let (lat, lon) = crate::map::located(m.gps).unwrap_or((f64::NAN, f64::NAN));
        let p = Place {
            name: String::new(),
            address: String::new(),
            lat,
            lon,
            location: m.location.trim().to_string(),
            city: m.city.trim().to_string(),
            state: m.state.trim().to_string(),
            country: m.country.trim().to_string(),
            kind: String::new(),
        };
        let has_text = [&p.location, &p.city, &p.state, &p.country].iter().any(|s| !s.is_empty());
        (has_text || lat.is_finite()).then_some(p)
    }
}

/// OpenStreetMap place types worth a keyword (the rest — `yes`, `house`, `administrative` — say
/// nothing a person would search for).
fn kind_keyword(kind: &str) -> Option<&'static str> {
    const KINDS: &[(&str, &str)] = &[
        ("restaurant", "restaurant"),
        ("cafe", "cafe"),
        ("bar", "bar"),
        ("pub", "pub"),
        ("brewery", "brewery"),
        ("winery", "winery"),
        ("fast_food", "restaurant"),
        ("hotel", "hotel"),
        ("museum", "museum"),
        ("gallery", "art gallery"),
        ("arts_centre", "arts centre"),
        ("theatre", "theater"),
        ("cinema", "cinema"),
        ("stadium", "stadium"),
        ("sports_centre", "sports"),
        ("pitch", "sports"),
        ("skatepark", "skatepark"),
        ("park", "park"),
        ("garden", "garden"),
        ("nature_reserve", "nature reserve"),
        ("national_park", "national park"),
        ("beach", "beach"),
        ("peak", "mountain"),
        ("volcano", "volcano"),
        ("waterfall", "waterfall"),
        ("lake", "lake"),
        ("river", "river"),
        ("bay", "bay"),
        ("island", "island"),
        ("viewpoint", "viewpoint"),
        ("attraction", "attraction"),
        ("zoo", "zoo"),
        ("aquarium", "aquarium"),
        ("theme_park", "theme park"),
        ("place_of_worship", "church"),
        ("church", "church"),
        ("cathedral", "cathedral"),
        ("temple", "temple"),
        ("mosque", "mosque"),
        ("castle", "castle"),
        ("monument", "monument"),
        ("memorial", "memorial"),
        ("ruins", "ruins"),
        ("lighthouse", "lighthouse"),
        ("bridge", "bridge"),
        ("marina", "marina"),
        ("harbour", "harbor"),
        ("airport", "airport"),
        ("aerodrome", "airport"),
        ("station", "train station"),
        ("university", "university"),
        ("college", "college"),
        ("school", "school"),
        ("library", "library"),
        ("marketplace", "market"),
        ("mall", "mall"),
        ("events_venue", "event venue"),
        ("community_centre", "community center"),
        ("music_venue", "music venue"),
        ("nightclub", "nightclub"),
        ("camp_site", "camping"),
        ("golf_course", "golf"),
        ("ski", "skiing"),
        ("farm", "farm"),
        ("vineyard", "vineyard"),
    ];
    let k = kind.trim().to_ascii_lowercase();
    KINDS.iter().find(|(t, _)| *t == k).map(|(_, kw)| *kw)
}

/// Typed coordinates: decimal degrees (`47.6062, -122.3321`, `47.6062 -122.3321`) or degrees,
/// minutes and seconds (`47°36'22"N 122°19'55"W`). `None` for anything with other characters
/// (a name or an address), so "Studio 54" or "Route 66" are searched, not read as numbers.
pub fn parse_coordinates(q: &str) -> Option<(f64, f64)> {
    let t = q.trim();
    if t.is_empty() || t.len() > 120 {
        return None;
    }
    let allowed = |c: char| c.is_ascii_digit() || c.is_whitespace() || ".,;-+°º'\"′″’”NSEWnsew".contains(c);
    if !t.chars().all(allowed) || t.chars().filter(|c| c.is_ascii_digit()).count() < 2 {
        return None;
    }
    // (a semicolon separates as a comma does)
    let t = t.replace(';', ",");
    // decimal degrees with signs and a comma, else the shared reader (hemisphere letters, DMS)
    let (lat, lon) = crate::cmd::library::parse_gps(&t)?;
    crate::map::valid(lat, lon).then_some((lat, lon))
}

// ------------------------------------------------------------------------------- geocoders

/// Something that finds places by name and names places by position.
pub trait Geocoder: Send + Sync {
    /// Places matching `q`, best first (at most `limit`).
    fn search(&self, q: &str, limit: usize) -> std::result::Result<Vec<Place>, String>;
    /// The address at a position (`None`: nothing there, e.g. the open sea).
    fn reverse(&self, lat: f64, lon: f64) -> std::result::Result<Option<Place>, String>;
    /// For messages: "OpenStreetMap Nominatim".
    fn name(&self) -> &str;
}

/// No geocoder (the web build, `LIGHTCRAFT_GEOCODE_OFFLINE`): coordinates still work, and cached
/// answers are still found.
pub struct Offline;

impl Geocoder for Offline {
    fn search(&self, _: &str, _: usize) -> std::result::Result<Vec<Place>, String> {
        Err("place search is offline: type coordinates (47.6062, -122.3321), or look the place up once while online".into())
    }
    fn reverse(&self, _: f64, _: f64) -> std::result::Result<Option<Place>, String> {
        Err("place search is offline".into())
    }
    fn name(&self) -> &str {
        "offline"
    }
}

/// OpenStreetMap's Nominatim (<https://nominatim.org>), following its usage policy: one request
/// a second at most, an identifying user agent (`LightCraft/<version>`, nothing personal), every
/// answer cached ([`lookup`]), nothing searched as the user types (only on `find.`).
pub struct Nominatim {
    pub base: String,
}

impl Default for Nominatim {
    fn default() -> Self {
        Nominatim { base: NOMINATIM.into() }
    }
}

/// Percent-encode a query value (RFC 3986 unreserved characters stay).
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

impl Nominatim {
    #[cfg(not(target_arch = "wasm32"))]
    fn get(&self, path_and_query: &str) -> std::result::Result<Value, String> {
        use lightcraft_fetch::http;
        static LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);
        static CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        // one request a second, whoever asks (the lock is held while waiting)
        let mut last = LAST.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(t) = *last {
            let wait = std::time::Duration::from_millis(1100).saturating_sub(t.elapsed());
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
        }
        *last = Some(std::time::Instant::now());
        let limits = http::Limits { connect: std::time::Duration::from_secs(8), stall: std::time::Duration::from_secs(15), cancel: &CANCEL };
        let url = http::Url::parse(&format!("{}{path_and_query}", self.base.trim_end_matches('/'))).map_err(|e| e.to_string())?;
        let headers = [("Accept", "application/json".to_string()), ("Accept-Language", "en".to_string())];
        let mut r = http::get(&url, &headers, &limits).map_err(|e| format!("place search: {e}"))?;
        drop(last);
        if r.status != 200 {
            return Err(format!("place search: the server answered {}", r.status));
        }
        let mut body = Vec::new();
        let mut buf = [0u8; 16 << 10];
        loop {
            let n = r.read(&mut buf, &limits).map_err(|e| format!("place search: {e}"))?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(buf.get(..n).unwrap_or_default());
            if body.len() > MAX_BODY {
                return Err("place search: the answer is larger than 2 MB".into());
            }
        }
        serde_json::from_slice(&body).map_err(|e| format!("place search: an answer that isn't JSON ({e})"))
    }

    #[cfg(target_arch = "wasm32")]
    fn get(&self, _: &str) -> std::result::Result<Value, String> {
        Err("place search isn't available here".into())
    }
}

impl Geocoder for Nominatim {
    fn search(&self, q: &str, limit: usize) -> std::result::Result<Vec<Place>, String> {
        let v = self.get(&format!("/search?format=jsonv2&addressdetails=1&limit={}&q={}", limit.clamp(1, 20), url_encode(q)))?;
        Ok(v.as_array().map(|a| a.iter().filter_map(place_from_nominatim).collect()).unwrap_or_default())
    }
    fn reverse(&self, lat: f64, lon: f64) -> std::result::Result<Option<Place>, String> {
        let v = self.get(&format!("/reverse?format=jsonv2&addressdetails=1&zoom=18&lat={lat:.6}&lon={lon:.6}"))?;
        if v.get("error").is_some() {
            return Ok(None);
        }
        Ok(place_from_nominatim(&v))
    }
    fn name(&self) -> &str {
        "OpenStreetMap Nominatim"
    }
}

/// One Nominatim (`format=jsonv2`, `addressdetails=1`) result as a [`Place`].
pub fn place_from_nominatim(v: &Value) -> Option<Place> {
    let num = |k: &str| v.get(k).and_then(|x| x.as_str().and_then(|s| s.trim().parse::<f64>().ok()).or_else(|| x.as_f64()));
    let (lat, lon) = crate::map::located(Some((num("lat")?, num("lon")?)))?;
    let a = v.get("address").cloned().unwrap_or(Value::Null);
    let s = |k: &str| a.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let first = |keys: &[&str]| keys.iter().find_map(|k| s(k)).unwrap_or_default();
    let city = first(&["city", "town", "village", "hamlet", "municipality", "suburb", "county"]);
    let state = first(&["state", "province", "region", "state_district"]);
    let country = first(&["country"]);
    let name = v.get("name").and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
    let addresstype = v.get("addresstype").and_then(Value::as_str).unwrap_or_default();
    let kind = v.get("type").and_then(Value::as_str).unwrap_or_default().to_string();
    let area = matches!(
        addresstype,
        "city"
            | "town"
            | "village"
            | "hamlet"
            | "municipality"
            | "county"
            | "state"
            | "province"
            | "region"
            | "country"
            | "postcode"
            | "suburb"
            | "state_district"
            | "continent"
    );
    let street = match (s("house_number"), s("road")) {
        (Some(n), Some(r)) => format!("{n} {r}"),
        (None, Some(r)) => r,
        _ => String::new(),
    };
    let location = if area {
        String::new()
    } else if !name.is_empty() && !name.eq_ignore_ascii_case(&city) {
        name.clone()
    } else {
        street
    };
    let address = v.get("display_name").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let name = if !name.is_empty() {
        name
    } else if !location.is_empty() {
        location.clone()
    } else {
        address.split(',').next().unwrap_or_default().trim().to_string()
    };
    let name = if name.is_empty() { Place::at(lat, lon).name } else { name };
    Some(Place { name, address, lat, lon, location, city, state, country, kind: if area { String::new() } else { kind } })
}

/// The geocoder the app starts with: Nominatim natively, unless `LIGHTCRAFT_GEOCODE_OFFLINE` is set.
pub fn default_geocoder() -> Arc<dyn Geocoder> {
    if cfg!(target_arch = "wasm32") || std::env::var_os("LIGHTCRAFT_GEOCODE_OFFLINE").is_some() {
        Arc::new(Offline)
    } else {
        Arc::new(Nominatim::default())
    }
}

// ------------------------------------------------------------------------------------ cache

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct CacheEntry {
    /// Seconds since 1970 when it was asked.
    at: u64,
    places: Vec<Place>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct CacheFile {
    entries: BTreeMap<String, CacheEntry>,
}

/// Every cache read-modify-write goes one at a time.
static CACHE_LOCK: Mutex<()> = Mutex::new(());

fn now_s() -> u64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn cache_read(path: &Path) -> CacheFile {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// A cached answer (`fresh`: younger than the cache's age limit).
fn cache_get(path: Option<&Path>, key: &str) -> Option<(Vec<Place>, bool)> {
    let path = path?;
    let _g = CACHE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let e = cache_read(path).entries.remove(key)?;
    Some((e.places, now_s().saturating_sub(e.at) < CACHE_MAX_AGE_S))
}

fn cache_put(path: Option<&Path>, key: &str, places: &[Place]) {
    let Some(path) = path else { return };
    let _g = CACHE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let mut f = cache_read(path);
    f.entries.insert(key.to_string(), CacheEntry { at: now_s(), places: places.to_vec() });
    while f.entries.len() > CACHE_MAX {
        let Some(oldest) = f.entries.iter().min_by_key(|(_, e)| e.at).map(|(k, _)| k.clone()) else { break };
        f.entries.remove(&oldest);
    }
    let Ok(bytes) = serde_json::to_vec(&f) else { return };
    // written whole, then renamed: a crash never leaves half a cache
    let tmp = path.with_extension("part");
    let ok = path.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok())
        && std::fs::write(&tmp, bytes).is_ok()
        && std::fs::rename(&tmp, path).is_ok();
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        log::info!("place search: can't write the cache {}", path.display());
    }
}

fn query_key(q: &str) -> String {
    format!("search:{}", q.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase())
}

/// Where answers are cached: the library's `geocode-cache.json`, else the user's cache folder.
pub fn cache_path(s: &Session) -> Option<PathBuf> {
    if cfg!(target_arch = "wasm32") {
        return None;
    }
    if let Some(p) = &s.places.cache_file {
        return Some(p.clone());
    }
    if let Some(l) = s.library.as_ref().filter(|l| l.on_disk) {
        return Some(l.dir.join("geocode-cache.json"));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches/LightCraft/geocode-cache.json"))
}

/// Where a search's answer came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Found {
    /// Typed coordinates (with the address found for them, when there was one).
    Coordinates,
    Cache,
    Geocoder,
}

/// Search for a place: coordinates are read locally (and named by reverse geocoding when
/// possible), names and addresses come from the cache or the geocoder. Blocking (network): the app
/// runs it on a worker thread.
pub fn lookup(geo: &dyn Geocoder, cache: Option<&Path>, q: &str, limit: usize) -> std::result::Result<(Vec<Place>, Found), String> {
    let q = q.trim();
    if q.is_empty() {
        return Err("type a place, a business, an address or coordinates (47.6062, -122.3321)".into());
    }
    if q.chars().count() > MAX_QUERY {
        return Err(format!("a search is at most {MAX_QUERY} characters"));
    }
    let limit = limit.clamp(1, 20);
    if let Some((lat, lon)) = parse_coordinates(q) {
        if crate::map::located(Some((lat, lon))).is_none() {
            return Err("0, 0 is no place (it's what cameras write without a GPS fix)".into());
        }
        let key = format!("reverse:{lat:.5},{lon:.5}");
        let named = match cache_get(cache, &key) {
            Some((p, _)) => p.into_iter().next(),
            None => match geo.reverse(lat, lon) {
                Ok(p) => {
                    cache_put(cache, &key, &p.clone().into_iter().collect::<Vec<_>>());
                    p
                }
                Err(e) => {
                    log::info!("{e}");
                    None
                }
            },
        };
        let mut out = Vec::new();
        if let Some(mut p) = named {
            // the typed position wins over the address's own centre
            (p.lat, p.lon) = (lat, lon);
            out.push(p);
        }
        out.push(Place::at(lat, lon));
        return Ok((out, Found::Coordinates));
    }
    let key = query_key(q);
    let cached = cache_get(cache, &key);
    if let Some((p, true)) = &cached {
        return Ok((p.iter().take(limit).cloned().collect(), Found::Cache));
    }
    match geo.search(q, limit) {
        Ok(p) => {
            cache_put(cache, &key, &p);
            Ok((p, Found::Geocoder))
        }
        // offline: an old answer is better than none
        Err(e) => match cached {
            Some((p, _)) => Ok((p.into_iter().take(limit).collect(), Found::Cache)),
            None => Err(e),
        },
    }
}

// ----------------------------------------------------------------------- session state

/// Text embeddings of [`CONTENT_LABELS`] (keyword, embedding), computed once.
pub type LabelVectors = Arc<Vec<(String, Vec<f32>)>>;

/// The session's place search and keyword suggestion state.
pub struct Places {
    pub geocoder: Arc<dyn Geocoder>,
    /// The search cache file (`None`: [`cache_path`]'s default).
    pub cache_file: Option<PathBuf>,
    /// The content labels' CLIP text embeddings (set by tests; otherwise computed on first use).
    pub labels: Option<LabelVectors>,
    /// Suggestions dropped for a set of photos that is no album or folder (this session only).
    pub dropped: Vec<String>,
}

impl Default for Places {
    fn default() -> Self {
        Places { geocoder: default_geocoder(), cache_file: None, labels: None, dropped: Vec::new() }
    }
}

// ------------------------------------------------------------------------------------ scopes

/// What a place or a keyword suggestion is for.
#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    /// These photos (the selection).
    Photos(Vec<PhotoId>),
    /// An album (collection).
    Album(AlbumId),
    /// A folder the library's photos were imported from (with its subfolders).
    Folder(String),
}

impl Scope {
    /// `{album: id}` | `{folder: path}` | `{ids: [..]}` | nothing (the selection).
    pub fn from_params(s: &Session, p: &Value, c: &str) -> Result<Scope> {
        if let Some(a) = p.get("album").filter(|v| !v.is_null()) {
            let id = a.as_u64().map(AlbumId).ok_or_else(|| bad(c, "`album` is an album id"))?;
            let al = s.catalog.album(id).ok_or_else(|| bad(c, format!("no album {}", id.0)))?;
            if al.folder {
                return Err(bad(c, format!("“{}” is a folder of albums, not an album", al.name)));
            }
            return Ok(Scope::Album(id));
        }
        if let Some(f) = p.get("folder").filter(|v| !v.is_null()) {
            let f = f.as_str().map(str::trim).filter(|f| !f.is_empty()).ok_or_else(|| bad(c, "`folder` is a folder path"))?;
            return Ok(Scope::Folder(f.to_string()));
        }
        let ids = s.targets(p);
        if ids.is_empty() {
            return Err(bad(c, "no photos: select some, or name an `album` or a `folder`"));
        }
        Ok(Scope::Photos(ids))
    }

    /// The album or folder the current Library source shows, if it is one.
    pub fn of_source(s: &Session) -> Option<Scope> {
        match s.source {
            crate::LibrarySource::Album(id) if s.catalog.album(id).is_some_and(|a| !a.folder) => Some(Scope::Album(id)),
            crate::LibrarySource::LibraryFolder => s.library_folder.clone().filter(|f| !f.is_empty() && f != ".").map(Scope::Folder),
            _ => None,
        }
    }

    /// The key its [`ScopeInfo`] is kept under (`None` for a set of photos).
    pub fn key(&self) -> Option<String> {
        match self {
            Scope::Photos(_) => None,
            Scope::Album(id) => Some(format!("album:{}", id.0)),
            Scope::Folder(f) => Some(format!("folder:{}", lightcraft_catalog::query::folder_key(f))),
        }
    }

    /// The library photos it holds.
    pub fn photos(&self, c: &Catalog) -> Vec<PhotoId> {
        let ids = match self {
            Scope::Photos(ids) => ids.clone(),
            Scope::Album(id) => c.album_photos(*id),
            Scope::Folder(f) => {
                c.query(&Filter { library_folder: Some(f.clone()), library_folder_only: false, ..Default::default() }, &Sort::default())
            }
        };
        ids.into_iter().filter(|id| c.photo(*id).is_some_and(|p| p.in_library())).collect()
    }

    /// Its name, for the panel and messages ("C&J WEDDING", "2024-06 Seattle", "3 photos").
    pub fn name(&self, c: &Catalog) -> String {
        match self {
            Scope::Photos(ids) => format!("{} photo{}", ids.len(), if ids.len() == 1 { "" } else { "s" }),
            Scope::Album(id) => c.album(*id).map(|a| a.name.clone()).unwrap_or_default(),
            Scope::Folder(f) => Path::new(f).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| f.clone()),
        }
    }

    pub fn json(&self, c: &Catalog) -> Value {
        match self {
            Scope::Photos(ids) => json!({"kind": "photos", "photos": ids.len(), "name": self.name(c)}),
            Scope::Album(id) => json!({"kind": "album", "album": id.0, "name": self.name(c)}),
            Scope::Folder(f) => json!({"kind": "folder", "folder": f, "name": self.name(c)}),
        }
    }
}

fn bad(c: &str, why: impl Into<String>) -> EngineError {
    crate::cmd::bad(c, why)
}

/// An album's or folder's place and the suggestions dropped for it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ScopeInfo {
    /// New photos imported into it get this place (and its keywords).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<Place>,
    /// Suggested keywords the user dropped (`−`): not offered again here.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dropped: Vec<String>,
}

impl ScopeInfo {
    fn is_empty(&self) -> bool {
        self.place.is_none() && self.dropped.is_empty()
    }
}

/// Every album's and folder's [`ScopeInfo`], by key (entries that don't parse are skipped).
pub fn scopes(c: &Catalog) -> BTreeMap<String, ScopeInfo> {
    c.record(SCOPES_KEY)
        .and_then(Value::as_object)
        .map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), serde_json::from_value::<ScopeInfo>(v.clone()).ok()?))).collect())
        .unwrap_or_default()
}

pub fn scope_info(c: &Catalog, key: &str) -> ScopeInfo {
    scopes(c).remove(key).unwrap_or_default()
}

/// The op that changes one scope's info (empty info is removed).
pub fn scope_op(c: &Catalog, key: &str, f: impl FnOnce(&mut ScopeInfo)) -> Op {
    let mut all = scopes(c);
    let mut info = all.remove(key).unwrap_or_default();
    f(&mut info);
    if !info.is_empty() {
        all.insert(key.to_string(), info);
    }
    let value = if all.is_empty() { None } else { serde_json::to_value(&all).ok() };
    Op::SetRecord { key: SCOPES_KEY.into(), value }
}

/// The place new photos get: an album's they were imported into first, else the deepest folder's
/// that holds them (with its key).
pub fn default_place_for(c: &Catalog, id: PhotoId, album: Option<AlbumId>, all: &BTreeMap<String, ScopeInfo>) -> Option<(String, Place)> {
    if let Some(a) = album {
        let k = format!("album:{}", a.0);
        if let Some(p) = all.get(&k).and_then(|i| i.place.clone()) {
            return Some((k, p));
        }
    }
    let photo = c.photo(id)?;
    let Source::File { path } = &photo.source else { return None };
    all.iter()
        .filter_map(|(k, i)| Some((k, k.strip_prefix("folder:")?, i.place.as_ref()?)))
        .filter(|(_, root, _)| lightcraft_catalog::query::folder_within(path, root))
        .max_by_key(|(_, root, _)| root.len())
        .map(|(k, _, p)| (k.clone(), p.clone()))
}

// --------------------------------------------------------------------------- applying places

/// SetMeta ops for `ids`, each photo's metadata changed by `f` (photos it doesn't change are left out).
pub fn meta_ops(c: &Catalog, ids: &[PhotoId], mut f: impl FnMut(&mut Meta)) -> Vec<Op> {
    ids.iter()
        .filter_map(|id| {
            let p = c.photo(*id)?;
            let mut m = p.meta.clone();
            f(&mut m);
            (m != p.meta).then(|| Op::SetMeta { id: *id, meta: Box::new(m) })
        })
        .collect()
}

fn add_keywords(m: &mut Meta, add: &[String]) {
    for k in add {
        if !m.keywords.iter().any(|x| x.eq_ignore_ascii_case(k)) {
            m.keywords.push(k.clone());
        }
    }
}

/// Commit `ops` as one undo step; keyword changes reach face names and Recent Keywords like
/// `photo.setMeta`'s.
pub(crate) fn commit_meta(s: &mut Session, label: &str, ops: Vec<Op>, added: &[String]) -> Result<()> {
    if ops.is_empty() {
        return Ok(());
    }
    let op = s.sync_faces_with_keywords(Op::Batch { ops });
    s.commit(label, op)?;
    if !added.is_empty() {
        crate::cmd::keywords::note_recent(s, added);
    }
    Ok(())
}

/// `location.apply`: the place on every photo of the scope (GPS + location fields, its keywords
/// when `keywords`), and as the album's / folder's place when `remember`. One undo step.
pub fn apply(s: &mut Session, scope: &Scope, place: &Place, keywords: bool, remember: bool) -> Result<Value> {
    let ids = scope.photos(&s.catalog);
    let kws = if keywords { place.keywords() } else { Vec::new() };
    let mut ops = meta_ops(&s.catalog, &ids, |m| {
        place.apply_to(m);
        add_keywords(m, &kws);
    });
    let key = scope.key().filter(|_| remember);
    if let Some(k) = &key {
        ops.push(scope_op(&s.catalog, k, |i| i.place = Some(place.clone())));
    }
    let changed = ops.len().saturating_sub(usize::from(key.is_some()));
    commit_meta(s, "Set Location", ops, &kws)?;
    Ok(json!({"photos": ids.len(), "changed": changed, "place": place, "keywords": kws, "remembered": key.is_some()}))
}

/// After an import: photos that came into an album or folder with a place get it (those without a
/// GPS position of their own), with the place's keywords; with Settings ▸ Import ▸ suggested
/// keywords on, the album's / folder's name keywords too. One undo step ("Place Imported Photos").
pub fn after_import(s: &mut Session, imported: &[PhotoId], album: Option<AlbumId>) -> Result<Value> {
    let d = s.import_defaults.clone();
    if imported.is_empty() || !(d.auto_place || d.auto_keywords) {
        return Ok(Value::Null);
    }
    let all = scopes(&s.catalog);
    let mut ops = Vec::new();
    let mut added: Vec<String> = Vec::new();
    let (mut placed, mut tagged) = (0usize, 0usize);
    let name_kws = if d.auto_keywords { album.map(|a| name_keywords(&Scope::Album(a).name(&s.catalog))).unwrap_or_default() } else { Vec::new() };
    for id in imported {
        let Some(p) = s.catalog.photo(*id) else { continue };
        let mut m = p.meta.clone();
        if d.auto_place
            && let Some((_, place)) = (!all.is_empty()).then(|| default_place_for(&s.catalog, *id, album, &all)).flatten()
        {
            if crate::map::located(m.gps).is_none() {
                place.apply_to(&mut m);
                placed += 1;
            }
            if d.place_keywords {
                let k = place.keywords();
                add_keywords(&mut m, &k);
                added.extend(k);
            }
        }
        if d.auto_keywords {
            let mut k = name_kws.clone();
            if let Source::File { path } = &p.source
                && let Some(dir) = Path::new(path).parent().and_then(|d| d.file_name())
            {
                k.extend(name_keywords(&dir.to_string_lossy()));
            }
            add_keywords(&mut m, &k);
            added.extend(k);
        }
        if m != p.meta {
            tagged += 1;
            ops.push(Op::SetMeta { id: *id, meta: Box::new(m) });
        }
    }
    added.sort();
    added.dedup();
    commit_meta(s, "Place Imported Photos", ops, &added)?;
    Ok(json!({"placed": placed, "changed": tagged, "keywords": added}))
}

// ----------------------------------------------------------------------- keyword suggestions

/// Words in album and folder names that say what the photos are (word → keyword).
const NAME_WORDS: &[(&str, &str)] = &[
    ("wedding", "wedding"),
    ("weddings", "wedding"),
    ("bride", "wedding"),
    ("bridal", "wedding"),
    ("reception", "wedding"),
    ("elopement", "elopement"),
    ("engagement", "engagement"),
    ("engaged", "engagement"),
    ("proposal", "proposal"),
    ("anniversary", "anniversary"),
    ("birthday", "birthday"),
    ("bday", "birthday"),
    ("graduation", "graduation"),
    ("grad", "graduation"),
    ("prom", "prom"),
    ("portrait", "portrait"),
    ("portraits", "portrait"),
    ("headshot", "headshot"),
    ("headshots", "headshot"),
    ("family", "family"),
    ("families", "family"),
    ("newborn", "newborn"),
    ("maternity", "maternity"),
    ("baby", "baby"),
    ("kids", "kids"),
    ("senior", "senior portraits"),
    ("seniors", "senior portraits"),
    ("boudoir", "boudoir"),
    ("fashion", "fashion"),
    ("editorial", "editorial"),
    ("concert", "concert"),
    ("gig", "concert"),
    ("band", "band"),
    ("music", "music"),
    ("festival", "festival"),
    ("fest", "festival"),
    ("party", "party"),
    ("reunion", "reunion"),
    ("memorial", "memorial"),
    ("funeral", "memorial"),
    ("conference", "conference"),
    ("event", "event"),
    ("events", "event"),
    ("skate", "skateboarding"),
    ("skating", "skateboarding"),
    ("skateboarding", "skateboarding"),
    ("skatepark", "skatepark"),
    ("surf", "surfing"),
    ("surfing", "surfing"),
    ("ski", "skiing"),
    ("skiing", "skiing"),
    ("snowboard", "snowboarding"),
    ("snowboarding", "snowboarding"),
    ("hike", "hiking"),
    ("hiking", "hiking"),
    ("camping", "camping"),
    ("fishing", "fishing"),
    ("travel", "travel"),
    ("trip", "travel"),
    ("vacation", "travel"),
    ("holiday", "holiday"),
    ("roadtrip", "road trip"),
    ("christmas", "christmas"),
    ("xmas", "christmas"),
    ("halloween", "halloween"),
    ("thanksgiving", "thanksgiving"),
    ("easter", "easter"),
    ("nye", "new year"),
    ("beach", "beach"),
    ("sunset", "sunset"),
    ("snow", "snow"),
    ("dog", "dog"),
    ("dogs", "dog"),
    ("puppy", "dog"),
    ("cat", "cat"),
    ("cats", "cat"),
    ("pets", "pets"),
    ("wildlife", "wildlife"),
    ("birds", "birds"),
    ("zoo", "zoo"),
    ("food", "food"),
    ("product", "product"),
    ("products", "product"),
    ("realestate", "real estate"),
    ("car", "car"),
    ("cars", "car"),
    ("automotive", "car"),
    ("sports", "sports"),
    ("football", "football"),
    ("basketball", "basketball"),
    ("soccer", "soccer"),
    ("baseball", "baseball"),
    ("hockey", "hockey"),
    ("landscape", "landscape"),
    ("landscapes", "landscape"),
    ("street", "street photography"),
    ("architecture", "architecture"),
    ("interiors", "interior"),
    ("nature", "nature"),
    ("garden", "garden"),
    ("studio", "studio"),
    ("church", "church"),
    ("museum", "museum"),
    ("drone", "aerial"),
    ("aerial", "aerial"),
];

/// Two-word phrases in names (checked before single words).
const NAME_PHRASES: &[(&str, &str)] = &[
    ("real estate", "real estate"),
    ("road trip", "road trip"),
    ("new year", "new year"),
    ("new years", "new year"),
    ("baby shower", "baby shower"),
    ("bridal shower", "bridal shower"),
    ("first birthday", "birthday"),
    ("rehearsal dinner", "wedding"),
];

/// Keywords an album's or folder's name gives ("*C&J WEDDING" → wedding, "2024 Family Beach Trip"
/// → family, beach, travel). Words it doesn't know (names, dates) give nothing.
pub fn name_keywords(name: &str) -> Vec<String> {
    let words: Vec<String> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect();
    let mut out: Vec<String> = Vec::new();
    let mut push = |k: &str| {
        if !out.iter().any(|x| x == k) {
            out.push(k.to_string());
        }
    };
    let mut i = 0;
    while let Some(w) = words.get(i) {
        let pair = words.get(i + 1).map(|b| format!("{w} {b}"));
        if let Some((_, k)) = pair.as_deref().and_then(|p| NAME_PHRASES.iter().find(|(ph, _)| *ph == p)) {
            push(k);
            i += 2;
            continue;
        }
        if let Some((_, k)) = NAME_WORDS.iter().find(|(nw, _)| nw == w) {
            push(k);
        }
        i += 1;
    }
    out
}

/// Subjects the AI search model is asked about: (keyword, phrase). An empty keyword is a
/// background class: it soaks up photos that are none of the others and is never suggested.
pub const CONTENT_LABELS: &[(&str, &str)] = &[
    ("wedding", "a photo of a wedding"),
    ("portrait", "a portrait photo of a person"),
    ("family", "a family photo"),
    ("kids", "a photo of children playing"),
    ("baby", "a photo of a baby"),
    ("couple", "a photo of a couple"),
    ("group photo", "a group photo of many people"),
    ("party", "a photo of a party"),
    ("concert", "a photo of a concert on stage"),
    ("dance", "a photo of people dancing"),
    ("sports", "a photo of people playing sports"),
    ("skateboarding", "a photo of a skateboarder"),
    ("surfing", "a photo of surfing"),
    ("skiing", "a photo of skiing"),
    ("beach", "a photo of a beach"),
    ("ocean", "a photo of the ocean"),
    ("lake", "a photo of a lake"),
    ("waterfall", "a photo of a waterfall"),
    ("mountains", "a photo of mountains"),
    ("forest", "a photo of a forest"),
    ("desert", "a photo of a desert"),
    ("snow", "a photo of snow"),
    ("sunset", "a photo of a sunset"),
    ("night", "a photo taken at night"),
    ("city", "a photo of a city street"),
    ("architecture", "a photo of a building"),
    ("interior", "a photo of a room interior"),
    ("landscape", "a landscape photo"),
    ("flowers", "a photo of flowers"),
    ("park", "a photo of a park"),
    ("food", "a photo of food"),
    ("drinks", "a photo of drinks"),
    ("restaurant", "a photo inside a restaurant"),
    ("car", "a photo of a car"),
    ("motorcycle", "a photo of a motorcycle"),
    ("boat", "a photo of a boat"),
    ("airplane", "a photo of an airplane"),
    ("dog", "a photo of a dog"),
    ("cat", "a photo of a cat"),
    ("birds", "a photo of a bird"),
    ("horse", "a photo of a horse"),
    ("wildlife", "a photo of a wild animal"),
    ("product", "a product photo"),
    ("art", "a photo of a painting"),
    ("church", "a photo of a church"),
    ("festival", "a photo of a festival"),
    ("christmas", "a photo of christmas decorations"),
    ("halloween", "a photo of halloween costumes"),
    ("birthday", "a photo of a birthday cake"),
    ("graduation", "a photo of a graduation ceremony"),
    ("camping", "a photo of a tent while camping"),
    ("hiking", "a photo of people hiking on a trail"),
    ("street photography", "a street photography photo"),
    ("black and white", "a black and white photo"),
    ("aerial", "an aerial photo taken from a drone"),
    ("macro", "a macro close-up photo"),
    ("", "a photo"),
    ("", "a blurry photo"),
    ("", "a screenshot"),
    ("", "a photo of a document"),
];

/// CLIP's logit scale (its similarities × 100 before the softmax).
const LOGIT_SCALE: f32 = 100.0;
/// A subject counts for a photo at this probability or more …
const CONTENT_MIN_P: f32 = 0.25;
/// … and only when the photo resembles its phrase at all (cosine).
const CONTENT_MIN_COS: f32 = 0.18;
/// An album's subject: at least this share of its photos show it.
const CONTENT_SHARE: f32 = 0.3;

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (mut d, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        d += x * y;
        na += x * x;
        nb += y * y;
    }
    let n = (na * nb).sqrt();
    if n > 0.0 && n.is_finite() { d / n } else { 0.0 }
}

/// The subjects one photo shows (keyword, probability), best first, at most three.
pub fn content_scores(image: &[f32], labels: &[(String, Vec<f32>)]) -> Vec<(String, f32)> {
    if image.is_empty() || labels.is_empty() {
        return Vec::new();
    }
    let cos: Vec<f32> = labels.iter().map(|(_, t)| cosine(image, t)).collect();
    let max = cos.iter().copied().fold(f32::MIN, f32::max);
    let exp: Vec<f32> = cos.iter().map(|c| ((c - max) * LOGIT_SCALE).exp()).collect();
    let sum: f32 = exp.iter().sum();
    if !(sum > 0.0 && sum.is_finite()) {
        return Vec::new();
    }
    let mut out: Vec<(String, f32)> = labels
        .iter()
        .zip(cos.iter().zip(&exp))
        .filter(|((k, _), (c, e))| !k.is_empty() && **c >= CONTENT_MIN_COS && **e / sum >= CONTENT_MIN_P)
        .map(|((k, _), (_, e))| (k.clone(), e / sum))
        .collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    out.truncate(3);
    out
}

/// The subjects of a set of photos (keyword, share of photos), from each photo's
/// [`content_scores`]: a subject at least [`CONTENT_SHARE`] of them show (any one, for one photo).
pub fn content_keywords(per_photo: &[Vec<(String, f32)>]) -> Vec<(String, f32)> {
    let n = per_photo.len();
    if n == 0 {
        return Vec::new();
    }
    let mut count: HashMap<&str, usize> = HashMap::new();
    for p in per_photo {
        for (k, _) in p {
            *count.entry(k.as_str()).or_default() += 1;
        }
    }
    let mut out: Vec<(String, f32)> =
        count.into_iter().map(|(k, c)| (k.to_string(), c as f32 / n as f32)).filter(|(_, share)| n == 1 || *share >= CONTENT_SHARE).collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

/// One suggested keyword.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub keyword: String,
    /// `place` | `name` (album / folder name) | `content` (what the photos show).
    pub from: &'static str,
    /// How sure (0–1): share of photos for content, 1 for the others.
    pub score: f32,
    /// How many of the photos have it already.
    pub have: usize,
}

impl Session {
    /// The content labels' text embeddings (computed once with the AI search model).
    fn label_vectors(&mut self) -> std::result::Result<LabelVectors, String> {
        if let Some(l) = &self.places.labels {
            return Ok(l.clone());
        }
        let mut v = Vec::with_capacity(CONTENT_LABELS.len());
        for (k, phrase) in CONTENT_LABELS {
            let e = self.embed_query(phrase).map_err(|e| e.to_string())?;
            v.push((k.to_string(), e));
        }
        let l = Arc::new(v);
        self.places.labels = Some(l.clone());
        Ok(l)
    }

    /// What the photos show, from the AI search index (keyword, share of photos). `Err`: why not
    /// (the model isn't installed, nothing is indexed yet).
    pub fn content_subjects(&mut self, ids: &[PhotoId]) -> std::result::Result<Vec<(String, f32)>, String> {
        let index = self.search_index().map_err(|e| e.to_string())?;
        let embs: Vec<Vec<f32>> = {
            let idx = index.lock().unwrap_or_else(PoisonError::into_inner);
            ids.iter().filter_map(|id| self.catalog.photo(*id)).filter_map(|p| idx.get(&crate::media::content_key(p)).map(|e| e.0.clone())).collect()
        };
        if embs.is_empty() {
            return Err("not indexed for AI search yet (Library menu: Index for AI Search)".into());
        }
        let labels = self.label_vectors()?;
        let per: Vec<Vec<(String, f32)>> = embs.iter().map(|e| content_scores(e, &labels)).collect();
        Ok(content_keywords(&per))
    }
}

/// Keyword suggestions for a scope: `{scope, photos, suggestions: [{keyword, from, score, have}],
/// looksLike: [..], place?, content: "ok" | why not}`. Keywords every photo has are left out,
/// and so are those dropped here before.
pub fn suggestions(s: &mut Session, scope: &Scope, with_content: bool) -> Value {
    let ids = scope.photos(&s.catalog);
    let n = ids.len();
    let info = scope.key().map(|k| scope_info(&s.catalog, &k)).unwrap_or_default();
    let dropped: Vec<String> = if scope.key().is_some() { info.dropped.clone() } else { s.places.dropped.clone() };
    let mut out: Vec<Suggestion> = Vec::new();
    let push = |out: &mut Vec<Suggestion>, k: &str, from: &'static str, score: f32| {
        let k = lightcraft_catalog::keywords::clean(k);
        if k.is_empty() || out.iter().any(|x| x.keyword.eq_ignore_ascii_case(&k)) || dropped.iter().any(|d| d.eq_ignore_ascii_case(&k)) {
            return;
        }
        out.push(Suggestion { keyword: k, from, score, have: 0 });
    };
    // the place: the scope's own, then what the photos' location fields share (a third or more)
    if let Some(p) = &info.place {
        for k in p.keywords() {
            push(&mut out, &k, "place", 1.0);
        }
    }
    let mut fields: HashMap<String, usize> = HashMap::new();
    for id in &ids {
        if let Some(p) = s.catalog.photo(*id).and_then(|p| Place::of_meta(&p.meta)) {
            for k in p.keywords() {
                *fields.entry(k).or_default() += 1;
            }
        }
    }
    let mut fields: Vec<(String, usize)> = fields.into_iter().filter(|(_, c)| *c * 3 >= n.max(1)).collect();
    fields.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (k, c) in fields {
        push(&mut out, &k, "place", c as f32 / n.max(1) as f32);
    }
    // the name: the album's or folder's, else the albums and folders most of the photos are in
    let mut names: Vec<String> = Vec::new();
    match scope {
        Scope::Album(_) | Scope::Folder(_) => names.push(scope.name(&s.catalog)),
        Scope::Photos(_) => {
            let mut seen: HashMap<String, usize> = HashMap::new();
            for id in &ids {
                let Some(p) = s.catalog.photo(*id) else { continue };
                if let Source::File { path } = &p.source
                    && let Some(d) = Path::new(path).parent().and_then(|d| d.file_name())
                {
                    *seen.entry(d.to_string_lossy().to_string()).or_default() += 1;
                }
                for a in s.catalog.albums_of(*id) {
                    if let Some(al) = s.catalog.album(a).filter(|al| !al.is_smart() && !al.quick) {
                        *seen.entry(al.name.clone()).or_default() += 1;
                    }
                }
            }
            names.extend(seen.into_iter().filter(|(_, c)| *c * 2 >= n.max(1)).map(|(k, _)| k));
            names.sort();
        }
    }
    let mut looks: Vec<String> = Vec::new();
    for name in &names {
        for k in name_keywords(name) {
            if !looks.contains(&k) {
                looks.push(k.clone());
            }
            push(&mut out, &k, "name", 1.0);
        }
    }
    // what the photos show (the AI search index; nothing leaves the machine)
    let content = if !with_content {
        "not asked".to_string()
    } else {
        match s.content_subjects(&ids) {
            Ok(c) => {
                for (k, share) in &c {
                    if *share >= 0.5 && looks.len() < 3 && !looks.contains(k) {
                        looks.push(k.clone());
                    }
                    push(&mut out, k, "content", (*share * 100.0).round() / 100.0);
                }
                "ok".to_string()
            }
            Err(e) => e,
        }
    };
    for sug in &mut out {
        sug.have = ids
            .iter()
            .filter(|id| s.catalog.photo(**id).is_some_and(|p| p.meta.keywords.iter().any(|k| k.eq_ignore_ascii_case(&sug.keyword))))
            .count();
    }
    json!({
        "scope": scope.json(&s.catalog),
        "photos": n,
        "suggestions": out,
        "looksLike": looks,
        "place": info.place,
        "dropped": dropped,
        "content": content,
    })
}

/// `keywords.applySuggested` / `keywords.dropSuggested`: add `add` to every photo of the scope,
/// take `remove` off them (and drop those from its suggestions, so they aren't offered again).
/// One undo step.
pub fn apply_suggested(s: &mut Session, scope: &Scope, add: &[String], remove: &[String], drop: &[String]) -> Result<Value> {
    let ids = scope.photos(&s.catalog);
    let add: Vec<String> = add.iter().map(|k| lightcraft_catalog::keywords::clean(k)).filter(|k| !k.is_empty()).collect();
    let remove: Vec<String> = remove.iter().map(|k| lightcraft_catalog::keywords::clean(k)).filter(|k| !k.is_empty()).collect();
    let mut ops = meta_ops(&s.catalog, &ids, |m| {
        add_keywords(m, &add);
        m.keywords.retain(|k| !remove.iter().any(|r| r.eq_ignore_ascii_case(k)));
    });
    let changed = ops.len();
    let mut forget: Vec<String> = remove.iter().chain(drop).map(|k| lightcraft_catalog::keywords::clean(k)).filter(|k| !k.is_empty()).collect();
    forget.dedup();
    // an added keyword is no longer dropped
    let restore = |list: &mut Vec<String>| {
        list.retain(|d| !add.iter().any(|a| a.eq_ignore_ascii_case(d)));
        for k in &forget {
            if !list.iter().any(|d| d.eq_ignore_ascii_case(k)) {
                list.push(k.clone());
            }
        }
    };
    match scope.key() {
        Some(k) => {
            let info = scope_info(&s.catalog, &k);
            let mut list = info.dropped.clone();
            restore(&mut list);
            if list != info.dropped {
                ops.push(scope_op(&s.catalog, &k, |i| i.dropped = list));
            }
        }
        None => restore(&mut s.places.dropped),
    }
    if ops.is_empty() {
        return Ok(json!({"photos": ids.len(), "changed": 0, "added": add, "removed": remove}));
    }
    let label = if add.is_empty() { "Remove Suggested Keywords" } else { "Add Suggested Keywords" };
    commit_meta(s, label, ops, &add)?;
    Ok(json!({"photos": ids.len(), "changed": changed, "added": add, "removed": remove}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_are_read_locally_and_names_are_not() {
        assert_eq!(parse_coordinates("47.6062, -122.3321"), Some((47.6062, -122.3321)));
        assert_eq!(parse_coordinates("  47.6062 -122.3321 "), Some((47.6062, -122.3321)));
        assert_eq!(parse_coordinates("47.6062; -122.3321"), Some((47.6062, -122.3321)));
        let (la, lo) = parse_coordinates("47°36'22\"N 122°19'56\"W").unwrap();
        assert!((la - 47.60611).abs() < 1e-4 && (lo + 122.33222).abs() < 1e-4, "{la} {lo}");
        let (la, lo) = parse_coordinates("33.8688 S, 151.2093 E").unwrap();
        assert!((la + 33.8688).abs() < 1e-9 && (lo - 151.2093).abs() < 1e-9);
        for name in ["Studio 54", "Route 66", "Pike Place Market", "1600 Pennsylvania Ave NW, Washington", "", "95, 10", "7", "N S E W"] {
            assert_eq!(parse_coordinates(name), None, "{name}");
        }
        // hostile input never panics
        let _ = parse_coordinates(&"9".repeat(10_000));
        let _ = parse_coordinates("°°°'''\"\"\",,,,");
    }

    #[test]
    fn nominatim_results_become_places() {
        let v: Value = serde_json::from_str(
            r#"{"lat":"47.6097","lon":"-122.3422","category":"amenity","type":"marketplace","addresstype":"amenity","name":"Pike Place Market",
                "display_name":"Pike Place Market, 85, Pike Street, Belltown, Seattle, King County, Washington, 98101, United States",
                "address":{"amenity":"Pike Place Market","house_number":"85","road":"Pike Street","suburb":"Belltown","city":"Seattle","county":"King County","state":"Washington","postcode":"98101","country":"United States","country_code":"us"}}"#,
        )
        .unwrap();
        let p = place_from_nominatim(&v).unwrap();
        assert_eq!(
            (p.name.as_str(), p.location.as_str(), p.city.as_str(), p.state.as_str(), p.country.as_str()),
            ("Pike Place Market", "Pike Place Market", "Seattle", "Washington", "United States")
        );
        assert_eq!(p.keywords(), vec!["Pike Place Market", "Seattle", "Washington", "United States", "market"]);
        // a town: no sublocation, no kind
        let v: Value = serde_json::from_str(
            r#"{"lat":"45.52","lon":"-122.67","type":"city","addresstype":"city","name":"Portland","display_name":"Portland, Multnomah County, Oregon, United States","address":{"city":"Portland","state":"Oregon","country":"United States"}}"#,
        )
        .unwrap();
        let p = place_from_nominatim(&v).unwrap();
        assert_eq!((p.location.as_str(), p.city.as_str(), p.kind.as_str()), ("", "Portland", ""));
        assert_eq!(p.summary(), "Portland, Oregon, United States");
        // a street address: the street is the location, but no keyword
        let v: Value = serde_json::from_str(
            r#"{"lat":"38.8977","lon":"-77.0365","type":"house","addresstype":"building","name":"","display_name":"1600, Pennsylvania Avenue Northwest, Washington, District of Columbia, 20500, United States","address":{"house_number":"1600","road":"Pennsylvania Avenue Northwest","city":"Washington","state":"District of Columbia","country":"United States"}}"#,
        )
        .unwrap();
        let p = place_from_nominatim(&v).unwrap();
        assert_eq!(p.location, "1600 Pennsylvania Avenue Northwest");
        assert_eq!(p.keywords(), vec!["Washington", "District of Columbia", "United States"]);
        // nonsense is skipped
        for bad in [json!({}), json!({"lat": "x", "lon": "1"}), json!({"lat": "0", "lon": "0"}), json!({"lat": "95", "lon": "1"}), json!(null)] {
            assert_eq!(place_from_nominatim(&bad), None, "{bad}");
        }
    }

    #[test]
    fn album_names_say_what_the_photos_are() {
        assert_eq!(name_keywords("*C&J WEDDING"), vec!["wedding"]);
        assert_eq!(name_keywords("2024-06 Family Beach Trip"), vec!["family", "beach", "travel"]);
        assert_eq!(name_keywords("Real Estate - 12 Oak St"), vec!["real estate"]);
        assert_eq!(name_keywords("IMG_2024_0612"), Vec::<String>::new());
        assert_eq!(name_keywords(""), Vec::<String>::new());
    }

    #[test]
    fn content_scores_pick_the_clear_subjects() {
        // three orthogonal "phrases" and a background class
        let unit = |i: usize| (0..8).map(|j| if j == i { 1.0 } else { 0.0 }).collect::<Vec<f32>>();
        let labels: Vec<(String, Vec<f32>)> =
            vec![("wedding".into(), unit(0)), ("beach".into(), unit(1)), ("dog".into(), unit(2)), ("".into(), unit(3))];
        let wedding_photo: Vec<f32> = vec![0.9, 0.1, 0.0, 0.2, 0.0, 0.0, 0.0, 0.0];
        let s = content_scores(&wedding_photo, &labels);
        assert_eq!(s.first().map(|x| x.0.as_str()), Some("wedding"), "{s:?}");
        assert!(s.iter().all(|x| x.0 != "dog"));
        // a photo of nothing in the list (closest to the background) suggests nothing
        assert!(content_scores(&unit(3), &labels).is_empty());
        assert!(content_scores(&[], &labels).is_empty());
        assert!(content_scores(&[f32::NAN; 8], &labels).is_empty());
        // an album: wedding on 3 of 4 photos counts, the beach on 1 of 4 doesn't
        let per = vec![
            vec![("wedding".to_string(), 0.9)],
            vec![("wedding".to_string(), 0.8)],
            vec![("wedding".to_string(), 0.7), ("beach".to_string(), 0.3)],
            vec![],
        ];
        assert_eq!(content_keywords(&per), vec![("wedding".to_string(), 0.75)]);
    }

    #[test]
    fn url_encoding() {
        assert_eq!(url_encode("Pike Place Market, Seattle"), "Pike+Place+Market%2C+Seattle");
        assert_eq!(url_encode("C&J"), "C%26J");
        assert_eq!(url_encode("café"), "caf%C3%A9");
    }
}
