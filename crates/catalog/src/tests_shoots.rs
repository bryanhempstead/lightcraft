//! Shoots (see [`crate::shoots`]): the library's work in one flat list.
//!
//! Scenarios, in the words of a photographer:
//!
//! * A shoot is named after its folder, not after the camera or `raw` folder its photos sit in.
//! * Two camera folders of one shoot are one shoot.
//! * A folder that only gathers shoots is not a shoot itself; its shoots are.
//! * A shoot knows its disk and when it was last imported into or edited.
//! * Odd paths never crash and never make a shoot of nothing.

use crate::shoots::{context_label, is_generic_name};
use crate::*;

fn add(c: &mut Catalog, path: &str, imported: &str) {
    let id = c.alloc_photo_id();
    let p = Photo::new(id, Source::File { path: path.into() }, "x.jpg", "JPEG", 60, 40, imported);
    c.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
}

fn library(paths: &[&str]) -> Catalog {
    let mut c = Catalog::new();
    for p in paths {
        add(&mut c, p, "2026-01-01T10:00:00");
    }
    c
}

fn names(c: &Catalog) -> Vec<(String, usize, bool)> {
    c.shoots(&[]).into_iter().map(|s| (s.name, s.count, s.deep)).collect()
}

fn shoot(name: &str, count: usize) -> (String, usize, bool) {
    (name.to_string(), count, true)
}

#[test]
fn camera_format_and_card_folders_are_generic() {
    for n in [
        "raw",
        "RAW",
        "raws",
        "Raw Photos",
        "-raw",
        "photos",
        "jpeg",
        "JPG",
        "dng",
        "DCIM",
        "100MSDCF",
        "100_FUJI",
        "M262",
        "m262",
        "GR3",
        "Griii",
        "GR III",
        "X-T2",
        "X-T2 raw",
        "Canon R6",
        "ILCE-7SM3",
        "Sony A7III",
        "FUJI",
        "iPhone",
        "iPhone 13 Pro",
        "Day 1",
        "Card 2",
        "1",
        "02",
        "",
        "   ",
        "***",
        "edits",
        "Selects",
    ] {
        assert!(is_generic_name(n), "{n:?} is generic");
    }
    for n in [
        "Erika and Connor wedding",
        "*C&J WEDDING",
        "sean photos",
        "Shoot 2",
        "chicago",
        "edit now",
        "Bismarck Street Photos",
        "Ciara+Logan.Wedding.Bismarck.October.3rd.2026",
        "Photos - Iceland - Our Wedding & Honeymoon [September - October 2022] - Photos",
        "2022",
        "09-23",
        "SYNC",
        "*lifeedit",
        "Wild Terra",
        "Ålesund",
        "東京",
    ] {
        assert!(!is_generic_name(n), "{n:?} says what it is");
    }
}

#[test]
fn a_shoot_is_named_after_its_folder_not_its_camera_folders() {
    let c = library(&[
        "/Volumes/X/Erika and Connor wedding/raw/M262/a.jpg",
        "/Volumes/X/Erika and Connor wedding/raw/M262/b.jpg",
        "/Volumes/X/C&J WEDDING/X-T2 raw/c.jpg",
        "/Volumes/Y/Shoot 2/photos/d.jpg",
    ]);
    assert_eq!(names(&c), vec![shoot("C&J WEDDING", 1), shoot("Erika and Connor wedding", 2), shoot("Shoot 2", 1)]);
    let s = c.shoots(&[]);
    assert_eq!(s[1].path, "/Volumes/X/Erika and Connor wedding");
    assert_eq!((s[1].disk.as_str(), s[1].disk_name.as_str()), ("/Volumes/X", "X"));
    assert_eq!(s[2].disk_name, "Y");
}

