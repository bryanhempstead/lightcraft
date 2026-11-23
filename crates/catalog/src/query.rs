//! Filtering, search and sorting.

use serde::{Deserialize, Serialize};

use crate::{AlbumId, Catalog, ColorLabel, Flag, MediaKind, Photo, PhotoId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RatingOp {
    #[default]
    AtLeast,
    Exactly,
    AtMost,
}

/// What the grid shows. Empty/None fields don't filter.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Filter {
    /// Free-text search: every token must match filename, title, caption, keywords, camera, lens,
    /// location or format. Tokens like `rating:3`, `flag:pick`, `iso:>800`, `camera:x2` are fielded.
    pub text: String,
    /// Lightroom's Text filter target: `any` (default: every searchable field), `fileName`,
    /// `copyName`, `title`, `caption`, `keywords`, `metadata` (IPTC text), `exif` (camera, lens,
    /// format).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_target: Option<String>,
    /// Its rule: `containsAll` (default: every word appears), `contains` (any word), `words`
    /// (whole words), `notContains`, `startsWith`, `endsWith`. A word starting with `!` must not
    /// appear (any rule).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_rule: Option<String>,
    pub rating: u8,
    pub rating_op: RatingOp,
    pub flag: Option<Flag>,
    /// Only fav. photos (the filter bar's heart).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub favorite: bool,
    pub label: Option<ColorLabel>,
    /// Only these photos (Find Similar results…); empty = no constraint.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub only: Vec<PhotoId>,
    /// Any of these labels (the filter bar's multi-select); empty = no constraint.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<ColorLabel>,
    pub kind: Option<MediaKind>,
    pub edited: Option<bool>,
    /// Photos without a colour label (with `labels`: those labels or none — Lightroom's No Label).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub no_label: bool,
    /// `true`: virtual copies only, `false`: originals only (Lightroom's Kind filter).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub virtual_copy: Option<bool>,
    /// `true`: photos in a stack, `false`: photos in none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stacked: Option<bool>,
    /// Exported (`true`) or never exported (`false`). The export record lives with the library,
    /// not the catalog: the engine applies this one ([`Filter::matches`] ignores it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exported: Option<bool>,
    /// The metadata column browser's picks ([`crate::columns`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<crate::columns::ColumnPick>,
    pub album: Option<AlbumId>,
    /// Show "Recently Deleted" instead of the library.
    pub deleted: bool,
    /// Capture date prefix (`2026`, `2026-04`, `2026-04-12`).
    pub date: Option<String>,
    /// A folder the library's photos were imported from: those photos and the ones imported from
    /// the folders inside it (see [`crate::folders`]). Photos only browsed in Local are never part
    /// of it, unlike [`Filter::folder`].
    pub library_folder: Option<String>,
    /// With [`Filter::library_folder`]: only the photos directly in that folder, not those of the
    /// folders inside it (Show Photos in Subfolders turned off).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub library_folder_only: bool,
    /// A keyword; hierarchical keywords match their children too (`travel` finds `travel|italy`).
    pub keyword: Option<String>,
    /// A person: photos with a named face region of this name (case-insensitive), as read from XMP.
    pub person: Option<String>,
    pub camera: Option<String>,
    /// Lens (case-insensitive substring).
    pub lens: Option<String>,
    /// Capture date range, inclusive: `dateFrom` is compared as a lower bound (`2026-04-01`),
    /// `dateTo` as a prefix upper bound (`2026-04` includes all of April).
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    /// Import date prefix.
    pub imported: Option<String>,
    /// Imported at or after this time (ISO; Recently Added).
    pub imported_from: Option<String>,
    /// Photo Merge results: `hdr`, `panorama`, `hdrPanorama` or `any` (see [`merged_kind`]).
    pub merged: Option<String>,
    /// A folder on disk: its files only (browsed ones too); `subfolders` includes everything below.
    pub folder: Option<String>,
    pub subfolders: bool,
    /// Smart-album rules (all / any / none, nested groups; see [`crate::rules`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_set: Option<crate::RuleSet>,
    /// Natural-language search ("dog on the beach"): the photos whose picture matches best, best first.
    /// The embeddings are the engine's: it applies this one ([`Filter::matches`] ignores it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortKey {
    #[default]
    CaptureDate,
    ImportDate,
    EditDate,
    FileName,
    Rating,
    FileSize,
    /// A shuffle fixed by [`Sort::seed`]: the same seed always gives the same order, and photos
    /// added or edited later never reshuffle the others. Pick a new seed to reshuffle.
    Random,
    /// Lightroom's other sort orders (L17): colour label (red … purple, none first), label name,
    /// edit count (history steps), aspect ratio, file type, extension, pixel count, pick flag
    /// (rejected, unflagged, flagged).
    LabelColor,
    LabelText,
    EditCount,
    AspectRatio,
    FileType,
    Extension,
    PixelCount,
    Pick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sort {
    pub key: SortKey,
    pub ascending: bool,
    /// Date headers in the grid (date sort keys only).
    pub group: crate::GroupBy,
    /// Which shuffle [`SortKey::Random`] gives (ignored by the other keys).
    pub seed: u64,
}

impl Default for Sort {
    fn default() -> Self {
        Sort { key: SortKey::CaptureDate, ascending: false, group: crate::GroupBy::Auto, seed: 0 }
    }
}

/// splitmix64 finaliser: a stateless, platform-independent mix (no RNG state, no `rand` version
/// to drift), so a seed names the same shuffle on every machine.
pub fn mix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A photo's place in the shuffle for `seed`.
fn shuffle_rank(seed: u64, id: PhotoId) -> u64 {
    mix64(mix64(seed) ^ id.0)
}

fn token_matches(p: &Photo, tok: &str) -> bool {
    let t = tok.to_lowercase();
    if let Some((field, val)) = t.split_once(':') {
        let num = |s: &str| -> Option<(char, f64)> {
            let (op, rest) = match s.chars().next()? {
                c @ ('>' | '<' | '=') => (c, &s[1..]),
                _ => ('=', s),
            };
            rest.parse().ok().map(|v| (op, v))
        };
        let cmp = |x: f64, s: &str| match num(s) {
            Some(('>', v)) => x > v,
            Some(('<', v)) => x < v,
            Some((_, v)) => (x - v).abs() < 1e-9,
            None => false,
        };
        return match field {
            "rating" | "stars" => cmp(p.rating as f64, val),
            "flag" => Flag::parse(val) == Some(p.flag),
            "fav" | "favorite" | "heart" => matches!(val, "yes" | "y" | "true" | "1" | "on") == p.favorite,
            "label" | "color" => p.label.is_some_and(|l| format!("{l:?}").eq_ignore_ascii_case(val)),
            "iso" => p.meta.iso.is_some_and(|i| cmp(i as f64, val)),
            "f" | "aperture" => p.meta.aperture.is_some_and(|a| cmp(a as f64, val)),
            "focal" => p.meta.focal_mm.is_some_and(|a| cmp(a as f64, val)),
            "camera" => p.meta.camera.to_lowercase().contains(val),
            "lens" => p.meta.lens.to_lowercase().contains(val),
            "keyword" | "kw" => p.meta.keywords.iter().any(|k| crate::keywords::is_under(k, val)),
            "person" | "who" => has_person(p, val),
            "type" | "kind" => format!("{:?}", p.kind).eq_ignore_ascii_case(val),
            "edited" => (val == "true" || val == "yes") == p.is_edited(),
            "date" => p.date().starts_with(val),
            "copy" | "virtual" => (val == "true" || val == "yes") == p.copy_of.is_some(),
            "name" | "file" => p.file_name.to_lowercase().contains(val),
            _ => false,
        };
    }
    let hay = [&p.file_name, &p.meta.title, &p.meta.caption, &p.meta.camera, &p.meta.lens, &p.meta.location, &p.format];
    hay.iter().any(|h| h.to_lowercase().contains(&t)) || p.meta.keywords.iter().any(|k| k.to_lowercase().contains(&t))
}

/// The text a Text filter target searches, lower-cased.
fn target_text(p: &Photo, target: &str) -> Vec<String> {
    let m = &p.meta;
    let v: Vec<&str> = match target {
        "fileName" => vec![&p.file_name],
        "copyName" => vec![p.copy_name.as_deref().unwrap_or("")],
        "title" => vec![&m.title],
        "caption" => vec![&m.caption],
        "keywords" => m.keywords.iter().map(String::as_str).collect(),
        "metadata" => {
            vec![&m.title, &m.caption, &m.creator, &m.copyright, &m.location, &m.city, &m.state, &m.country, &m.alt_text, &m.extended_description]
        }
        "exif" => vec![&m.camera, &m.lens, &p.format, &m.shutter],
        _ => {
            let mut v: Vec<&str> =
                vec![&p.file_name, &m.title, &m.caption, &m.camera, &m.lens, &m.location, &m.city, &m.state, &m.country, &m.creator, &p.format];
            v.extend(m.keywords.iter().map(String::as_str));
            v.extend(p.copy_name.as_deref());
            v
        }
    };
    v.into_iter().map(str::to_lowercase).collect()
}

/// Lightroom's Text filter with a target and a rule (L29). Words starting with `!` exclude.
fn text_rule_matches(p: &Photo, text: &str, target: &str, rule: &str) -> bool {
    let hay = target_text(p, target);
    let (not, want): (Vec<String>, Vec<String>) = text.split_whitespace().map(str::to_lowercase).partition(|w| w.starts_with('!') && w.len() > 1);
    let not: Vec<String> = not.into_iter().map(|w| w[1..].to_string()).collect();
    if not.iter().any(|w| hay.iter().any(|h| h.contains(w.as_str()))) {
        return false;
    }
    if want.is_empty() {
        return true;
    }
    let is_word = |h: &str, w: &str| h.split(|c: char| !c.is_alphanumeric()).any(|x| x == w);
    match rule {
        "contains" => want.iter().any(|w| hay.iter().any(|h| h.contains(w.as_str()))),
        "words" => want.iter().all(|w| hay.iter().any(|h| is_word(h, w))),
        "notContains" => !want.iter().any(|w| hay.iter().any(|h| h.contains(w.as_str()))),
        "startsWith" => want.iter().all(|w| hay.iter().any(|h| h.split(|c: char| !c.is_alphanumeric()).any(|x| x.starts_with(w.as_str())))),
        "endsWith" => want.iter().all(|w| hay.iter().any(|h| h.split(|c: char| !c.is_alphanumeric()).any(|x| x.ends_with(w.as_str())))),
        _ => want.iter().all(|w| hay.iter().any(|h| h.contains(w.as_str()))),
    }
}

/// Whether `p` has a named face region called `name` (case-insensitive).
fn has_person(p: &Photo, name: &str) -> bool {
    let name = name.trim().to_lowercase();
    p.meta.regions.iter().any(|r| r.kind == lightcraft_meta::RegionKind::Face && r.name.as_deref().is_some_and(|n| n.to_lowercase() == name))
}

impl Filter {
    /// Human-readable summary of the active rules (smart album tooltips, `album.list`).
    pub fn describe(&self) -> String {
        let mut v: Vec<String> = Vec::new();
        if self.rating > 0 {
            let op = match self.rating_op {
                RatingOp::AtLeast => "≥",
                RatingOp::Exactly => "=",
                RatingOp::AtMost => "≤",
            };
            v.push(format!("rating {op} {}", self.rating));
        }
        if let Some(f) = self.flag {
            v.push(format!("flag {}", format!("{f:?}").to_lowercase()));
        }
        if let Some(l) = self.label {
            v.push(format!("label {}", format!("{l:?}").to_lowercase()));
        }
        if !self.labels.is_empty() || self.no_label {
            let mut names: Vec<String> = self.labels.iter().map(|l| format!("{l:?}").to_lowercase()).collect();
            if self.no_label {
                names.push("none".into());
            }
            v.push(format!("label {}", names.join(" or ")));
        }
        if let Some(c) = self.virtual_copy {
            v.push(if c { "virtual copies".into() } else { "originals".into() });
        }
        if let Some(st) = self.stacked {
            v.push(if st { "stacked".into() } else { "not stacked".into() });
        }
        if let Some(e) = self.exported {
            v.push(if e { "exported".into() } else { "not exported".into() });
        }
        for c in self.columns.iter().filter(|c| !c.values.is_empty()) {
            v.push(format!(
                "{} {}",
                c.column,
                c.values.iter().map(|x| if x.is_empty() { "unknown" } else { x.as_str() }).collect::<Vec<_>>().join(" or ")
            ));
        }
        if let Some(k) = self.kind {
            v.push(format!("kind {}", format!("{k:?}").to_lowercase()));
        }
        if let Some(e) = self.edited {
            v.push(if e { "edited".into() } else { "unedited".into() });
        }
        for (name, val) in [
            ("keyword", &self.keyword),
            ("person", &self.person),
            ("camera", &self.camera),
            ("lens", &self.lens),
            ("date", &self.date),
            ("folder", &self.library_folder),
            ("imported", &self.imported),
        ] {
            if let Some(x) = val {
                v.push(format!("{name} {x}"));
            }
        }
        match (&self.date_from, &self.date_to) {
            (Some(a), Some(b)) => v.push(format!("captured {a} – {b}")),
            (Some(a), None) => v.push(format!("captured from {a}")),
            (None, Some(b)) => v.push(format!("captured until {b}")),
            _ => {}
        }
        if !self.text.trim().is_empty() {
            v.push(format!("“{}”", self.text.trim()));
        }
        if let Some(rs) = self.rule_set.as_ref().filter(|r| !r.rules.is_empty()) {
            v.push(rs.describe());
        }
        if v.is_empty() { "all photos".into() } else { v.join(", ") }
    }

    /// Whether matches depend on the clock (only "in the last…" rules do).
    pub fn depends_on_now(&self) -> bool {
        self.rule_set.as_ref().is_some_and(crate::RuleSet::depends_on_now)
    }

    pub fn matches(&self, p: &Photo, cat: &Catalog) -> bool {
        self.matches_in(p, cat, self.library_root().as_deref())
    }

    /// [`Filter::library_folder`] as a folder identity ([`folder_key`]); `None` without a choice.
    fn library_root(&self) -> Option<String> {
        // as given: a folder may be named with spaces at its edge, and trimming would make it
        // another folder
        self.library_folder.as_deref().filter(|d| !d.trim().is_empty()).map(folder_key)
    }

    /// [`Filter::matches`] with the library folder already turned into its identity, so a query
    /// over a big library does that once, not once per photo.
    fn matches_in(&self, p: &Photo, cat: &Catalog, root: Option<&str>) -> bool {
        if p.deleted != self.deleted {
            return false;
        }
        match &self.folder {
            // browsed photos only show in folder views
            None if p.local => return false,
            None => {}
            Some(dir) => {
                let crate::Source::File { path } = &p.source else { return false };
                if !in_folder(path, dir, self.subfolders) {
                    return false;
                }
            }
        }
        if self.rating > 0 {
            let ok = match self.rating_op {
                RatingOp::AtLeast => p.rating >= self.rating,
                RatingOp::Exactly => p.rating == self.rating,
                RatingOp::AtMost => p.rating <= self.rating,
            };
            if !ok {
                return false;
            }
        }
        if (self.favorite && !p.favorite) || self.flag.is_some_and(|f| f != p.flag) || self.label.is_some_and(|l| Some(l) != p.label) || self.kind.is_some_and(|k| k != p.kind) {
            return false;
        }
        if self.edited.is_some_and(|e| e != p.is_edited()) {
            return false;
        }
        if let Some(rs) = &self.rule_set
            && !rs.matches(p, cat)
        {
            return false;
        }
        if (!self.labels.is_empty() || self.no_label) && !(p.label.is_some_and(|l| self.labels.contains(&l)) || (self.no_label && p.label.is_none()))
        {
            return false;
        }
        if self.virtual_copy.is_some_and(|c| c != p.copy_of.is_some()) {
            return false;
        }
        if self.stacked.is_some_and(|st| st != cat.stack_of(p.id).is_some()) {
            return false;
        }
        if !self.columns.is_empty() && !crate::columns::matches(&self.columns, p, cat) {
            return false;
        }
        if !self.only.is_empty() && !self.only.contains(&p.id) {
            return false;
        }
        if let Some(want) = &self.merged {
            match merged_kind(&p.file_name) {
                Some(k) if want == "any" || want == k => {}
                _ => return false,
            }
        }
        if let Some(a) = self.album
            && !cat.album_contains(a, p)
        {
            return false;
        }
        if let Some(from) = &self.date_from
            && p.date() < from.as_str()
        {
            return false;
        }
        if let Some(to) = &self.date_to
            && p.date().get(..to.len()).unwrap_or(p.date()) > to.as_str()
        {
            return false;
        }
        if let Some(l) = &self.lens
            && !p.meta.lens.to_lowercase().contains(&l.to_lowercase())
        {
            return false;
        }
        if let Some(d) = &self.date
            && !p.date().starts_with(d.as_str())
        {
            return false;
        }
        if let Some(root) = root
            && !(if self.library_folder_only { photo_directly_in(p, root) } else { photo_in_root(p, root) })
        {
            return false;
        }
        if let Some(d) = &self.imported
            && !p.imported.starts_with(d.as_str())
        {
            return false;
        }
        if let Some(d) = &self.imported_from
            && p.imported.as_str() < d.as_str()
        {
            return false;
        }
        if let Some(k) = &self.keyword
            && !p.meta.keywords.iter().any(|x| crate::keywords::is_under(x, k))
        {
            return false;
        }
        if let Some(n) = &self.person
            && !has_person(p, n)
        {
            return false;
        }
        if let Some(c) = &self.camera
            && !p.meta.camera.eq_ignore_ascii_case(c)
        {
            return false;
        }
        if self.text_target.is_none() && self.text_rule.is_none() {
            return self.text.split_whitespace().all(|tok| token_matches(p, tok));
        }
        text_rule_matches(p, &self.text, self.text_target.as_deref().unwrap_or("any"), self.text_rule.as_deref().unwrap_or("containsAll"))
    }
}

impl Catalog {
    /// Photos matching `filter`, in `sort` order (ties broken by id for stability).
    /// How many photos [`Catalog::query`] would return, plus `keep` on each: no list, no sort (a
    /// sort of 15,865 photos was ~5 ms of the frame after every flag, QA-PERF).
    pub fn count(&self, filter: &Filter, keep: impl Fn(&Photo) -> bool) -> usize {
        let root = filter.library_root();
        self.photos().filter(|p| filter.matches_in(p, self, root.as_deref()) && keep(p)).count()
    }

    pub fn query(&self, filter: &Filter, sort: &Sort) -> Vec<PhotoId> {
        let root = filter.library_root();
        let mut v: Vec<&Photo> = self.photos().map(|p| p.as_ref()).filter(|p| filter.matches_in(p, self, root.as_deref())).collect();
        v.sort_by(|a, b| {
            let o = match sort.key {
                SortKey::CaptureDate => a.date().cmp(b.date()),
                SortKey::ImportDate => a.imported.cmp(&b.imported),
                SortKey::EditDate => a.edited.cmp(&b.edited),
                SortKey::FileName => a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()),
                SortKey::Rating => a.rating.cmp(&b.rating),
                SortKey::FileSize => a.file_size.cmp(&b.file_size),
                SortKey::Random => shuffle_rank(sort.seed, a.id).cmp(&shuffle_rank(sort.seed, b.id)),
                SortKey::LabelColor => a.label.map(|l| l as u8).cmp(&b.label.map(|l| l as u8)),
                SortKey::LabelText => {
                    let name = |p: &Photo| p.label.map(|l| self.label_name(l).to_lowercase()).unwrap_or_default();
                    name(a).cmp(&name(b))
                }
                SortKey::EditCount => a.history.len().cmp(&b.history.len()),
                // w₁/h₁ vs w₂/h₂ without division (a zero height sorts as the widest)
                SortKey::AspectRatio => (u64::from(a.width) * u64::from(b.height.max(1))).cmp(&(u64::from(b.width) * u64::from(a.height.max(1)))),
                SortKey::FileType => a.format.to_lowercase().cmp(&b.format.to_lowercase()),
                SortKey::Extension => {
                    let ext = |p: &Photo| p.file_name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
                    ext(a).cmp(&ext(b))
                }
                SortKey::PixelCount => (u64::from(a.width) * u64::from(a.height)).cmp(&(u64::from(b.width) * u64::from(b.height))),
                SortKey::Pick => {
                    let rank = |p: &Photo| match p.flag {
                        Flag::Reject => 0u8,
                        Flag::None => 1,
                        Flag::Pick => 2,
                    };
                    rank(a).cmp(&rank(b))
                }
            }
            .then(a.id.cmp(&b.id));
            if sort.ascending { o } else { o.reverse() }
        });
        v.into_iter().map(|p| p.id).collect()
    }

    /// Year → month → day counts for the "By Date" section (newest first).
    pub fn date_groups(&self) -> Vec<DateGroup> {
        use std::collections::BTreeMap;
        let mut years: BTreeMap<&str, (BTreeMap<&str, usize>, BTreeMap<&str, usize>)> = BTreeMap::new();
        for p in self.photos().filter(|p| p.in_library()) {
            let d = p.date();
            if d.len() >= 10 {
                let (months, days) = years.entry(&d[..4]).or_default();
                *months.entry(&d[..7]).or_default() += 1;
                *days.entry(&d[..10]).or_default() += 1;
            }
        }
        let newest_first = |m: BTreeMap<&str, usize>| m.into_iter().rev().map(|(k, n)| (k.to_string(), n)).collect::<Vec<_>>();
        years
            .into_iter()
            .rev()
            .map(|(year, (months, days))| DateGroup {
                year: year.to_string(),
                count: months.values().sum(),
                months: newest_first(months),
                days: newest_first(days),
            })
            .collect()
    }

    /// All keywords with usage counts, sorted by name.
    pub fn keywords(&self) -> Vec<(String, usize)> {
        let mut m: std::collections::BTreeMap<String, usize> = Default::default();
        for p in self.photos().filter(|p| p.in_library()) {
            for k in &p.meta.keywords {
                *m.entry(k.clone()).or_default() += 1;
            }
        }
        m.into_iter().collect()
    }

    /// The people named on faces in the library (MWG regions read from XMP): how many photos each
    /// appears in and the photo showing their largest face (for a card's picture). Most photos first,
    /// then by name. Names that differ only in case are one person, shown as first seen; a person
    /// twice in one photo counts once.
    pub fn people(&self) -> Vec<Person> {
        self.people_in(&Filter::default())
    }

    /// [`Self::people`] among the photos `filter` lets through (its `person` is ignored): what the
    /// People view offers while other filters (a date, a rating, an album…) are active, so picking
    /// a person never ends in an empty grid.
    pub fn people_in(&self, filter: &Filter) -> Vec<Person> {
        let filter = Filter { person: None, ..filter.clone() };
        let mut m: std::collections::HashMap<String, (Person, f64)> = Default::default();
        let root = filter.library_root();
        for p in self.photos().filter(|p| filter.matches_in(p, self, root.as_deref())) {
            let mut seen: Vec<String> = Vec::new();
            for r in p.meta.regions.iter().filter(|r| r.kind == lightcraft_meta::RegionKind::Face) {
                let Some(name) = r.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) else { continue };
                let key = name.to_lowercase();
                // the face's size in pixels of the photo
                let area = (r.rect.x1 - r.rect.x0) * (r.rect.y1 - r.rect.y0) * p.width as f64 * p.height as f64;
                let (person, best) =
                    m.entry(key.clone()).or_insert_with(|| (Person { name: name.to_string(), count: 0, photo: p.id, face: r.rect }, area));
                if !seen.contains(&key) {
                    person.count += 1;
                    seen.push(key);
                }
                if area > *best || (area == *best && p.id < person.photo) {
                    (person.photo, person.face, *best) = (p.id, r.rect, area);
                }
            }
        }
        let mut v: Vec<Person> = m.into_values().map(|(p, _)| p).collect();
        v.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        v
    }
}

