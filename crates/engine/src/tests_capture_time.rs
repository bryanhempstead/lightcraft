//! Edit Capture Time on a mixed selection (LR-LIB-CAPTURETIME): the anchor's corrected time moves
//! every selected photo by the same amount — two cameras, two days, a virtual copy, a collapsed
//! stack, a photo without a capture time — in one undo step that survives reopening; and matching
//! one camera's clock to another's.

use lightcraft_catalog::dates::iso_seconds;
use lightcraft_catalog::{Op, PhotoId, Stack};
use serde_json::json;

use crate::Session;

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lc-captime-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn time(s: &Session, id: PhotoId) -> Option<String> {
    s.catalog.photo(id).unwrap().captured.clone()
}

fn secs(t: &Option<String>) -> i64 {
    iso_seconds(t.as_deref().unwrap()).unwrap()
}

/// Two bodies, two days: camera A (a0–a2), camera B (b0, b1, and `h` hidden in b0's collapsed
/// stack), `u` without a capture time; `vc` is a virtual copy of a1.
pub(crate) struct Shoot {
    pub a: [PhotoId; 3],
    pub b: [PhotoId; 2],
    pub h: PhotoId,
    pub u: PhotoId,
    pub vc: PhotoId,
}

pub(crate) fn shoot(s: &mut Session) -> Shoot {
    let ids: Vec<PhotoId> = s.visible_cloned().iter().take(7).copied().collect();
    assert_eq!(ids.len(), 7);
    let plan = [
        (ids[0], "Canon EOS R6", Some("2026-06-13T14:34:12")),
        (ids[1], "Canon EOS R6", Some("2026-06-13T14:40:00")),
        (ids[2], "Canon EOS R6", Some("2026-06-14T09:00:00")),
        (ids[3], "ILCE-7M4", Some("2026-06-13T13:30:05")),
        // the same moment as a1, by a clock 1 h 2 min 3 s slow
        (ids[4], "ILCE-7M4", Some("2026-06-13T13:37:57")),
        (ids[5], "ILCE-7M4", Some("2026-06-13T13:30:06")),
        (ids[6], "", None),
    ];
    for (id, camera, t) in plan {
        let mut meta = s.catalog.photo(id).unwrap().meta.clone();
        meta.camera = camera.into();
        s.commit("t", Op::SetMeta { id, meta: Box::new(meta) }).unwrap();
        s.commit("t", Op::SetCaptured { id, captured: t.map(str::to_string) }).unwrap();
    }
    let r = s.execute("photo.virtualCopy", &json!({"ids": [ids[1].0]})).unwrap();
    let vc = PhotoId(r["ids"][0].as_u64().unwrap());
    assert_eq!(s.catalog.photo(vc).unwrap().copy_of, Some(ids[1]));
    let sid = s.catalog.alloc_stack_id();
    s.commit("t", Op::AddStack { stack: Stack { id: sid, photos: vec![ids[3], ids[5]], collapsed: true } }).unwrap();
    Shoot { a: [ids[0], ids[1], ids[2]], b: [ids[3], ids[4]], h: ids[5], u: ids[6], vc }
}

