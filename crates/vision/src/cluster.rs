//! Grouping faces into people (Lightroom Classic's Unnamed People stacks) and matching faces to named people
//! (its suggestions), from unit-length face embeddings compared by cosine similarity.
//!
//! Clustering is greedy and deterministic: faces are taken in the order given (callers sort them, e.g. by photo),
//! each joins the group whose mean embedding it is most similar to when that similarity reaches the threshold, else
//! starts a new group; a second pass then moves every face to the group it fits best, so the result doesn't depend
//! much on the order. Cost is faces × groups dot products of 128 numbers (≈ 0.5 s for 20,000 faces in 3,000 groups).

/// Faces at least this similar to a group's mean join it (SFace embeddings; OpenCV's same-person threshold is 0.363,
/// stricter here so that two people rarely end up in one stack).
pub const CLUSTER: f32 = 0.48;
/// A face at least this similar to a named person's faces is suggested as them.
pub const SUGGEST: f32 = 0.42;

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 && n.is_finite() {
        for x in v {
            *x /= n;
        }
    }
}

/// Groups of indices into `faces` (each group's faces in the order given), largest group first; faces of a different
/// length than the first are left out.
pub fn cluster(faces: &[&[f32]], threshold: f32) -> Vec<Vec<usize>> {
    let Some(dim) = faces.first().map(|f| f.len()) else { return Vec::new() };
    let mut sums: Vec<Vec<f32>> = Vec::new();
    let mut means: Vec<Vec<f32>> = Vec::new();
    let mut of: Vec<Option<usize>> = vec![None; faces.len()];
    let best = |means: &[Vec<f32>], f: &[f32]| -> Option<(usize, f32)> {
        means.iter().enumerate().map(|(i, m)| (i, dot(m, f))).max_by(|a, b| a.1.total_cmp(&b.1))
    };
    for (i, f) in faces.iter().enumerate() {
        if f.len() != dim || f.iter().any(|v| !v.is_finite()) {
            continue;
        }
        match best(&means, f) {
            Some((g, s)) if s >= threshold => {
                if let (Some(sum), Some(mean)) = (sums.get_mut(g), means.get_mut(g)) {
                    for (a, b) in sum.iter_mut().zip(f.iter()) {
                        *a += b;
                    }
                    mean.copy_from_slice(sum);
                    normalize(mean);
                }
                of[i] = Some(g);
            }
            _ => {
                sums.push(f.to_vec());
                let mut m = f.to_vec();
                normalize(&mut m);
                means.push(m);
                of[i] = Some(means.len() - 1);
            }
        }
    }
    // second pass: every face to the group it fits best now (its own group if nothing reaches the threshold)
    for (i, f) in faces.iter().enumerate() {
        let Some(own) = of[i] else { continue };
        if let Some((g, s)) = best(&means, f)
            && g != own
            && s >= threshold
            && s > means.get(own).map(|m| dot(m, f)).unwrap_or(f32::MIN)
        {
            of[i] = Some(g);
        }
    }
    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); means.len()];
    for (i, g) in of.iter().enumerate() {
        if let Some(g) = g.and_then(|g| groups.get_mut(g)) {
            g.push(i);
        }
    }
    groups.retain(|g| !g.is_empty());
    groups.sort_by(|a, b| b.len().cmp(&a.len()).then(a.first().cmp(&b.first())));
    groups
}

/// How well `face` matches a person whose confirmed faces are `known`: the mean of its (up to) three best
/// similarities, so one odd photo neither makes nor breaks a match.
pub fn match_score(face: &[f32], known: &[&[f32]]) -> f32 {
    let mut s: Vec<f32> = known.iter().filter(|k| k.len() == face.len()).map(|k| dot(face, k)).filter(|v| v.is_finite()).collect();
    if s.is_empty() {
        return f32::MIN;
    }
    s.sort_by(|a, b| b.total_cmp(a));
    let n = s.len().min(3);
    s.iter().take(n).sum::<f32>() / n as f32
}

/// The named person (index into `people`) `face` most likely is, with the score, when it reaches `threshold` and
/// isn't one of `rejected` (indices the user said no to for this face).
pub fn suggest(face: &[f32], people: &[Vec<&[f32]>], rejected: &[usize], threshold: f32) -> Option<(usize, f32)> {
    people
        .iter()
        .enumerate()
        .filter(|(i, _)| !rejected.contains(i))
        .map(|(i, known)| (i, match_score(face, known)))
        .filter(|(_, s)| *s >= threshold)
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: &[f32]) -> Vec<f32> {
        let mut v = v.to_vec();
        normalize(&mut v);
        v
    }

    #[test]
    fn similar_faces_group_and_others_stay_apart() {
        let a1 = unit(&[1.0, 0.1, 0.0]);
        let a2 = unit(&[1.0, -0.1, 0.05]);
        let b1 = unit(&[0.0, 1.0, 0.1]);
        let a3 = unit(&[0.9, 0.0, 0.1]);
        let odd = vec![f32::NAN, 0.0, 0.0];
        let faces: Vec<&[f32]> = vec![&a1, &b1, &a2, &a3, &odd];
        let g = cluster(&faces, 0.8);
        assert_eq!(g, vec![vec![0, 2, 3], vec![1]]);
        assert!(cluster(&[], 0.5).is_empty());
    }

    #[test]
    fn suggestions_skip_rejected_people() {
        let ann = unit(&[1.0, 0.0, 0.0]);
        let bob = unit(&[0.0, 1.0, 0.0]);
        let face = unit(&[0.9, 0.2, 0.0]);
        let people: Vec<Vec<&[f32]>> = vec![vec![&ann], vec![&bob]];
        assert_eq!(suggest(&face, &people, &[], 0.5).map(|s| s.0), Some(0));
        assert_eq!(suggest(&face, &people, &[0], 0.5), None);
        assert_eq!(match_score(&face, &[]), f32::MIN);
    }
}