/// A person named on faces in the library ([`Catalog::people`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub name: String,
    /// Photos they appear in.
    pub count: usize,
    /// The photo with their largest face, and that face (normalized, in the photo's upright frame).
    pub photo: PhotoId,
    pub face: lightcraft_meta::Rect,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DateGroup {
    pub year: String,
    pub count: usize,
    /// `YYYY-MM` and counts, newest first.
    pub months: Vec<(String, usize)>,
    /// `YYYY-MM-DD` and counts, newest first.
    pub days: Vec<(String, usize)>,
}

/// Whether file `path` is directly in `dir` (or anywhere below it with `deep`). Both `/` and `\\`
/// separate.
pub fn in_folder(path: &str, dir: &str, deep: bool) -> bool {
    let dir = dir.trim_end_matches(['/', '\\']);
    let Some(rest) = path.strip_prefix(dir) else { return false };
    let Some(rest) = rest.strip_prefix(['/', '\\']) else { return false };
    !rest.is_empty() && (deep || !rest.contains(['/', '\\']))
}

/// A path read the way [`folder_key`] reads it: the part `..` can't climb above (`//server/share`,
/// `c:`; empty for `/` and relative paths), the names below it, and whether it is absolute.
pub(crate) struct SplitPath {
    pub prefix: String,
    pub parts: Vec<String>,
    pub absolute: bool,
}