#[test]
fn anchor_shift_moves_a_mixed_selection_by_one_amount() {
    let dir = temp_dir("anchor");
    let mut s = Session::new();
    s.open_library(&dir, true).unwrap();
    let sh = shoot(&mut s);
    let all = [sh.a[0], sh.a[1], sh.a[2], sh.b[0], sh.b[1], sh.h, sh.u, sh.vc];
    let before: Vec<Option<String>> = all.iter().map(|id| time(&s, *id)).collect();
    let sel = [sh.a[0], sh.a[1], sh.a[2], sh.b[0], sh.b[1], sh.u];
    // the active photo is b1, but the right-clicked a0 is the anchor
    s.execute("library.select", &json!({"ids": sel.map(|i| i.0), "active": sh.b[1].0})).unwrap();
    let dry = s.execute("photo.setCaptureTime", &json!({"anchor": sh.a[0].0, "time": "3:34 pm", "dryRun": true})).unwrap();
    assert_eq!(dry["delta"], 3600, "{dry}");
    assert_eq!(all.iter().map(|id| time(&s, *id)).collect::<Vec<_>>(), before, "a dry run changes nothing");
    let r = s.execute("photo.setCaptureTime", &json!({"anchor": sh.a[0].0, "time": "3:34 pm"})).unwrap();
    // 5 dated selected + the copy + the photo in the collapsed stack; u has no time to shift
    assert_eq!(r["changed"], 7, "{r}");
    assert_eq!((r["copies"].as_u64(), r["stacked"].as_u64()), (Some(1), Some(1)), "{r}");
    assert_eq!(r["skipped"], json!([{"id": sh.u.0, "reason": "no capture time"}]));
    assert_eq!(time(&s, sh.a[0]).as_deref(), Some("2026-06-13T15:34:12"));
    for (id, b) in all.iter().zip(&before) {
        if *id == sh.u {
            assert_eq!(time(&s, *id), None);
        } else {
            assert_eq!(secs(&time(&s, *id)) - secs(b), 3600, "{id:?}");
        }
    }
    // one undo step brings all of them back, redo moves them again
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(all.iter().map(|id| time(&s, *id)).collect::<Vec<_>>(), before);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(time(&s, sh.vc).as_deref(), Some("2026-06-13T15:40:00"));
    s.execute("edit.undo", &json!({})).unwrap();

    // a step on explicit ids; the anchor must be one of them
    s.execute("photo.setCaptureTime", &json!({"ids": [sh.a[2].0], "time": "-1d"})).unwrap();
    assert_eq!(time(&s, sh.a[2]).as_deref(), Some("2026-06-13T09:00:00"));
    assert!(s.execute("photo.setCaptureTime", &json!({"ids": [sh.a[2].0], "anchor": sh.b[0].0, "time": "+1h"})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    // an anchor without a capture time gets the typed time
    let r = s.execute("photo.setCaptureTime", &json!({"ids": [sh.u.0], "time": "2026-06-13 12:00:00"})).unwrap();
    assert_eq!(r["changed"], 1);
    assert_eq!(time(&s, sh.u).as_deref(), Some("2026-06-13T12:00:00"));
    s.execute("edit.undo", &json!({})).unwrap();
    // a time-zone shift leaves the undated photo alone
    let r = s.execute("photo.setCaptureTime", &json!({"ids": [sh.a[0].0, sh.u.0], "hours": -2})).unwrap();
    assert_eq!(r["changed"], 1, "{r}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(all.iter().map(|id| time(&s, *id)).collect::<Vec<_>>(), before);

    // match camera clocks: b1 was taken at the same moment as a1 — camera B moves, camera A stays
    s.execute("library.select", &json!({"ids": sel.map(|i| i.0), "active": sh.b[1].0})).unwrap();
    assert!(s.execute("photo.setCaptureTime", &json!({"anchor": sh.b[1].0, "match": sh.b[0].0})).is_err(), "same camera");
    let r = s.execute("photo.setCaptureTime", &json!({"anchor": sh.b[1].0, "match": sh.a[1].0})).unwrap();
    assert_eq!(r["delta"], 3723, "{r}");
    assert_eq!(r["changed"], 3, "b0, b1 and h: {r}");
    assert_eq!(time(&s, sh.b[1]), time(&s, sh.a[1]));
    assert_eq!(time(&s, sh.h).as_deref(), Some("2026-06-13T14:32:09"));
    assert_eq!(time(&s, sh.a[0]), before[0]);

    // saved: reopening the library gives the same times
    let expect = s.catalog.to_snapshot();
    let moved = time(&s, sh.b[0]);
    drop(s);
    let mut s = Session::new();
    s.open_library(&dir, false).unwrap();
    assert_eq!(s.catalog.to_snapshot(), expect);
    assert_eq!(time(&s, sh.b[0]), moved);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Revert to Original / file date also take the virtual copies along (they share the file).
#[test]
fn followers_include_copies_and_collapsed_stacks() {
    let mut s = Session::with_demo();
    let sh = shoot(&mut s);
    let f = crate::capture_time::with_followers(&s, &[sh.a[1], sh.b[0]]);
    let ids: Vec<PhotoId> = f.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, vec![sh.a[1], sh.b[0], sh.vc, sh.h]);
    assert_eq!(f[2].1, Some(crate::capture_time::Follow::Copy));
    assert_eq!(f[3].1, Some(crate::capture_time::Follow::Stack));
    // selecting the copy brings its original; an expanded stack's members don't follow
    let sid = s.catalog.stack_of(sh.b[0]).unwrap().id;
    s.commit("t", Op::SetStack { id: sid, photos: vec![sh.b[0], sh.h], collapsed: false }).unwrap();
    let ids: Vec<PhotoId> = crate::capture_time::with_followers(&s, &[sh.vc, sh.b[0]]).into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids, vec![sh.vc, sh.b[0], sh.a[1]]);
}
