//! Scene analysis over depth frames: valid zones, the nearest object and where it is, motion
//! between frames, presence against a learned background, left/middle/right sector distances,
//! and per-zone noise. Pure functions plus one small stateful tracker, all unit-tested.

/// A zone reading is valid when it is inside the sensor's rated range (20-3500 mm by default).
/// 0 and out-of-range values mean "no target" (the sensor's own status flags are not exposed
/// by the board firmware, so this is the only validity signal available).
pub fn valid(d: u16, max_mm: u16) -> bool {
    (20..=max_mm).contains(&d)
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct Nearest {
    pub mm: u16,
    pub x: usize,
    pub y: usize,
}

pub fn nearest(frame: &[u16], side: usize, max_mm: u16) -> Option<Nearest> {
    frame
        .iter()
        .enumerate()
        .filter(|(_, &d)| valid(d, max_mm))
        .min_by_key(|(_, &d)| d)
        .map(|(i, &d)| Nearest {
            mm: d,
            x: i % side,
            y: i / side,
        })
}

pub fn valid_fraction(frame: &[u16], max_mm: u16) -> f64 {
    if frame.is_empty() {
        return 0.0;
    }
    frame.iter().filter(|&&d| valid(d, max_mm)).count() as f64 / frame.len() as f64
}

pub fn median_mm(frame: &[u16], max_mm: u16) -> Option<f64> {
    let mut v: Vec<u16> = frame
        .iter()
        .copied()
        .filter(|&d| valid(d, max_mm))
        .collect();
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    let m = v.len() / 2;
    Some(if v.len().is_multiple_of(2) {
        (f64::from(v[m - 1]) + f64::from(v[m])) / 2.0
    } else {
        f64::from(v[m])
    })
}

/// Mean absolute change (mm) over zones valid in both frames.
pub fn motion_mm(prev: &[u16], cur: &[u16], max_mm: u16) -> Option<f64> {
    let d: Vec<f64> = prev
        .iter()
        .zip(cur)
        .filter(|(a, b)| valid(**a, max_mm) && valid(**b, max_mm))
        .map(|(a, b)| (f64::from(*a) - f64::from(*b)).abs())
        .collect();
    if d.is_empty() {
        None
    } else {
        Some(d.iter().sum::<f64>() / d.len() as f64)
    }
}

/// Nearest valid distance in the left, middle and right thirds of the columns.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct Sectors {
    pub left: Option<u16>,
    pub middle: Option<u16>,
    pub right: Option<u16>,
}

pub fn sectors(frame: &[u16], side: usize, max_mm: u16) -> Sectors {
    let third = |lo: usize, hi: usize| {
        frame
            .iter()
            .enumerate()
            .filter(|(i, &d)| (lo..hi).contains(&(i % side)) && valid(d, max_mm))
            .map(|(_, &d)| d)
            .min()
    };
    let (a, b) = if side == 8 { (3, 5) } else { (1, 3) };
    Sectors {
        left: third(0, a),
        middle: third(a, b),
        right: third(b, side),
    }
}

/// Learns a per-zone background (median of the first frames), then reports zones that are
/// closer than the background by more than `presence_mm` as occupied.
pub struct Background {
    learn_frames: usize,
    samples: Vec<Vec<u16>>,
    pub background: Option<Vec<u16>>,
}

impl Background {
    pub fn new(learn_frames: usize) -> Self {
        Self {
            learn_frames: learn_frames.max(1),
            samples: Vec::new(),
            background: None,
        }
    }

    pub fn learned(&self) -> bool {
        self.background.is_some()
    }

    pub fn update(&mut self, frame: &[u16]) {
        if self.background.is_some() {
            return;
        }
        self.samples.push(frame.to_vec());
        if self.samples.len() >= self.learn_frames {
            let zones = frame.len();
            let bg = (0..zones)
                .map(|z| {
                    let mut v: Vec<u16> = self
                        .samples
                        .iter()
                        .filter_map(|f| f.get(z).copied())
                        .collect();
                    v.sort_unstable();
                    v[v.len() / 2]
                })
                .collect();
            self.background = Some(bg);
            self.samples.clear();
        }
    }

