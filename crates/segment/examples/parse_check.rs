//! Runs a SegFormer parser on a photo and writes one PNG per class group (white = selected), for
//! looking at the result: `cargo run -p lightcraft-segment --release --example parse_check --
//! <model dir> <photo.jpg> <out dir> [long edge] [class,class…]…`. Without classes it prints the
//! classes found (share of the photo) and writes the most likely class as a colour map.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: parse_check <model dir> <photo> <out dir> [long edge] [classes…]";
    let dir = std::path::PathBuf::from(args.next().ok_or(usage)?);
    let photo = args.next().ok_or(usage)?;
    let out = std::path::PathBuf::from(args.next().ok_or(usage)?);
    let long: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(1024);
    let groups: Vec<String> = args.collect();
    std::fs::create_dir_all(&out)?;
    let img = image::open(&photo)?.to_rgb8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let t = std::time::Instant::now();
    let model = lightcraft_segment::segformer::SegFormer::load(&dir, lightcraft_segment::best_device())?;
    eprintln!("load {:?} on {:?}", t.elapsed(), model.device());
    for run in 0..2 {
        let t = std::time::Instant::now();
        let p = model.parse(img.as_raw(), w, h, long)?;
        eprintln!("run {run}: parse {w}×{h} at {long} → {}×{} in {:?}", p.vw, p.vh, t.elapsed());
        if run == 0 {
            continue;
        }
        let labels = model.labels();
        if groups.is_empty() {
            let am = p.argmax();
            let mut count = vec![0usize; labels.len()];
            am.iter().for_each(|c| count[*c] += 1);
            let mut found: Vec<(usize, usize)> = count.iter().copied().enumerate().filter(|c| c.1 > 0).collect();
            found.sort_by_key(|c| std::cmp::Reverse(c.1));
            for (c, n) in found.iter().take(15) {
                eprintln!("  {:>5.1} %  {}", *n as f64 * 100.0 / am.len() as f64, labels[*c]);
            }
            let colour = |c: usize| [(c * 67 % 256) as u8, (c * 151 % 256) as u8, (c * 211 % 256) as u8];
            let map = image::RgbImage::from_fn(p.vw as u32, p.vh as u32, |x, y| image::Rgb(colour(am[y as usize * p.vw + x as usize])));
            map.save(out.join("argmax.png"))?;
        }
        for g in &groups {
            let names: Vec<&str> = g.split(',').collect();
            let idx = lightcraft_segment::parsers::indices(labels, &names);
            let prob = p.prob(&idx);
            let m = image::GrayImage::from_fn(p.vw as u32, p.vh as u32, |x, y| image::Luma([(prob[y as usize * p.vw + x as usize] * 255.0) as u8]));
            m.save(out.join(format!("{}.png", g.replace(',', "+").replace(' ', "_"))))?;
        }
    }
    Ok(())
}
