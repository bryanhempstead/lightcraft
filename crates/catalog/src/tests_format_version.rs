//! Issue #102: catalog format versions. Older formats load and are upgraded; a newer format is
//! refused with a clear error and left untouched (never read in part, truncated or set aside).

use crate::journal::{LOG, SNAPSHOT, VERSION, encode_record};
use crate::*;

fn photo(c: &mut Catalog) -> Op {
    let id = c.alloc_photo_id();
    Op::AddPhoto { photo: Box::new(Photo::new(id, Source::File { path: format!("/p/{}.jpg", id.0) }, "p.jpg", "JPEG", 4, 3, "2026-01-01")) }
}

/// A catalog with two photos and the log line for a third op on top of it.
fn legacy_parts() -> (Catalog, String, Catalog) {
    let mut c = Catalog::new();
    for _ in 0..2 {
        let op = photo(&mut c);
        c.apply(op).unwrap();
    }
    let base = c.clone();
    let id = c.photos().next().unwrap().id;
    let op = Op::SetRating { id, rating: 4 };
    c.apply(op.clone()).unwrap();
    (base, format!("{}\n", encode_record(3, &op)), c)
}

fn snapshot_version(m: &MemStore) -> u64 {
    let v: serde_json::Value = serde_json::from_slice(&m.get(SNAPSHOT).unwrap()).unwrap();
    v["version"].as_u64().unwrap()
}

#[test]
fn v1_library_loads_and_is_upgraded() {
    let (base, log, full) = legacy_parts();
    let m = MemStore::new();
    m.set(SNAPSHOT, format!("{{\"format\":\"lightcraft-catalog\",\"version\":1,\"seq\":2,\"catalog\":{}}}\n", base.to_snapshot()).into_bytes());
    m.set(LOG, log.into_bytes());
    let (mut j, c, r) = Journal::open(Box::new(m.clone())).unwrap();
    assert_eq!(c.to_snapshot(), full.to_snapshot());
    assert_eq!((r.replayed, r.upgraded_from), (1, Some(1)));
    // rewritten in this format before anything is appended
    assert_eq!(snapshot_version(&m), u64::from(VERSION));
    assert!(m.get(LOG).unwrap().is_empty());
    assert_eq!(j.seq(), 3);
    j.append(&[Op::SetLabelName { label: ColorLabel::ALL[0], name: Some("x".into()) }]).unwrap();
    drop(j);
    let (_, _, r) = Journal::open(Box::new(m)).unwrap();
    assert_eq!((r.replayed, r.upgraded_from), (1, None));
}

#[test]
fn versionless_log_only_library_is_upgraded() {
    let mut c = Catalog::new();
    let op = photo(&mut c);
    c.apply(op.clone()).unwrap();
    let m = MemStore::new();
    m.set(LOG, format!("{}\n", encode_record(1, &op)).into_bytes());
    let (_, loaded, r) = Journal::open(Box::new(m.clone())).unwrap();
    assert_eq!(loaded.to_snapshot(), c.to_snapshot());
    assert_eq!(r.upgraded_from, Some(0));
    assert_eq!(snapshot_version(&m), u64::from(VERSION));
}

/// A new library gets a snapshot in this format at once, so an older build never sees a bare log
/// it would read in part.
#[test]
fn new_library_is_written_in_this_format() {
    let m = MemStore::new();
    let (_, _, r) = Journal::open(Box::new(m.clone())).unwrap();
    assert!(r.created);
    assert_eq!(r.upgraded_from, None);
    assert_eq!(snapshot_version(&m), u64::from(VERSION));
}

fn assert_refused_untouched(m: &MemStore) {
    let before = m.files.lock().unwrap().clone();
    let e = Journal::open(Box::new(m.clone())).err().expect("refused");
    assert!(matches!(e, CatalogError::Newer(_)), "{e:?}");
    let msg = e.to_string();
    assert!(msg.contains("newer version of LightCraft") && msg.contains("left unchanged"), "{msg}");
    assert_eq!(*m.files.lock().unwrap(), before, "nothing modified, nothing renamed");
}