    /// Indices of occupied zones (nothing until the background is learned).
    pub fn occupied(&self, frame: &[u16], presence_mm: u16, max_mm: u16) -> Vec<usize> {
        let Some(bg) = &self.background else {
            return Vec::new();
        };
        frame
            .iter()
            .zip(bg)
            .enumerate()
            .filter(|(_, (d, b))| {
                valid(**d, max_mm) && (!valid(**b, max_mm) || b.saturating_sub(**d) > presence_mm)
            })
            .map(|(i, _)| i)
            .collect()
    }
}

/// Per-zone standard deviation (mm) over a set of frames; None where a zone had <2 valid reads.
pub fn zone_noise(frames: &[Vec<u16>], max_mm: u16) -> Vec<Option<f64>> {
    let zones = frames.first().map_or(0, Vec::len);
    (0..zones)
        .map(|z| {
            let v: Vec<f64> = frames
                .iter()
                .filter_map(|f| f.get(z).copied())
                .filter(|&d| valid(d, max_mm))
                .map(f64::from)
                .collect();
            if v.len() < 2 {
                return None;
            }
            let m = v.iter().sum::<f64>() / v.len() as f64;
            Some((v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64).sqrt())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall(side: usize, mm: u16) -> Vec<u16> {
        vec![mm; side * side]
    }

    #[test]
    fn nearest_and_validity_ignore_zero_and_out_of_range() {
        let mut f = wall(8, 2500);
        f[10] = 0;
        f[11] = 4000;
        f[20] = 900;
        assert_eq!(
            nearest(&f, 8, 3500),
            Some(Nearest {
                mm: 900,
                x: 4,
                y: 2
            })
        );
        assert!((valid_fraction(&f, 3500) - 62.0 / 64.0).abs() < 1e-9);
        assert_eq!(median_mm(&f, 3500), Some(2500.0));
        assert_eq!(nearest(&[0, 0], 1, 3500), None);
    }

    #[test]
    fn motion_is_mean_change_over_shared_valid_zones() {
        let a = vec![1000, 1000, 0, 1000];
        let b = vec![1100, 900, 1000, 1000];
        assert!((motion_mm(&a, &b, 3500).unwrap() - 200.0 / 3.0).abs() < 1e-9);
        assert_eq!(motion_mm(&[0], &[0], 3500), None);
    }

    #[test]
    fn sectors_split_columns_into_thirds() {
        let mut f = wall(8, 3000);
        f[8 + 1] = 500; // left
        f[8 + 4] = 700; // middle
        f[8 + 7] = 900; // right
        let s = sectors(&f, 8, 3500);
        assert_eq!(
            (s.left, s.middle, s.right),
            (Some(500), Some(700), Some(900))
        );
        assert_eq!(sectors(&wall(4, 1000), 4, 3500).middle, Some(1000));
    }

    #[test]
    fn background_learns_then_flags_closer_zones() {
        let mut bg = Background::new(3);
        for _ in 0..3 {
            bg.update(&wall(4, 2000));
        }
        assert!(bg.learned());
        let mut f = wall(4, 2000);
        f[5] = 1500;
        f[6] = 1950; // within the 150 mm presence margin
        assert_eq!(bg.occupied(&f, 150, 3500), vec![5]);
        assert!(
            Background::new(3).occupied(&f, 150, 3500).is_empty(),
            "nothing before learning"
        );
    }

    #[test]
    fn zone_noise_is_per_zone_sample_std() {
        let frames = vec![vec![1000, 0], vec![1010, 0], vec![990, 2000]];
        let n = zone_noise(&frames, 3500);
        assert!((n[0].unwrap() - 10.0).abs() < 1e-9);
        assert_eq!(n[1], None);
    }
}