/// Both separators, repeated separators (also between a server and its share), `.` and a
/// trailing separator dropped, `..` resolved lexically, a Windows verbatim prefix removed;
/// `lower_drive` lower-cases a drive letter (the identity does, a display name wants it as is).
pub(crate) fn split_path(path: &str, lower_drive: bool) -> SplitPath {
    let mut s = path.replace('\\', "/");
    if let Some(rest) = s.strip_prefix("//?/").or_else(|| s.strip_prefix("//./")) {
        s = match rest.strip_prefix("UNC/") {
            Some(unc) => format!("//{unc}"),
            None => rest.to_string(),
        };
    }
    let (prefix, rest) = if let Some(unc) = s.strip_prefix("//").filter(|u| !u.is_empty() && !u.starts_with('/')) {
        let mut segs = unc.split('/').filter(|x| !x.is_empty());
        let (server, share) = (segs.next().unwrap_or(""), segs.next().unwrap_or(""));
        (format!("//{server}/{share}"), segs.collect::<Vec<_>>().join("/"))
    } else if let (true, Some(drive), Some(rest)) = (matches!(s.as_bytes(), [a, b':', ..] if a.is_ascii_alphabetic()), s.get(..2), s.get(2..)) {
        (if lower_drive { drive.to_ascii_lowercase() } else { drive.to_string() }, rest.to_string())
    } else {
        (String::new(), s.clone())
    };
    let absolute = rest.starts_with('/') || !prefix.is_empty();
    let mut parts: Vec<String> = Vec::new();
    for c in rest.split('/') {
        match c {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| p != "..") => {
                parts.pop();
            }
            ".." if absolute => {}
            c => parts.push(c.to_string()),
        }
    }
    SplitPath { prefix, parts, absolute }
}

