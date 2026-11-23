//! LR-MAP-MODULE: the Map module's model — Web Mercator ("slippy map") maths, saved locations, and
//! the OpenStreetMap tile store the UI draws from.
//!
//! **Saved locations** live in the catalog as one library record ([`LOCATIONS_KEY`], via
//! `Op::SetRecord`): a name, a centre and a radius in metres (Lightroom Classic's Saved Locations
//! panel). A photo is "in" a location when its GPS position lies within the radius.
//!
//! **Tiles** come from `tile.openstreetmap.org` at runtime (never part of the repository) and are
//! kept on disk (the library's `Map Tiles/` folder, else the user's cache folder) so a map that was
//! seen once works offline. Following the OpenStreetMap tile usage policy: an identifying
//! `User-Agent` (`LightCraft/<version>`, nothing personal), at most two connections, cached tiles
//! reused for [`TILE_MAX_AGE`] before they are fetched again, nothing prefetched beyond the view,
//! and the attribution ([`ATTRIBUTION`]) always shown on the map. The network never blocks the UI:
//! tiles load on two worker threads; a tile that can't be loaded is drawn as an empty grid square,
//! and [`TileStore::status`] says why.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use lightcraft_catalog::{Catalog, PhotoId};
use lightcraft_raster::Rgba8;
use serde::{Deserialize, Serialize};

/// Where saved locations are kept in the catalog (a JSON array of [`SavedLocation`]).
pub const LOCATIONS_KEY: &str = "map.locations";
/// Shown on every map (required by the OpenStreetMap licence).
pub const ATTRIBUTION: &str = "© OpenStreetMap contributors";
/// Where the attribution links to.
pub const COPYRIGHT_URL: &str = "https://www.openstreetmap.org/copyright";
/// Tile server (standard OSM raster tiles, 256 px PNG).
pub const TILE_SERVER: &str = "https://tile.openstreetmap.org";
/// Pixel size of a tile.
pub const TILE: f64 = 256.0;
/// Deepest zoom the tile server has.
pub const MAX_ZOOM: f64 = 19.0;
/// Web Mercator can't show the poles: latitudes are clamped to this.
pub const MAX_LAT: f64 = 85.051_128_78;
/// A cached tile older than this is fetched again (when online).
pub const TILE_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 3600);
/// Decoded tiles kept in memory (256 KB each).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const MEMORY_TILES: usize = 384;
/// Requests waiting for a worker; the oldest are dropped first (the view moved on).
const QUEUE: usize = 96;
/// A tile that failed is tried again after this long.
const RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(60);
/// Largest tile body accepted.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const MAX_TILE_BYTES: usize = 2 << 20;
/// Mean Earth radius (metres), for distances.
const EARTH_RADIUS_M: f64 = 6_371_008.8;

// ------------------------------------------------------------------------------- saved places

/// A Saved Location (the Map module's left panel).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedLocation {
    pub id: u64,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// Metres around the centre that count as "in" the location.
    pub radius: f64,
    /// Lightroom's "Private" option: photos here are exported without their GPS position.
    #[serde(default)]
    pub private: bool,
}

/// The saved locations of a library, in name order (entries that don't parse are skipped).
pub fn locations(c: &Catalog) -> Vec<SavedLocation> {
    let mut v: Vec<SavedLocation> = c
        .record(LOCATIONS_KEY)
        .and_then(|r| r.as_array())
        .map(|a| a.iter().filter_map(|x| serde_json::from_value::<SavedLocation>(x.clone()).ok()).filter(|l| valid(l.lat, l.lon)).collect())
        .unwrap_or_default();
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    v
}

/// A usable latitude / longitude pair.
pub fn valid(lat: f64, lon: f64) -> bool {
    lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0
}

/// A photo's position as the map places it: a usable latitude / longitude that isn't exactly
/// 0, 0 ("null island", in the Gulf of Guinea: what cameras and apps write when they have no fix).
/// `None`: the photo has no location and is left off the map.
pub fn located(gps: Option<(f64, f64)>) -> Option<(f64, f64)> {
    gps.filter(|g| valid(g.0, g.1) && !(g.0 == 0.0 && g.1 == 0.0))
}

/// Great-circle distance in metres.
pub fn distance_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (la1, la2) = (a.0.to_radians(), b.0.to_radians());
    let dla = la2 - la1;
    let dlo = (b.1 - a.1).to_radians();
    let h = (dla / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlo / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().clamp(0.0, 1.0).asin()
}