#[test]
fn newer_snapshot_is_refused_untouched() {
    let (base, log, _) = legacy_parts();
    let m = MemStore::new();
    m.set(
        SNAPSHOT,
        format!("{{\"format\":\"lightcraft-catalog\",\"version\":{},\"seq\":2,\"catalog\":{}}}\n", VERSION + 1, base.to_snapshot()).into_bytes(),
    );
    m.set(LOG, log.into_bytes());
    assert_refused_untouched(&m);
    // even when its catalog doesn't parse as this version's
    let m = MemStore::new();
    m.set(
        SNAPSHOT,
        format!("{{\"format\":\"lightcraft-catalog\",\"version\":{},\"seq\":2,\"catalog\":{{\"photos\":7}}}}\n", VERSION + 1).into_bytes(),
    );
    assert_refused_untouched(&m);
    let e = Journal::open(Box::new(m)).err().unwrap().to_string();
    assert!(e.contains(&format!("v{}", VERSION + 1)), "{e}");
}

/// A log record with a valid CRC whose op this version doesn't know: refused, not taken for a
/// torn tail (truncated) or damage (renamed) — wherever it is in the log.
#[test]
fn unknown_op_in_the_log_is_refused_untouched() {
    let (base, log, _) = legacy_parts();
    let future = |seq: u64| {
        let body = r#"{"op":"fromTheFuture","id":1,"what":"something new"}"#;
        format!("{{\"seq\":{seq},\"crc\":{},\"op\":{body}}}\n", crc32fast::hash(body.as_bytes()))
    };
    let snap = format!("{{\"format\":\"lightcraft-catalog\",\"version\":{VERSION},\"seq\":2,\"catalog\":{}}}\n", base.to_snapshot());
    let id = base.photos().next().unwrap().id;
    let after = format!("{}\n", encode_record(5, &Op::SetRating { id, rating: 1 }));
    for log in [format!("{log}{}", future(4)), format!("{log}{}{after}", future(4)), format!("{}{log}", future(3))] {
        let m = MemStore::new();
        m.set(SNAPSHOT, snap.clone().into_bytes());
        m.set(LOG, log.into_bytes());
        assert_refused_untouched(&m);
    }
    // a known op with a field of an unknown shape is "newer" too
    let body = r#"{"op":"setRating","id":1,"rating":"five stars"}"#;
    let m = MemStore::new();
    m.set(SNAPSHOT, snap.into_bytes());
    m.set(LOG, format!("{log}{{\"seq\":4,\"crc\":{},\"op\":{body}}}\n", crc32fast::hash(body.as_bytes())).into_bytes());
    assert_refused_untouched(&m);
}

/// A CRC mismatch is still damage / a torn tail (not "newer").
#[test]
fn bad_crc_is_still_a_torn_tail() {
    let (base, log, _) = legacy_parts();
    let m = MemStore::new();
    m.set(
        SNAPSHOT,
        format!("{{\"format\":\"lightcraft-catalog\",\"version\":{VERSION},\"seq\":2,\"catalog\":{}}}\n", base.to_snapshot()).into_bytes(),
    );
    let body = r#"{"op":"fromTheFuture"}"#;
    m.set(LOG, format!("{log}{{\"seq\":4,\"crc\":1,\"op\":{body}}}\n").into_bytes());
    let (_, _, r) = Journal::open(Box::new(m)).unwrap();
    assert!(r.torn_bytes > 0 && r.damaged.is_none(), "{r:?}");
}