/// A folder path's identity, for telling whether two spellings name the same folder: `/` and
/// `\\` are both separators, repeated separators, `.` and a trailing separator are dropped,
/// `..` is resolved lexically, a Windows verbatim prefix (`\\?\`) is removed and the drive
/// letter lower-cased; on Windows (case-insensitive file names) the whole path is lower-cased.
/// File names are otherwise compared as written, so on a case-insensitive disk elsewhere
/// (macOS, a Samba share) two spellings that differ in case are two folders here.
/// No file-system access: the paths compared should already be absolute.
pub fn folder_key(path: &str) -> String {
    let SplitPath { prefix, parts, absolute } = split_path(path, true);
    let key = if absolute { format!("{prefix}/{}", parts.join("/")) } else { parts.join("/") };
    if cfg!(windows) { key.to_lowercase() } else { key }
}

/// Whether `path` is folder `root` or lies somewhere inside it, however either is spelled
/// (see [`folder_key`]).
pub fn folder_within(path: &str, root: &str) -> bool {
    key_within(&folder_key(path), &folder_key(root))
}

/// Whether a library photo (not one only browsed in Local) lies in the folder whose
/// [`folder_key`] is `root`. A plain POSIX path (no `\`, `.`, `..`, repeated or trailing
/// separators) is its own identity, so it is read without building a key for it: the check runs
/// for every photo of the library on each refresh, and this keeps it cheap.
pub(crate) fn photo_in_root(p: &Photo, root: &str) -> bool {
    if p.local {
        return false;
    }
    let crate::Source::File { path } = &p.source else { return false };
    let plain = path.starts_with('/')
        && !path.starts_with("//")
        && !path.ends_with('/')
        && !path.contains(['\\'])
        && !path.contains("//")
        && !path.contains("/.");
    if !cfg!(windows) && plain && root.starts_with('/') && !root.starts_with("//") {
        let r = root.trim_end_matches('/');
        return path.strip_prefix(r).is_some_and(|rest| rest.starts_with('/'));
    }
    crate::local::folder_of(p).is_some_and(|f| key_within(&f, root))
}

