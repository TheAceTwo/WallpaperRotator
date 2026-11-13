//! Random selection that avoids recently shown photos.

use crate::model::{Asset, Orientation};
use rand::seq::SliceRandom;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

const MAX_HISTORY: usize = 200;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct History {
    recent: VecDeque<String>,
}

impl History {
    pub fn contains(&self, id: &str) -> bool {
        self.recent.iter().any(|r| r == id)
    }

    pub fn push(&mut self, id: &str) {
        self.recent.retain(|r| r != id);
        self.recent.push_back(id.to_string());
        while self.recent.len() > MAX_HISTORY {
            self.recent.pop_front();
        }
    }

    fn recent_window(&self, n: usize) -> impl Iterator<Item = &String> {
        self.recent.iter().rev().take(n)
    }
}

/// Pick one asset.
///
/// * Prefers images matching `orientation`; falls back to any image, then to
///   anything (videos still have a preview frame) so an album never yields nothing.
/// * Skips the most recent `min(candidates/2, MAX_HISTORY)` picks so photos
///   don't repeat until roughly half the album has been shown.
pub fn pick<'a, R: Rng + ?Sized>(
    assets: &'a [Asset],
    orientation: Orientation,
    history: &mut History,
    rng: &mut R,
) -> Option<&'a Asset> {
    let tiers: [&dyn Fn(&Asset) -> bool; 3] = [
        &|a| a.is_image && a.matches(orientation),
        &|a| a.is_image,
        &|_| true,
    ];
    let candidates: Vec<&Asset> = tiers
        .iter()
        .map(|f| assets.iter().filter(|a| f(a)).collect::<Vec<_>>())
        .find(|c| !c.is_empty())?;

    let window = (candidates.len() / 2).min(MAX_HISTORY);
    let blocked: Vec<&String> = history.recent_window(window).collect();
    let fresh: Vec<&Asset> = candidates
        .iter()
        .copied()
        .filter(|a| !blocked.contains(&&a.id))
        .collect();
    let pool = if fresh.is_empty() { &candidates } else { &fresh };

    let chosen = *pool.choose(rng)?;
    history.push(&chosen.id);
    Some(chosen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn asset(id: &str, image: bool, ratio: f64) -> Asset {
        Asset { id: id.into(), is_image: image, ratio: Some(ratio), mime: None }
    }

    #[test]
    fn respects_orientation_and_skips_videos() {
        let assets = vec![asset("land", true, 1.6), asset("port", true, 0.6), asset("vid", false, 1.7)];
        let mut h = History::default();
        let mut rng = StdRng::seed_from_u64(1);
        for _ in 0..20 {
            assert_eq!(pick(&assets, Orientation::Landscape, &mut h, &mut rng).unwrap().id, "land");
        }
    }

    #[test]
    fn falls_back_when_no_orientation_match() {
        let assets = vec![asset("port", true, 0.6)];
        let mut h = History::default();
        let mut rng = StdRng::seed_from_u64(2);
        assert_eq!(pick(&assets, Orientation::Landscape, &mut h, &mut rng).unwrap().id, "port");
    }

    #[test]
    fn no_immediate_repeats() {
        let assets: Vec<Asset> = (0..10).map(|i| asset(&i.to_string(), true, 1.5)).collect();
        let mut h = History::default();
        let mut rng = StdRng::seed_from_u64(3);
        let mut seq = Vec::new();
        for _ in 0..200 {
            seq.push(pick(&assets, Orientation::Any, &mut h, &mut rng).unwrap().id.clone());
        }
        // With a window of 5 out of 10, any 6 consecutive picks are distinct.
        for w in seq.windows(6) {
            let mut v = w.to_vec();
            v.sort();
            v.dedup();
            assert_eq!(v.len(), 6, "repeat within window: {w:?}");
        }
    }

    #[test]
    fn empty_album_returns_none() {
        let mut h = History::default();
        assert!(pick(&[], Orientation::Any, &mut h, &mut rand::thread_rng()).is_none());
    }
}
