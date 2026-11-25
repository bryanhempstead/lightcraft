//! `merge_eval <catalog.snap> <truth.json> [--lr-panos]`: run the merge finder on photo groups
//! someone judged by eye and print, per group, what it was judged to be against what the finder
//! now says, with the feature-match numbers of every neighbouring pair.
//!
//! Read-only: the catalog is read from the given snapshot file (pass a COPY of a library's
//! `catalog.snap`, never the live one) into memory; photos are only read. `truth.json` is a list
//! of `{"photos": [ids…], "truth": "none" | "sequence" | "hdr" | "pano" | …, "kind": "<old>"}`.
//! `--lr-panos` also checks Lightroom's own merges in the catalog (`<first>-Pano.dng`): the
//! finder, run on the frames fired with `<first>`, should find a panorama holding it.
use std::collections::BTreeMap;

use lightcraft_catalog::{Catalog, Journal, MemStore, Op, PhotoId, Store};
use lightcraft_engine::Session;
use lightcraft_engine::media::SourceLevel;
use lightcraft_engine::merge_find::{self, Thumb};
use serde_json::Value;

fn load_catalog(path: &str) -> Result<Catalog, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    let mut store = MemStore::new();
    store.write_atomic("catalog.snap", &bytes)?;
    store.write_atomic("catalog.log", &[])?;
    let (_, mut cat, _) = Journal::open(Box::new(store))?;
    // the groups found before: their frames would be left out
    let keys: Vec<String> = cat.records_with_prefix(merge_find::GROUP_PREFIX).map(|(k, _)| k.to_string()).collect();
    for key in keys {
        cat.apply(Op::SetRecord { key, value: None })?;
    }
    Ok(cat)
}

fn secs(cat: &Catalog, id: PhotoId) -> Option<f64> {
    merge_find::capture_secs(cat.photo(id)?.captured.as_deref()?)
}

/// `ids` and the photos of their folders fired within 10 s of them.
fn with_neighbours(cat: &Catalog, ids: &[PhotoId]) -> Vec<PhotoId> {
    let folders: Vec<String> = ids.iter().filter_map(|id| cat.photo(*id)).filter_map(|p| lightcraft_catalog::folder_of(p)).collect();
    let ts: Vec<f64> = ids.iter().filter_map(|id| secs(cat, *id)).collect();
    let (lo, hi) = (ts.iter().copied().fold(f64::MAX, f64::min) - 10.0, ts.iter().copied().fold(f64::MIN, f64::max) + 10.0);
    let mut out: Vec<PhotoId> = cat
        .photos()
        .filter(|p| lightcraft_catalog::folder_of(p).is_some_and(|f| folders.contains(&f)))
        .filter(|p| merge_find::capture_secs(p.captured.as_deref().unwrap_or("")).is_some_and(|t| t >= lo && t <= hi))
        .map(|p| p.id)
        .collect();
    out.extend(ids);
    out.sort();
    out.dedup();
    out
}

fn verdict(c: &[merge_find::Candidate], ids: &[PhotoId]) -> String {
    let hits: Vec<String> = c
        .iter()
        .filter(|c| c.photos.iter().any(|p| ids.contains(p)))
        .map(|c| format!("{} {} {:?}", c.kind.id(), c.confidence.id(), c.photos.iter().map(|p| p.0).collect::<Vec<_>>()))
        .collect();
    if hits.is_empty() { "none".into() } else { hits.join(" + ") }
}