#[test]
fn camera_folders_of_one_shoot_merge_into_one() {
    let c = library(&[
        "/Volumes/X/Erika and Connor wedding/raw/M262/a.jpg",
        "/Volumes/X/Erika and Connor wedding/raw/GR3/b.jpg",
        "/Volumes/X/Erika and Connor wedding/jpeg/c.jpg",
        "/Volumes/X/*C&J WEDDING/M262/1.jpg",
        "/Volumes/X/*C&J WEDDING/X-T2 raw/2.jpg",
        "/Volumes/X/*C&J WEDDING/griii/3.jpg",
    ]);
    assert_eq!(names(&c), vec![shoot("*C&J WEDDING", 3), shoot("Erika and Connor wedding", 3)]);
}

#[test]
fn folders_that_gather_shoots_are_not_shoots() {
    // the folders of Bryan's library, from its catalog
    let c = library(&[
        "/Users/me/a.jpg",
        "/Users/me/Downloads/b.jpg",
        "/Users/me/Pictures/Lightroom/SYNC/c.jpg",
        "/Users/me/Pictures/Lightroom/SYNC/2022/09-23/d.jpg",
        "/Volumes/HEMPSTEAD/- High Priority Edit/-current/Bismarck Street Photos/e.jpg",
        "/Volumes/HEMPSTEAD/- High Priority Edit/-current/chicago/-raw/f.jpg",
        "/Volumes/HEMPSTEAD/- High Priority Edit/-current/edit now/g.jpg",
        "/Volumes/HEMPSTEAD/- High Priority Edit/Iceland - Our Wedding/Photos - Iceland - Our Wedding - Photos/Canon R6/h.jpg",
        "/Volumes/HEMPSTEAD/- High Priority Edit/Iceland - Our Wedding/Photos - Iceland - Our Wedding - Photos/FUJI/i.jpg",
        "/Volumes/HEMPSTEAD/- High Priority Edit/Iceland - Our Wedding/Photos - Iceland - Our Wedding - Photos/iPhone/Wild Terra/j.jpg",
        "/Volumes/BONUSBOY/sean photos/k.jpg",
        "/Volumes/BONUSBOY/*lifeedit/raw/l.jpg",
    ]);
    let got = c.shoots(&[]);
    let rows: Vec<(&str, usize, bool)> = got.iter().map(|s| (s.name.as_str(), s.count, s.deep)).collect();
    assert_eq!(
        rows,
        vec![
            ("me", 1, false),
            ("Downloads", 1, false),
            ("SYNC", 2, true),
            ("*lifeedit", 1, true),
            ("sean photos", 1, true),
            ("Bismarck Street Photos", 1, true),
            ("chicago", 1, true),
            ("edit now", 1, true),
            ("Iceland - Our Wedding", 3, true),
        ]
    );
    assert_eq!(got[0].disk, "/");
}

#[test]
fn a_folder_that_only_leads_somewhere_is_named_by_where_it_leads() {
    // one client so far: the shoot is the client's folder, and stays it when a second turns up
    let one = library(&["/Volumes/X/Work/Clients/A wedding/raw/1.jpg"]);
    assert_eq!(names(&one), vec![shoot("A wedding", 1)]);
    let two = library(&["/Volumes/X/Work/Clients/A wedding/raw/1.jpg", "/Volumes/X/Work/Clients/B wedding/2.jpg"]);
    assert_eq!(names(&two), vec![shoot("A wedding", 1), shoot("B wedding", 1)]);
    assert_eq!(one.shoots(&[])[0].path, two.shoots(&[])[0].path);
    // a folder deep in system folders (a temporary folder on the startup disk)
    let tmp = library(&["/var/folders/ab/T/lc/Shoot 2/photos/1.jpg", "/var/folders/ab/T/lc/Shoot 3/2.jpg"]);
    assert_eq!(names(&tmp), vec![shoot("Shoot 2", 1), shoot("Shoot 3", 1)]);
}