/// Guard for the version rule (see `journal` → *Format versions*): adding an [`Op`] variant breaks
/// this match. When it does, bump `journal::VERSION`, add a row to the format table, and add the
/// variant here under the new version.
#[test]
fn op_variants_are_versioned() {
    fn since(op: &Op) -> u32 {
        match op {
            Op::AddPhoto { .. }
            | Op::RemovePhoto { .. }
            | Op::SetRating { .. }
            | Op::SetFlag { .. }
            | Op::SetLabel { .. }
            | Op::SetDevelop { .. }
            | Op::SetMeta { .. }
            | Op::SetDeleted { .. }
            | Op::SetLocal { .. }
            | Op::SetVersions { .. }
            | Op::SetHistory { .. }
            | Op::PushHistory { .. }
            | Op::AddAlbum { .. }
            | Op::RemoveAlbum { .. }
            | Op::RenameAlbum { .. }
            | Op::MoveAlbum { .. }
            | Op::SetAlbumPhotos { .. }
            | Op::SetAlbumCover { .. }
            | Op::SetAlbumRules { .. }
            | Op::AddStack { .. }
            | Op::RemoveStack { .. }
            | Op::SetStack { .. }
            | Op::SetCaptured { .. }
            | Op::SetAnalysis { .. }
            | Op::SetFile { .. }
            | Op::Relink { .. }
            | Op::SetContent { .. }
            | Op::SetLabelName { .. }
            | Op::Batch { .. } => 1,
            Op::SetBrowsed { .. } => 2,
            Op::SetKeyword { .. } => 3,
            Op::SetFolderRecord { .. } | Op::SetAlbumMarks { .. } => 4,
            Op::SetXmpSync { .. } => 6,
            Op::SetVideo { .. } | Op::SetRecord { .. } => 8,
            Op::SetRegions { .. } => 9,
            Op::SetFavorite { .. } => 11,
        }
    }
    let newest = since(&Op::SetFavorite { id: PhotoId(1), favorite: true });
    // a format can also add a field (v5: `Meta.iptc_ext`), so the newest op may be older
    assert!(newest <= VERSION, "an op newer than the current format version");
}

/// v8: a video's trim / poster and a library record survive a snapshot round trip, undo restores
/// them, and nonsense never inverts a trim range.
#[test]
fn v8_video_edit_and_records_round_trip() {
    let mut c = Catalog::new();
    let id = c.alloc_photo_id();
    let p = Photo::new(id, Source::File { path: "/v/clip.mov".into() }, "clip.mov", "MOV", 1920, 1080, "2026-11-22");
    c.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
    let v = VideoEdit { trim_in: 1.5, trim_out: Some(4.0), poster: Some(2.0) };
    let undo = c.apply(Op::SetVideo { id, video: Some(v) }).unwrap();
    c.apply(Op::SetRecord { key: "map.locations".into(), value: Some(serde_json::json!([{"name": "Home"}])) }).unwrap();
    let back: Catalog = serde_json::from_str(&c.to_snapshot()).unwrap();
    assert_eq!(back.photo(id).unwrap().video, Some(v));
    assert_eq!(back.record("map.locations").unwrap()[0]["name"], "Home");
    assert_eq!(back.records_with_prefix("map.").count(), 1);
    assert_eq!(back.records_with_prefix("publish.").count(), 0);
    c.apply(undo).unwrap();
    assert_eq!(c.photo(id).unwrap().video, None);
    assert!(c.apply(Op::SetRecord { key: " ".into(), value: None }).is_err());
    // a default edit is stored as none
    c.apply(Op::SetVideo { id, video: Some(VideoEdit::default()) }).unwrap();
    assert_eq!(c.photo(id).unwrap().video, None);
    assert_eq!(VideoEdit { trim_in: 5.0, trim_out: Some(2.0), poster: None }.range(Some(10.0)), (0.0, Some(10.0)));
    assert_eq!(VideoEdit { trim_in: 2.0, trim_out: None, poster: None }.range(Some(10.0)), (2.0, Some(10.0)));
    assert_eq!(VideoEdit { trim_in: f64::NAN, trim_out: Some(f64::INFINITY), poster: None }.range(Some(3.0)), (0.0, Some(3.0)));
}