/// Library photos (not deleted, not only browsed) whose GPS position is within `radius` metres of
/// (`lat`, `lon`), in id order.
pub fn photos_within(c: &Catalog, lat: f64, lon: f64, radius: f64) -> Vec<PhotoId> {
    let r = if radius.is_finite() { radius.max(0.0) } else { 0.0 };
    c.photos()
        .filter(|p| !p.deleted && !p.local)
        .filter(|p| located(p.meta.gps).is_some_and(|g| distance_m(g, (lat, lon)) <= r))
        .map(|p| p.id)
        .collect()
}

/// The position lies inside a saved location marked Private (exports leave its GPS out).
pub fn in_private_location(c: &Catalog, gps: Option<(f64, f64)>) -> bool {
    let Some(g) = located(gps) else { return false };
    c.record(LOCATIONS_KEY).is_some() && locations(c).iter().any(|l| l.private && distance_m(g, (l.lat, l.lon)) <= l.radius)
}

// ------------------------------------------------------------------------ location filter

/// LRC-MAP-FILTER: the Map module's location filter bar — which of the view's photos the
/// filmstrip shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum LocationFilter {
    /// No location filter.
    #[default]
    None,
    /// Visible on Map: photos placed inside the map's view (degrees; `west > east` crosses the
    /// antimeridian).
    Visible { south: f64, west: f64, north: f64, east: f64 },
    /// Photos with a GPS position.
    Tagged,
    /// Photos without one.
    Untagged,
    /// Photos inside any saved location (the Saved Locations panel as a filter).
    Saved,
}

impl LocationFilter {
    /// The id the filter bar's buttons and `map.locationFilter` use.
    pub fn mode(&self) -> &'static str {
        match self {
            LocationFilter::None => "none",
            LocationFilter::Visible { .. } => "visible",
            LocationFilter::Tagged => "tagged",
            LocationFilter::Untagged => "untagged",
            LocationFilter::Saved => "saved",
        }
    }

    /// A photo at `gps` passes (`saved`: the library's saved locations, for [`LocationFilter::Saved`]).
    pub fn matches(&self, gps: Option<(f64, f64)>, saved: &[SavedLocation]) -> bool {
        let gps = located(gps);
        match *self {
            LocationFilter::None => true,
            LocationFilter::Tagged => gps.is_some(),
            LocationFilter::Untagged => gps.is_none(),
            LocationFilter::Saved => gps.is_some_and(|g| saved.iter().any(|l| distance_m(g, (l.lat, l.lon)) <= l.radius)),
            LocationFilter::Visible { south, west, north, east } => gps.is_some_and(|(lat, lon)| {
                let in_lon = if west <= east { lon >= west && lon <= east } else { lon >= west || lon <= east };
                lat >= south && lat <= north && in_lon
            }),
        }
    }

    /// The map view's bounds as a filter (degrees): longitudes wrapped into ±180, a view as wide
    /// as the world keeps every longitude. `None` for bounds that aren't numbers or are upside down.
    pub fn visible(south: f64, west: f64, north: f64, east: f64) -> Option<LocationFilter> {
        if ![south, west, north, east].iter().all(|v| v.is_finite()) || south > north || west > east {
            return None;
        }
        let wrap = |l: f64| (l + 180.0).rem_euclid(360.0) - 180.0;
        let (w, e) = if east - west >= 360.0 {
            (-180.0, 180.0)
        } else {
            let (w, e) = (wrap(west), wrap(east));
            // an edge exactly on the antimeridian keeps its own side
            (if w == -180.0 && west > 0.0 { 180.0 } else { w }, if e == -180.0 && east > 0.0 { 180.0 } else { e })
        };
        Some(LocationFilter::Visible { south: south.clamp(-90.0, 90.0), west: w, north: north.clamp(-90.0, 90.0), east: e })
    }
}

// --------------------------------------------------------------------------- web mercator

/// World pixel position of a latitude / longitude at `zoom` (the world is `256 · 2^zoom` px wide).
pub fn world_px(lat: f64, lon: f64, zoom: f64) -> (f64, f64) {
    let size = TILE * 2f64.powf(zoom.clamp(0.0, MAX_ZOOM));
    let lat = if lat.is_finite() { lat.clamp(-MAX_LAT, MAX_LAT) } else { 0.0 };
    let lon = if lon.is_finite() { lon.clamp(-180.0, 180.0) } else { 0.0 };
    let x = (lon + 180.0) / 360.0 * size;
    let s = lat.to_radians().sin();
    let y = (0.5 - ((1.0 + s) / (1.0 - s)).ln() / (4.0 * std::f64::consts::PI)) * size;
    (x, y)
}