#[test]
fn a_shoot_whose_name_says_nothing_carries_its_parents() {
    let c = library(&["/Users/me/Pictures/raw/a.jpg", "/Volumes/X/2022/09-23/b.jpg", "/Volumes/X/2022/09-25/c.jpg", "/Volumes/Z/raw/M262/d.jpg"]);
    let n: Vec<String> = c.shoots(&[]).into_iter().map(|s| s.name).collect();
    assert_eq!(n, vec!["Pictures › raw", "2022 › 09-23", "2022 › 09-25", "raw"]);
}

#[test]
fn a_shoot_remembers_when_it_was_last_imported_or_edited() {
    let mut c = Catalog::new();
    add(&mut c, "/Volumes/X/A/raw/1.jpg", "2026-01-01T10:00:00");
    add(&mut c, "/Volumes/X/A/raw/2.jpg", "2026-03-01T10:00:00");
    let id = c.alloc_photo_id();
    let mut p = Photo::new(id, Source::File { path: "/Volumes/X/B/3.jpg".into() }, "3.jpg", "JPEG", 60, 40, "2026-02-01T10:00:00");
    p.edited = Some("2026-04-01T10:00:00".into());
    c.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
    let s = c.shoots(&[]);
    let a = s.iter().find(|s| s.name == "A").unwrap();
    let b = s.iter().find(|s| s.name == "B").unwrap();
    assert_eq!(a.latest, "2026-03-01T10:00:00");
    assert_eq!(b.latest, "2026-04-01T10:00:00");
}

#[test]
fn folders_rows_name_a_camera_folder_by_its_shoot() {
    assert_eq!(context_label("/Volumes/X/Erika and Connor wedding/raw/M262", "M262"), "Erika and Connor wedding › M262");
    assert_eq!(context_label("/Volumes/X/C&J WEDDING", "C&J WEDDING"), "C&J WEDDING");
    assert_eq!(context_label("/Volumes/X/raw/M262", "M262"), "M262", "nothing above says more");
    assert_eq!(context_label("/Users/me/Pictures/raw", "raw"), "raw", "Pictures is a place, not a shoot");
    assert_eq!(context_label("relative/raw", "raw"), "raw");
    assert_eq!(context_label("", ""), "");
}

#[test]
fn odd_paths_never_crash_or_make_empty_shoots() {
    let deep: String = (0..300).map(|i| format!("/raw{}", i % 7)).collect::<String>() + "/x.jpg";
    let c = library(&[
        "/a.jpg",
        "/raw/b.jpg",
        "a.jpg",
        "",
        "/",
        "//server",
        "//server/share/raw/c.jpg",
        "C:\\Shoots\\Wedding\\raw\\d.jpg",
        "/Volumes/X/\u{FFFD}\u{FFFD}/e.jpg",
        "/Volumes/X/   /f.jpg",
        "/Volumes/X/🎉 party 🎉/g.jpg",
        &deep,
    ]);
    let s = c.shoots(&[]);
    assert!(s.iter().all(|s| s.count > 0 && !s.path.is_empty()), "{s:?}");
    let total: usize = s.iter().map(|s| s.count).sum();
    assert!(total <= c.photos().count());
    assert!(s.iter().any(|s| s.name == "🎉 party 🎉"));
    assert!(s.iter().any(|s| s.name == "Wedding"), "{s:?}");
    // loose photos at the top of a disk are that disk's own shoot (only those photos)
    let top = library(&["/Volumes/nas/1.jpg", "/Volumes/nas/trip/2.jpg"]);
    let rows: Vec<(String, usize, bool)> = names(&top);
    assert_eq!(rows, vec![("nas".to_string(), 1, false), shoot("trip", 1)]);
    // a folder made in the library that is still empty is no shoot
    assert!(Catalog::new().shoots(&["/Volumes/X/New".into()]).is_empty());
    for n in ["\u{0}", "\u{FFFD}", "-", "x".repeat(10_000).as_str(), "../..", "a/b\\c"] {
        let _ = is_generic_name(n);
        let _ = context_label(n, n);
    }
}