/// v9: found faces, rejected suggestions and face culling scores survive a snapshot round trip and
/// undo; a v8 photo (no such fields) reads with none.
#[test]
fn v9_regions_and_face_analysis_round_trip() {
    let mut c = Catalog::new();
    let id = c.alloc_photo_id();
    let p = Photo::new(id, Source::File { path: "/p/a.jpg".into() }, "a.jpg", "JPEG", 400, 300, "2026-11-22");
    c.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
    let mut face = lightcraft_meta::Region::face(lightcraft_meta::Rect { x0: 0.1, y0: 0.1, x1: 0.3, y1: 0.4 });
    face.rejected.push("Ann".into());
    let undo = c.apply(Op::SetRegions { id, regions: vec![face.clone()] }).unwrap();
    let a = Analysis { sharpness: 50.0, faces: 2, eyes_closed: 1, face_focus: Some(33.0), ..Default::default() };
    c.apply(Op::SetAnalysis { id, analysis: Some(a) }).unwrap();
    let back: Catalog = serde_json::from_str(&c.to_snapshot()).unwrap();
    assert_eq!(back.photo(id).unwrap().meta.regions, vec![face]);
    assert_eq!(back.photo(id).unwrap().analysis, Some(a));
    c.apply(undo).unwrap();
    assert!(c.photo(id).unwrap().meta.regions.is_empty());
    // v8 data: no `rejected`, no face fields
    let r: lightcraft_meta::Region =
        serde_json::from_str(r#"{"rect":{"x0":0.1,"y0":0.1,"x1":0.2,"y1":0.2},"kind":"Face","name":"Bo","description":null}"#).unwrap();
    assert!(r.rejected.is_empty());
    let a: Analysis = serde_json::from_str(r#"{"sharpness":1.0,"clipped":0.0,"group":null,"best":false}"#).unwrap();
    assert_eq!((a.faces, a.eyes_closed, a.face_focus), (0, 0, None));
}

/// v10: a hand-drawn face and the expression scores survive a snapshot round trip; v9 data reads with neither.
#[test]
fn v10_drawn_faces_and_expressions_round_trip() {
    let mut c = Catalog::new();
    let id = c.alloc_photo_id();
    let p = Photo::new(id, Source::File { path: "/p/a.jpg".into() }, "a.jpg", "JPEG", 400, 300, "2026-11-22");
    c.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
    let face = lightcraft_meta::Region::drawn_face(lightcraft_meta::Rect { x0: 0.1, y0: 0.1, x1: 0.3, y1: 0.4 });
    c.apply(Op::SetRegions { id, regions: vec![face.clone()] }).unwrap();
    let a = Analysis { sharpness: 50.0, faces: 2, smiles: 1, expression: Some(71.5), ..Default::default() };
    c.apply(Op::SetAnalysis { id, analysis: Some(a) }).unwrap();
    let snap = c.to_snapshot();
    assert!(snap.contains("\"drawn\":true") && snap.contains("\"smiles\":1"), "{snap}");
    let back: Catalog = serde_json::from_str(&snap).unwrap();
    assert!(back.photo(id).unwrap().meta.regions[0].drawn);
    assert_eq!(back.photo(id).unwrap().analysis, Some(a));
    // a found face doesn't write the field at all
    assert!(!serde_json::to_string(&lightcraft_meta::Region::face(face.rect)).unwrap().contains("drawn"));
    let a: Analysis = serde_json::from_str(r#"{"sharpness":1.0,"clipped":0.0,"group":null,"best":false,"faces":1}"#).unwrap();
    assert_eq!((a.smiles, a.expression), (0, None));
}

/// Upgrading keeps the older format's files beside the library, once.
#[test]
fn an_upgrade_keeps_the_older_files() {
    let m = MemStore::new();
    let old = format!("{{\"format\":\"lightcraft-catalog\",\"version\":8,\"seq\":0,\"catalog\":{}}}\n", Catalog::new().to_snapshot());
    m.set(SNAPSHOT, old.clone().into_bytes());
    m.set(LOG, b"".to_vec());
    let (_, _, r) = Journal::open(Box::new(m.clone())).unwrap();
    assert_eq!(r.upgraded_from, Some(8));
    assert_eq!(m.get("catalog.snap.v8.bak"), Some(old.into_bytes()));
    assert_eq!(snapshot_version(&m), VERSION as u64);
}