fn pairs(s: &mut Session, ids: &[PhotoId]) {
    let mut ids = ids.to_vec();
    ids.sort_by(|a, b| secs(&s.catalog, *a).unwrap_or(0.0).total_cmp(&secs(&s.catalog, *b).unwrap_or(0.0)));
    let thumbs: BTreeMap<u64, Option<Thumb>> =
        ids.iter().map(|id| (id.0, s.source_now(*id, SourceLevel::Thumb).ok().and_then(|img| Thumb::of(&img)))).collect();
    for w in ids.windows(2) {
        let (Some(Some(a)), Some(Some(b))) = (thumbs.get(&w[0].0), thumbs.get(&w[1].0)) else {
            println!("    {} → {}: no thumbnail", w[0].0, w[1].0);
            continue;
        };
        let old = merge_find::link(a, b);
        let dt = secs(&s.catalog, w[1]).unwrap_or(0.0) - secs(&s.catalog, w[0]).unwrap_or(0.0);
        match merge_find::verify(a, b) {
            Some(p) => println!(
                "    {} → {} ({dt:.1} s, {}×{} feat {}/{}): matches {:3} inliers {:3} shift ({:+.3}, {:+.3}) overlap {:.2} scale {:.3} turn {:+.1}° ncc {:.2} cover {:.2} → {:?} | old {:?} {:.2}",
                w[0].0,
                w[1].0,
                a.work.width,
                a.work.height,
                a.feat.len(),
                b.feat.len(),
                p.matches,
                p.inliers,
                p.dx,
                p.dy,
                p.overlap,
                p.scale,
                p.turn,
                p.ncc,
                p.cover,
                merge_find::relation(a, b),
                old.rel,
                old.score
            ),
            None => println!(
                "    {} → {} ({dt:.1} s): no fit ({}×{} vs {}×{}, feat {}/{}) | old {:?}",
                w[0].0,
                w[1].0,
                a.work.width,
                a.work.height,
                b.work.width,
                b.work.height,
                a.feat.len(),
                b.feat.len(),
                old.rel
            ),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(snap), Some(truth)) = (args.first(), args.get(1)) else {
        return Err("usage: merge_eval <catalog.snap copy> <truth.json> [--lr-panos]".into());
    };
    let detail = !args.iter().any(|a| a == "--quiet");
    let cat = load_catalog(snap)?;
    let mut s = Session::new().with_fs();
    s.catalog = cat;
    let truth: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(truth)?)?;
    let mut table = Vec::new();
    for (i, g) in truth.iter().enumerate() {
        let ids: Vec<PhotoId> = g["photos"].as_array().map(|a| a.iter().filter_map(Value::as_u64).map(PhotoId).collect()).unwrap_or_default();
        let around = with_neighbours(&s.catalog, &ids);
        let t0 = std::time::Instant::now();
        let found = s.merge_find_candidates(&around);
        let v = verdict(&found, &ids);
        let row = format!(
            "#{:<2} truth {:<8} old {:<5} {:<6} → new {}  ({:.1} s)",
            i + 1,
            g["truth"].as_str().unwrap_or("?"),
            g["kind"].as_str().unwrap_or("?"),
            g["confidence"].as_str().unwrap_or(""),
            v,
            t0.elapsed().as_secs_f64()
        );
        println!("{row}");
        if detail {
            pairs(&mut s, &ids);
        }
        table.push(row);
    }
    if args.iter().any(|a| a == "--lr-panos") {
        let panos: Vec<(PhotoId, String, String)> = s
            .catalog
            .photos()
            .filter(|p| p.file_name.contains("-Pano"))
            .filter_map(|p| Some((p.id, p.file_name.split("-Pano").next()?.to_string(), lightcraft_catalog::folder_of(p)?)))
            .collect();
        let (mut hit, mut n) = (0, 0);
        for (pano, stem, folder) in panos {
            let Some(first) = s
                .catalog
                .photos()
                .find(|p| {
                    p.id != pano
                        && lightcraft_catalog::folder_of(p).as_deref() == Some(folder.as_str())
                        && p.file_name.split('.').next() == Some(stem.as_str())
                })
                .map(|p| p.id)
            else {
                continue;
            };
            let Some(t) = secs(&s.catalog, first) else { continue };
            let ids: Vec<PhotoId> = s
                .catalog
                .photos()
                .filter(|p| p.id != pano && !p.file_name.contains("-Pano") && lightcraft_catalog::folder_of(p).as_deref() == Some(folder.as_str()))
                .filter(|p| merge_find::capture_secs(p.captured.as_deref().unwrap_or("")).is_some_and(|x| x >= t - 0.5 && x <= t + 30.0))
                .map(|p| p.id)
                .collect();
            n += 1;
            let found = s.merge_find_candidates(&ids);
            let v = verdict(&found, &[first]);
            let ok = found.iter().any(|c| c.photos.contains(&first) && matches!(c.kind.id(), "pano" | "hdrPano"));
            hit += usize::from(ok);
            println!("LR {stem}: {} frames in 30 s → {v}", ids.len());
            if detail && (!ok || args.iter().any(|a| a == "--all-pairs")) {
                pairs(&mut s, &ids);
            }
        }
        println!("Lightroom panoramas found: {hit} / {n}");
    }
    println!("\n{}", table.join("\n"));
    Ok(())
}