/// Whether a library photo lies directly in the folder whose [`folder_key`] is `root` (not in a
/// folder inside it).
pub(crate) fn photo_directly_in(p: &Photo, root: &str) -> bool {
    !p.local && crate::local::folder_of(p).is_some_and(|f| !root.is_empty() && f.trim_end_matches('/') == root.trim_end_matches('/'))
}

/// [`folder_within`] for two paths already turned into their [`folder_key`].
pub(crate) fn key_within(p: &str, r: &str) -> bool {
    // `.` or `a/..` name no folder: not "everything"
    if r.is_empty() {
        return false;
    }
    // a disk's root key ends in `/` (`c:/`, `//srv/share/`), a folder's does not: same folder
    let (p, r) = (p.trim_end_matches('/'), r.trim_end_matches('/'));
    p == r || p.strip_prefix(r).is_some_and(|rest| rest.starts_with('/'))
}

/// The kind of Photo Merge result a file is, from the name merges give it (`IMG_1-HDR.dng`,
/// `IMG_1-Pano.dng`, `IMG_1-HDR-Pano.dng` — also the names other raw editors use):
/// `hdr`, `panorama` or `hdrPanorama`.
pub fn merged_kind(file_name: &str) -> Option<&'static str> {
    let stem = file_name.rsplit_once('.').map_or(file_name, |(s, _)| s).to_ascii_lowercase();
    let stem = stem.trim_end_matches(|c: char| c.is_ascii_digit() || c == '-' || c == ' ');
    if stem.ends_with("-hdr-pano") {
        Some("hdrPanorama")
    } else if stem.ends_with("-hdr") {
        Some("hdr")
    } else if stem.ends_with("-pano") {
        Some("panorama")
    } else {
        None
    }
}