/// The latitude / longitude at a world pixel position (x wraps around the world).
pub fn lat_lon(x: f64, y: f64, zoom: f64) -> (f64, f64) {
    let size = TILE * 2f64.powf(zoom.clamp(0.0, MAX_ZOOM));
    let x = if x.is_finite() { x.rem_euclid(size) } else { 0.0 };
    let y = if y.is_finite() { y.clamp(0.0, size) } else { size / 2.0 };
    let lon = x / size * 360.0 - 180.0;
    let n = std::f64::consts::PI * (1.0 - 2.0 * y / size);
    let lat = n.sinh().atan().to_degrees();
    (lat.clamp(-MAX_LAT, MAX_LAT), lon)
}

/// Metres per pixel at `lat` and `zoom`.
pub fn metres_per_px(lat: f64, zoom: f64) -> f64 {
    let c = 2.0 * std::f64::consts::PI * 6_378_137.0;
    c * lat.clamp(-MAX_LAT, MAX_LAT).to_radians().cos() / (TILE * 2f64.powf(zoom.clamp(0.0, MAX_ZOOM)))
}

/// The view (centre, zoom) that shows every point within `w × h` px with a margin; one point (or
/// none) gets a street-level zoom.
pub fn fit(points: &[(f64, f64)], w: f64, h: f64) -> Option<((f64, f64), f64)> {
    let pts: Vec<(f64, f64)> = points.iter().copied().filter(|p| valid(p.0, p.1)).collect();
    if pts.is_empty() {
        return None;
    }
    let (w, h) = ((w - 160.0).max(64.0), (h - 160.0).max(64.0));
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in &pts {
        let (x, y) = world_px(p.0, p.1, 0.0);
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    let span = ((x1 - x0) / w.max(1.0)).max((y1 - y0) / h.max(1.0));
    let zoom = if span <= 0.0 { 15.0 } else { (1.0 / span).log2().clamp(1.0, 16.0) };
    Some((lat_lon((x0 + x1) / 2.0, (y0 + y1) / 2.0, 0.0), zoom))
}

/// One tile of the view: zoom level and its column / row (`x` already wrapped into the world).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TileId {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

impl TileId {
    /// The tile at column `x` (any integer: wraps around) and row `y` (`None` above / below the
    /// world) of zoom `z`.
    pub fn wrapped(z: u8, x: i64, y: i64) -> Option<TileId> {
        let z = z.min(MAX_ZOOM as u8);
        let n = 1i64 << z;
        if !(0..n).contains(&y) {
            return None;
        }
        Some(TileId { z, x: u32::try_from(x.rem_euclid(n)).ok()?, y: u32::try_from(y).ok()? })
    }
    pub fn url(&self) -> String {
        format!("{TILE_SERVER}/{}/{}/{}.png", self.z, self.x, self.y)
    }
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn file(&self) -> PathBuf {
        PathBuf::from(self.z.to_string()).join(self.x.to_string()).join(format!("{}.png", self.y))
    }
}

// ---------------------------------------------------------------------------------- tiles

/// A tile as the map can draw it now.
#[derive(Clone)]
pub enum Tile {
    Ready(Arc<Rgba8>),
    /// Being read or fetched.
    Loading,
    /// Not available (offline, the server refused, a damaged file): drawn as an empty square.
    Missing,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
enum Entry {
    Ready(Arc<Rgba8>, u64),
    Queued,
    Failed(web_time::Instant),
}

#[derive(Default)]
struct Inner {
    tiles: HashMap<TileId, Entry>,
    queue: VecDeque<TileId>,
    tick: u64,
    last_error: Option<String>,
    fetched: u64,
    stop: bool,
}

/// Map tiles: memory LRU, disk cache and (native, when online) the tile server.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub struct TileStore {
    dir: Option<PathBuf>,
    online: bool,
    shared: Arc<(Mutex<Inner>, Condvar)>,
}

impl TileStore {
    /// Tiles cached in `dir` (`None`: memory only), fetched from the server when `online`.
    /// `LIGHTCRAFT_MAP_OFFLINE` set: never online. The web build never fetches.
    pub fn new(dir: Option<PathBuf>, online: bool) -> Arc<TileStore> {
        let online = online && cfg!(not(target_arch = "wasm32")) && std::env::var_os("LIGHTCRAFT_MAP_OFFLINE").is_none();
        let store = Arc::new(TileStore { dir, online, shared: Arc::new((Mutex::new(Inner::default()), Condvar::new())) });
        #[cfg(not(target_arch = "wasm32"))]
        for i in 0..2 {
            let shared = store.shared.clone();
            let (dir, online) = (store.dir.clone(), store.online);
            let spawned = std::thread::Builder::new().name(format!("map-tiles-{i}")).spawn(move || worker(&shared, dir, online));
            if let Err(e) = spawned {
                log::warn!("map tiles: can't start a worker: {e}");
            }
        }
        store
    }

    /// The library's tile cache folder, else the user's cache folder.
    pub fn default_dir(library: Option<&std::path::Path>) -> Option<PathBuf> {
        if let Some(l) = library {
            return Some(l.join("Map Tiles"));
        }
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches/LightCraft/Map Tiles"))
    }

    pub fn online(&self) -> bool {
        self.online
    }

    /// The tile now; a tile not in memory is queued for loading (the newest requests first).
    pub fn get(&self, id: TileId) -> Tile {
        let (lock, cv) = &*self.shared;
        let mut g = lock.lock().unwrap_or_else(PoisonError::into_inner);
        g.tick += 1;
        let tick = g.tick;
        match g.tiles.get_mut(&id) {
            Some(Entry::Ready(img, used)) => {
                *used = tick;
                return Tile::Ready(img.clone());
            }
            Some(Entry::Queued) => {
                // asked again: move it to the front of the line
                if let Some(i) = g.queue.iter().position(|q| *q == id) {
                    g.queue.remove(i);
                    g.queue.push_back(id);
                }
                return Tile::Loading;
            }
            Some(Entry::Failed(at)) if at.elapsed() < RETRY_AFTER => return Tile::Missing,
            _ => {}
        }
        if cfg!(target_arch = "wasm32") {
            g.tiles.insert(id, Entry::Failed(web_time::Instant::now()));
            return Tile::Missing;
        }
        g.tiles.insert(id, Entry::Queued);
        g.queue.push_back(id);
        while g.queue.len() > QUEUE {
            if let Some(old) = g.queue.pop_front() {
                g.tiles.remove(&old);
            }
        }
        cv.notify_one();
        Tile::Loading
    }

    /// Tiles still waiting or loading.
    pub fn pending(&self) -> usize {
        let g = self.shared.0.lock().unwrap_or_else(PoisonError::into_inner);
        g.tiles.values().filter(|e| matches!(e, Entry::Queued)).count()
    }

    /// `{online, cached, fetched, pending, error?}`: what the map's notice says.
    pub fn status(&self) -> serde_json::Value {
        let g = self.shared.0.lock().unwrap_or_else(PoisonError::into_inner);
        let ready = g.tiles.values().filter(|e| matches!(e, Entry::Ready(..))).count();
        let pending = g.tiles.values().filter(|e| matches!(e, Entry::Queued)).count();
        serde_json::json!({"online": self.online, "inMemory": ready, "fetched": g.fetched, "pending": pending, "error": g.last_error})
    }
}

impl Drop for TileStore {
    fn drop(&mut self) {
        let (lock, cv) = &*self.shared;
        lock.lock().unwrap_or_else(PoisonError::into_inner).stop = true;
        cv.notify_all();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn worker(shared: &(Mutex<Inner>, Condvar), dir: Option<PathBuf>, online: bool) {
    let (lock, cv) = shared;
    loop {
        let id = {
            let mut g = lock.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if g.stop {
                    return;
                }
                // newest first: what the view shows now
                if let Some(id) = g.queue.pop_back() {
                    break id;
                }
                g = cv.wait(g).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let r = load(&dir, online, id);
        let mut g = lock.lock().unwrap_or_else(PoisonError::into_inner);
        g.tick += 1;
        let tick = g.tick;
        match r {
            Ok((img, fetched)) => {
                if fetched {
                    g.fetched += 1;
                    g.last_error = None;
                }
                g.tiles.insert(id, Entry::Ready(Arc::new(img), tick));
            }
            Err(e) => {
                g.last_error = Some(e);
                g.tiles.insert(id, Entry::Failed(web_time::Instant::now()));
            }
        }
        // memory budget: the least recently drawn tiles go
        let ready = g.tiles.iter().filter_map(|(k, e)| if let Entry::Ready(_, u) = e { Some((*u, *k)) } else { None }).collect::<Vec<_>>();
        if ready.len() > MEMORY_TILES {
            let mut ready = ready;
            ready.sort_by_key(|r| r.0);
            for (_, k) in ready.iter().take(ready.len() - MEMORY_TILES) {
                g.tiles.remove(k);
            }
        }
    }
}

/// A tile from the disk cache (fresh, or any age when offline), else from the server (and cached).
/// `true`: it came over the network.
#[cfg(not(target_arch = "wasm32"))]
fn load(dir: &Option<PathBuf>, online: bool, id: TileId) -> Result<(Rgba8, bool), String> {
    let path = dir.as_ref().map(|d| d.join(id.file()));
    let cached = path.as_ref().and_then(|p| {
        let m = std::fs::metadata(p).ok()?;
        let age = m.modified().ok().and_then(|t| t.elapsed().ok()).unwrap_or_default();
        Some((p.clone(), age))
    });
    if let Some((p, age)) = &cached
        && (!online || *age < TILE_MAX_AGE)
        && let Ok(img) = std::fs::read(p).map_err(|e| e.to_string()).and_then(|b| decode(&b))
    {
        return Ok((img, false));
    }
    if !online {
        return Err("map tiles are offline: only places seen before are drawn".into());
    }
    match fetch(&id) {
        Ok(bytes) => {
            let img = decode(&bytes)?;
            if let Some(p) = &path {
                // written whole, then renamed: a crash never leaves half a tile under its name
                let tmp = p.with_extension("part");
                let ok = p.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok())
                    && std::fs::write(&tmp, &bytes).is_ok()
                    && std::fs::rename(&tmp, p).is_ok();
                if !ok {
                    let _ = std::fs::remove_file(&tmp);
                }
            }
            Ok((img, true))
        }
        // the network failed: an old cached copy is better than nothing
        Err(e) => match cached.and_then(|(p, _)| std::fs::read(p).ok()).and_then(|b| decode(&b).ok()) {
            Some(img) => Ok((img, false)),
            None => Err(e),
        },
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn fetch(id: &TileId) -> Result<Vec<u8>, String> {
    use lightcraft_fetch::http;
    static CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let limits = http::Limits { connect: std::time::Duration::from_secs(8), stall: std::time::Duration::from_secs(15), cancel: &CANCEL };
    let mut url = http::Url::parse(&id.url()).map_err(|e| e.to_string())?;
    for _ in 0..3 {
        let mut r = http::get(&url, &[("Accept", "image/png".to_string())], &limits).map_err(|e| format!("map tiles: {e}"))?;
        if (300..400).contains(&r.status) {
            let loc = r.header("location").ok_or("map tiles: a redirect without a location")?.to_string();
            url = url.join(&loc).map_err(|e| e.to_string())?;
            continue;
        }
        if r.status != 200 {
            return Err(format!("map tiles: the tile server answered {}", r.status));
        }
        let mut body = Vec::with_capacity(r.content_length().map_or(32 << 10, |n| (n as usize).min(MAX_TILE_BYTES)));
        let mut buf = [0u8; 16 << 10];
        loop {
            let n = r.read(&mut buf, &limits).map_err(|e| format!("map tiles: {e}"))?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(buf.get(..n).unwrap_or_default());
            if body.len() > MAX_TILE_BYTES {
                return Err("map tiles: a tile larger than 2 MB".into());
            }
        }
        return Ok(body);
    }
    Err("map tiles: too many redirects".into())
}

/// A tile's PNG (or JPEG) bytes as display sRGB.
pub fn decode(bytes: &[u8]) -> Result<Rgba8, String> {
    let d = lightcraft_codecs::decode(bytes, lightcraft_codecs::DecodeOptions::fit(512, 512)).map_err(|e| format!("map tile: {e}"))?;
    Ok(d.to_srgb8())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mercator_round_trips() {
        for (lat, lon) in [(0.0, 0.0), (51.5072, -0.1276), (-33.8688, 151.2093), (85.0, 179.9), (-85.0, -179.9)] {
            for z in [0.0, 3.5, 12.0, 19.0] {
                let (x, y) = world_px(lat, lon, z);
                let (la, lo) = lat_lon(x, y, z);
                assert!((la - lat).abs() < 1e-6 && (lo - lon).abs() < 1e-6, "{lat},{lon}@{z}: {la},{lo}");
            }
        }
        // the world is 256 px at zoom 0, centred on (0, 0)
        assert_eq!(world_px(0.0, 0.0, 0.0), (128.0, 128.0));
        // nonsense never panics
        let _ = world_px(f64::NAN, f64::INFINITY, f64::NAN);
        let _ = lat_lon(f64::NAN, f64::NEG_INFINITY, -4.0);
    }

    #[test]
    fn tiles_wrap_and_stay_in_the_world() {
        assert_eq!(TileId::wrapped(2, -1, 0), Some(TileId { z: 2, x: 3, y: 0 }));
        assert_eq!(TileId::wrapped(2, 4, 3), Some(TileId { z: 2, x: 0, y: 3 }));
        assert_eq!(TileId::wrapped(2, 0, 4), None);
        assert_eq!(TileId::wrapped(2, 0, -1), None);
        assert_eq!(TileId::wrapped(40, 0, 0).map(|t| t.z), Some(19));
        assert_eq!(TileId { z: 3, x: 4, y: 2 }.url(), "https://tile.openstreetmap.org/3/4/2.png");
    }

    #[test]
    fn distances_and_fit() {
        // London → Paris ≈ 344 km
        let d = distance_m((51.5072, -0.1276), (48.8566, 2.3522));
        assert!((d - 343_500.0).abs() < 2_000.0, "{d}");
        assert_eq!(fit(&[], 800.0, 600.0), None);
        let ((la, lo), z) = fit(&[(51.5, -0.12), (48.85, 2.35)], 800.0, 600.0).unwrap();
        assert!(la > 48.0 && la < 52.0 && lo > -0.2 && lo < 2.4 && (4.0..9.0).contains(&z), "{la},{lo}@{z}");
        let (_, z1) = fit(&[(10.0, 10.0)], 800.0, 600.0).unwrap();
        assert_eq!(z1, 15.0);
    }

    /// Live: one real tile from the OpenStreetMap server, decoded and cached (needs the network:
    /// `cargo test -p lightcraft-engine fetches_a_real_tile -- --ignored`).
    #[test]
    #[ignore]
    fn fetches_a_real_tile() {
        let dir = std::env::temp_dir().join(format!("lc-map-live-{}", std::process::id()));
        let store = TileStore::new(Some(dir.clone()), true);
        let id = TileId { z: 2, x: 1, y: 1 };
        let t0 = web_time::Instant::now();
        let mut ok = false;
        while t0.elapsed() < std::time::Duration::from_secs(30) && !ok {
            ok = matches!(store.get(id), Tile::Ready(ref i) if i.width == 256 && i.height == 256);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(ok, "{}", store.status());
        assert!(dir.join("2/1/1.png").is_file());
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Offline with an empty cache: tiles end up missing (never a hang or a panic), and the
    /// status says why; a cached tile is read from disk.
    #[test]
    fn offline_tiles_come_from_the_cache_or_are_missing() {
        let dir = std::env::temp_dir().join(format!("lc-map-tiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let id = TileId { z: 1, x: 0, y: 1 };
        let mut img = Rgba8::new(4, 4);
        img.data.iter_mut().for_each(|p| *p = [10, 200, 30, 255]);
        let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        std::fs::create_dir_all(dir.join("1/0")).unwrap();
        std::fs::write(dir.join("1/0/1.png"), &png).unwrap();
        let store = TileStore::new(Some(dir.clone()), false);
        let other = TileId { z: 1, x: 1, y: 1 };
        let t0 = web_time::Instant::now();
        let (mut got, mut missing) = (false, false);
        while t0.elapsed() < std::time::Duration::from_secs(10) && !(got && missing) {
            got |= matches!(store.get(id), Tile::Ready(ref i) if i.data[0] == [10, 200, 30, 255]);
            missing |= matches!(store.get(other), Tile::Missing);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(got && missing, "cached tile read: {got}, uncached one missing: {missing}");
        assert_eq!(store.status()["online"], false);
        assert!(store.status()["error"].as_str().is_some_and(|e| e.contains("offline")), "{}", store.status());
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
