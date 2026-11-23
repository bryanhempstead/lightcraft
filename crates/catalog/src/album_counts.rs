//! Album counts for the sources list, recounted after every catalog change without re-testing
//! every photo against every smart album.
//!
//! A smart album's count tests its rules on each photo of the library: on a 15,865-photo library
//! that was 10–30 ms of the frame after every flag or star while culling (QA-PERF). Photos are
//! copy-on-write `Arc`s, so a photo a change didn't touch is the same `Arc` as before:
//! [`AlbumCountMemo`] keeps which smart albums each photo matched and re-tests only the photos
//! that changed. Everything else the rules can look at — the albums (an "album is" rule, the
//! rules themselves), stacks, custom label names and the clock ("in the last…") — is compared
//! with what the memo was built from, and any difference starts it over.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::{Album, AlbumId, Catalog, ColorLabel, Photo, PhotoId, Stack, StackId};

/// See the module docs. Holds the photos it last saw (one old version of each changed photo
/// until the next count).
#[derive(Default)]
pub struct AlbumCountMemo {
    context: Option<Context>,
    /// The smart albums, in the bit order of `photos`' masks (at most 64; more aren't memoized).
    smart: Vec<AlbumId>,
    photos: HashMap<PhotoId, (Arc<Photo>, u64)>,
    /// Photos tested against the smart albums by the last count (tests check that a one-photo
    /// change re-tests one photo).
    pub retested: usize,
}

#[derive(PartialEq)]
struct Context {
    albums: BTreeMap<AlbumId, Album>,
    stacks: BTreeMap<StackId, Stack>,
    label_names: BTreeMap<ColorLabel, String>,
    now: String,
}

impl Catalog {
    /// [`Catalog::album_count`] of every album, re-testing only the photos that changed since
    /// `memo`'s last count. `now`: what "in the last…" rules count back from (the caller sets
    /// [`crate::rules::set_now`]; any value that changes when it does).
    pub fn album_counts(&self, memo: &mut AlbumCountMemo, now: &str) -> HashMap<AlbumId, usize> {
        let same = memo
            .context
            .as_ref()
            .is_some_and(|c| c.albums == self.albums && c.stacks == self.stacks && c.label_names == self.label_names && c.now == now);
        if !same {
            memo.photos.clear();
            memo.smart = self.albums.values().filter(|a| a.is_smart()).map(|a| a.id).collect();
            memo.context = Some(Context {
                albums: self.albums.clone(),
                stacks: self.stacks.clone(),
                label_names: self.label_names.clone(),
                now: now.to_string(),
            });
        }
        let mut out: HashMap<AlbumId, usize> = self.albums.values().filter(|a| !a.is_smart()).map(|a| (a.id, self.album_count(a.id))).collect();
        memo.retested = 0;
        if memo.smart.len() > 64 {
            // too many for a mask: count each one in full
            memo.photos.clear();
            out.extend(memo.smart.iter().map(|id| (*id, self.album_count(*id))));
            memo.retested = self.photos.len();
            return out;
        }
        let mut counts = vec![0usize; memo.smart.len()];
        let mut photos = HashMap::with_capacity(self.photos.len());
        for p in self.photos.values() {
            let mask = match memo.photos.get(&p.id) {
                Some((seen, mask)) if Arc::ptr_eq(seen, p) => *mask,
                _ => {
                    memo.retested += 1;
                    memo.smart.iter().enumerate().filter(|(_, a)| self.album_contains(**a, p)).fold(0u64, |m, (i, _)| m | (1 << i))
                }
            };
            for (i, n) in counts.iter_mut().enumerate() {
                *n += usize::from(mask & (1 << i) != 0);
            }
            photos.insert(p.id, (p.clone(), mask));
        }
        memo.photos = photos;
        out.extend(memo.smart.iter().copied().zip(counts));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Filter, Flag, Op, RuleSet, Source};

    fn photo(id: u64) -> Op {
        Op::AddPhoto {
            photo: Box::new(Photo::new(PhotoId(id), Source::Demo { scene: 0 }, &format!("p{id}.jpg"), "JPEG", 60, 40, "2026-01-01T00:00:00")),
        }
    }

    fn smart(id: u64, rules: serde_json::Value) -> Op {
        let mut a = Album::new(AlbumId(id), format!("smart {id}"));
        let rules: RuleSet = serde_json::from_value(rules).unwrap();
        a.smart = Some(Box::new(Filter { rule_set: Some(rules), ..Default::default() }));
        Op::AddAlbum { album: a }
    }

    /// The memoized counts equal a full count after every kind of change, and a change to one
    /// photo re-tests that photo only.
    #[test]
    fn memoized_counts_match_a_full_count_and_retest_only_changed_photos() {
        let mut cat = Catalog::default();
        for i in 1..=200 {
            cat.apply(photo(i)).unwrap();
        }
        cat.apply(Op::AddAlbum { album: Album::new(AlbumId(1), "plain") }).unwrap();
        cat.apply(Op::SetAlbumPhotos { id: AlbumId(1), photos: vec![PhotoId(3), PhotoId(4)] }).unwrap();
        cat.apply(smart(10, serde_json::json!({"rules": [{"field": "flag", "op": "is", "value": "pick"}]}))).unwrap();
        cat.apply(smart(11, serde_json::json!({"rules": [{"field": "label", "op": "is", "value": "keepers"}]}))).unwrap();
        cat.apply(smart(12, serde_json::json!({"rules": [{"field": "album", "op": "is", "value": 1}]}))).unwrap();
        let mut memo = AlbumCountMemo::default();
        let full = |cat: &Catalog| cat.albums().map(|a| (a.id, cat.album_count(a.id))).collect::<HashMap<_, _>>();
        assert_eq!(cat.album_counts(&mut memo, ""), full(&cat));
        assert_eq!(memo.retested, 200);
        assert_eq!(cat.album_counts(&mut memo, ""), full(&cat));
        assert_eq!(memo.retested, 0, "nothing changed: nothing re-tested");
        // a flag (culling): one photo re-tested
        cat.apply(Op::SetFlag { id: PhotoId(7), flag: Flag::Pick }).unwrap();
        let n = cat.album_counts(&mut memo, "");
        assert_eq!((n[&AlbumId(10)], memo.retested), (1, 1));
        assert_eq!(n, full(&cat));
        // a label, then the label's custom name (not a photo change): counts follow
        cat.apply(Op::SetLabel { id: PhotoId(8), label: Some(ColorLabel::Red) }).unwrap();
        assert_eq!(cat.album_counts(&mut memo, "")[&AlbumId(11)], 0);
        cat.apply(Op::SetLabelName { label: ColorLabel::Red, name: Some("Keepers".into()) }).unwrap();
        assert_eq!(cat.album_counts(&mut memo, "")[&AlbumId(11)], 1);
        assert_eq!(cat.album_counts(&mut memo, ""), full(&cat));
        // album membership (no photo changes) feeds an "album is" rule
        cat.apply(Op::SetAlbumPhotos { id: AlbumId(1), photos: vec![PhotoId(3), PhotoId(4), PhotoId(5)] }).unwrap();
        let n = cat.album_counts(&mut memo, "");
        assert_eq!((n[&AlbumId(1)], n[&AlbumId(12)]), (3, 3));
        // removed photos leave the counts
        cat.apply(Op::RemovePhoto { id: PhotoId(7) }).unwrap();
        assert_eq!(cat.album_counts(&mut memo, ""), full(&cat));
        assert_eq!(cat.album_counts(&mut memo, "")[&AlbumId(10)], 0);
    }
}
