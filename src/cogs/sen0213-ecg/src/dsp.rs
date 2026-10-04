//! ECG signal processing: display filtering, Pan-Tompkins R-peak detection, heart rate,
//! HRV and signal-quality checks. Everything is streaming (one sample in at a time) so the
//! same code serves a 10 s `--once` capture and an unbounded `--interval` run.

use std::collections::VecDeque;
use std::f64::consts::PI;

/// Direct-form-I biquad with RBJ-cookbook coefficients.
#[derive(Clone, Debug)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    fn from_raw(b: [f64; 3], a: [f64; 3]) -> Self {
        Self {
            b0: b[0] / a[0],
            b1: b[1] / a[0],
            b2: b[2] / a[0],
            a1: a[1] / a[0],
            a2: a[2] / a[0],
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    pub fn lowpass(fs: f64, f0: f64, q: f64) -> Self {
        let w = 2.0 * PI * f0 / fs;
        let (c, alpha) = (w.cos(), w.sin() / (2.0 * q));
        Self::from_raw(
            [(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0],
            [1.0 + alpha, -2.0 * c, 1.0 - alpha],
        )
    }

    pub fn highpass(fs: f64, f0: f64, q: f64) -> Self {
        let w = 2.0 * PI * f0 / fs;
        let (c, alpha) = (w.cos(), w.sin() / (2.0 * q));
        Self::from_raw(
            [(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0],
            [1.0 + alpha, -2.0 * c, 1.0 - alpha],
        )
    }

    pub fn notch(fs: f64, f0: f64, q: f64) -> Self {
        let w = 2.0 * PI * f0 / fs;
        let (c, alpha) = (w.cos(), w.sin() / (2.0 * q));
        Self::from_raw([1.0, -2.0 * c, 1.0], [1.0 + alpha, -2.0 * c, 1.0 - alpha])
    }

    pub fn step(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

const BUTTERWORTH_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// The waveform people look at: 0.5-40 Hz band-pass plus an optional mains notch, in mV.
pub struct DisplayFilter {
    stages: Vec<Biquad>,
}

impl DisplayFilter {
    pub fn new(fs: f64, mains_hz: f64) -> Self {
        let mut stages = vec![
            Biquad::highpass(fs, 0.5, BUTTERWORTH_Q),
            Biquad::lowpass(fs, 40.0_f64.min(fs * 0.45), BUTTERWORTH_Q),
        ];
        if mains_hz > 0.0 && mains_hz < fs / 2.0 {
            stages.push(Biquad::notch(fs, mains_hz, 30.0));
        }
        Self { stages }
    }

    /// Volts in, millivolts out.
    pub fn step(&mut self, volts: f64) -> f64 {
        self.stages.iter_mut().fold(volts, |x, s| s.step(x)) * 1000.0
    }
}

/// Streaming Pan-Tompkins QRS detector (Pan & Tompkins, IEEE TBME 1985): 5-15 Hz band-pass,
/// five-point derivative, squaring, 150 ms moving-window integration, adaptive thresholds and
/// a 200 ms refractory period. Returns R-peak sample indices.
pub struct QrsDetector {
    fs: f64,
    bp: [Biquad; 2],
    deriv_hist: [f64; 4],
    mwi_buf: VecDeque<f64>,
    mwi_sum: f64,
    mwi_len: usize,
    mwi_prev: [f64; 2],
    bp_hist: VecDeque<(u64, f64)>,
    search_len: usize,
    spki: f64,
    npki: f64,
    learn_until: u64,
    learn_max: f64,
    learn_sum: f64,
    learn_n: u64,
    last_qrs: Option<u64>,
    refractory: u64,
    n: u64,
}

impl QrsDetector {
    pub fn new(fs: f64) -> Self {
        let mwi_len = ((0.150 * fs).round() as usize).max(1);
        Self {
            fs,
            bp: [
                Biquad::highpass(fs, 5.0, BUTTERWORTH_Q),
                Biquad::lowpass(fs, 15.0, BUTTERWORTH_Q),
            ],
            deriv_hist: [0.0; 4],
            mwi_buf: VecDeque::with_capacity(mwi_len + 1),
            mwi_sum: 0.0,
            mwi_len,
            mwi_prev: [0.0; 2],
            bp_hist: VecDeque::new(),
            search_len: (0.300 * fs).round() as usize,
            spki: 0.0,
            npki: 0.0,
            learn_until: (2.0 * fs) as u64,
            learn_max: 0.0,
            learn_sum: 0.0,
            learn_n: 0,
            last_qrs: None,
            refractory: (0.200 * fs) as u64,
            n: 0,
        }
    }

    /// Feeds one sample (any unit; the detector is scale-free). Returns the sample index of an
    /// R peak when one is confirmed. Confirmation lags the R peak by roughly 100-200 ms.
    pub fn step(&mut self, x: f64) -> Option<u64> {
        let idx = self.n;
        self.n += 1;
        let hp = self.bp[0].step(x);
        let bp = self.bp[1].step(hp);
        self.bp_hist.push_back((idx, bp));
        while self.bp_hist.len() > self.search_len {
            self.bp_hist.pop_front();
        }

        // Five-point derivative: (2x[n] + x[n-1] - x[n-3] - 2x[n-4]) * fs / 8.
        let h = self.deriv_hist;
        let d = (2.0 * bp + h[0] - h[2] - 2.0 * h[3]) * self.fs / 8.0;
        self.deriv_hist = [bp, h[0], h[1], h[2]];

        let sq = d * d;
        self.mwi_buf.push_back(sq);
        self.mwi_sum += sq;
        if self.mwi_buf.len() > self.mwi_len {
            self.mwi_sum -= self.mwi_buf.pop_front().unwrap_or(0.0);
        }
        let mwi = self.mwi_sum / self.mwi_len as f64;

        // A local maximum of the integrated signal at idx-1.
        let [p1, p2] = self.mwi_prev;
        self.mwi_prev = [mwi, p1];
        let is_peak = p1 > p2 && p1 >= mwi && p1 > 0.0;

        if idx < self.learn_until {
            self.learn_max = self.learn_max.max(mwi);
            self.learn_sum += mwi;
            self.learn_n += 1;
            return None;
        }
        if idx == self.learn_until {
            self.spki = self.learn_max / 3.0;
            self.npki = 0.5 * self.learn_sum / self.learn_n.max(1) as f64;
        }
        if !is_peak {
            return None;
        }

        let threshold = self.npki + 0.25 * (self.spki - self.npki);
        let in_refractory = self
            .last_qrs
            .is_some_and(|q| idx.saturating_sub(q) < self.refractory);
        if p1 > threshold && !in_refractory {
            self.spki = 0.125 * p1 + 0.875 * self.spki;
            // The R peak is the largest band-passed excursion in the preceding 300 ms.
            let r = self
                .bp_hist
                .iter()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .map(|&(i, _)| i)
                .unwrap_or(idx);
            if self
                .last_qrs
                .is_some_and(|q| r <= q || r - q < self.refractory)
            {
                return None;
            }
            self.last_qrs = Some(r);
            Some(r)
        } else {
            self.npki = 0.125 * p1 + 0.875 * self.npki;
            None
        }
    }
}

/// Plausible adult RR range: 30-200 bpm.
pub const RR_MIN_MS: f64 = 300.0;
pub const RR_MAX_MS: f64 = 2000.0;

/// RR intervals in ms from R-peak sample indices, dropping physiologically implausible ones.
pub fn rr_intervals_ms(peaks: &[u64], fs: f64) -> Vec<f64> {
    peaks
        .windows(2)
        .map(|w| (w[1] - w[0]) as f64 * 1000.0 / fs)
        .filter(|rr| (RR_MIN_MS..=RR_MAX_MS).contains(rr))
        .collect()
}

pub fn median(v: &[f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let m = s.len() / 2;
    Some(if s.len().is_multiple_of(2) {
        (s[m - 1] + s[m]) / 2.0
    } else {
        s[m]
    })
}

/// Heart rate from the median RR (robust to one missed or extra beat). Needs 2+ intervals.
pub fn heart_rate_bpm(rr_ms: &[f64]) -> Option<f64> {
    if rr_ms.len() < 2 {
        return None;
    }
    median(rr_ms).map(|m| 60_000.0 / m)
}

/// SDNN: standard deviation of RR intervals. Needs 3+ intervals.
pub fn sdnn_ms(rr_ms: &[f64]) -> Option<f64> {
    if rr_ms.len() < 3 {
        return None;
    }
    let mean = rr_ms.iter().sum::<f64>() / rr_ms.len() as f64;
    let var = rr_ms.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (rr_ms.len() - 1) as f64;
    Some(var.sqrt())
}

/// RMSSD: root mean square of successive RR differences. Needs 3+ intervals.
pub fn rmssd_ms(rr_ms: &[f64]) -> Option<f64> {
    if rr_ms.len() < 3 {
        return None;
    }
    let d: Vec<f64> = rr_ms.windows(2).map(|w| (w[1] - w[0]).powi(2)).collect();
    Some((d.iter().sum::<f64>() / d.len() as f64).sqrt())
}

/// What the raw waveform says about the electrode/sensor state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LeadState {
    /// Signal moves and stays off the rails.
    Ok,
    /// More than 5% of samples within 50 mV of 0 V or 3.3 V: leads off or saturated.
    Saturated,
    /// Almost no variation: sensor unpowered, signal wire loose, or nothing connected.
    Flat,
    /// No samples to judge.
    Unknown,
}

pub const RAIL_LOW_V: f64 = 0.05;
pub const RAIL_HIGH_V: f64 = 3.25;
pub const FLAT_STD_V: f64 = 0.002;

pub fn lead_state(raw_v: &[f64]) -> LeadState {
    if raw_v.is_empty() {
        return LeadState::Unknown;
    }
    let n = raw_v.len() as f64;
    let rail = raw_v
        .iter()
        .filter(|&&v| !(RAIL_LOW_V..=RAIL_HIGH_V).contains(&v))
        .count() as f64;
    if rail / n > 0.05 {
        return LeadState::Saturated;
    }
    let mean = raw_v.iter().sum::<f64>() / n;
    let std = (raw_v.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n).sqrt();
    if std < FLAT_STD_V {
        LeadState::Flat
    } else {
        LeadState::Ok
    }
}

/// 0-1 quality: zero unless leads are OK and beats were found; otherwise penalised by RR
/// irregularity (coefficient of variation). Irregular rhythm also lowers it, so a low score
/// means "do not trust the numbers", not "arrhythmia".
pub fn quality(lead: LeadState, rr_ms: &[f64]) -> f64 {
    if lead != LeadState::Ok || rr_ms.len() < 2 {
        return 0.0;
    }
    let mean = rr_ms.iter().sum::<f64>() / rr_ms.len() as f64;
    let sd = sdnn_ms(rr_ms).unwrap_or(0.0);
    (1.0 - 2.0 * sd / mean).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ads1115::{EcgSource, Simulator};

    fn simulate(fs: f64, bpm: f64, secs: f64) -> Vec<f64> {
        let mut s = Simulator::new(fs, bpm, 60.0);
        (0..(fs * secs) as usize)
            .map(|_| s.read_volts().unwrap())
            .collect()
    }

    fn detect(samples: &[f64], fs: f64) -> Vec<u64> {
        let mut d = QrsDetector::new(fs);
        samples.iter().filter_map(|&x| d.step(x)).collect()
    }

    #[test]
    fn detects_heart_rate_on_synthetic_ecg_at_several_rates_and_sample_rates() {
        for &fs in &[250.0, 500.0] {
            for &bpm in &[50.0, 72.0, 110.0, 160.0] {
                let x = simulate(fs, bpm, 20.0);
                let peaks = detect(&x, fs);
                let hr = heart_rate_bpm(&rr_intervals_ms(&peaks, fs)).expect("hr");
                assert!(
                    (hr - bpm).abs() < 2.0,
                    "fs {fs} bpm {bpm}: got {hr:.1} from {} peaks",
                    peaks.len()
                );
            }
        }
    }

    #[test]
    fn r_peaks_land_on_the_r_wave() {
        let (fs, bpm) = (250.0, 72.0);
        let peaks = detect(&simulate(fs, bpm, 10.0), fs);
        let period = 60.0 / bpm;
        for p in peaks {
            let ph = (p as f64 / fs) % period;
            assert!(
                (ph - 0.30).abs() < 0.03,
                "peak at phase {ph:.3}s, R is at 0.300s"
            );
        }
    }

    #[test]
    fn hrv_of_a_perfectly_regular_rhythm_is_near_zero() {
        let fs = 250.0;
        let rr = rr_intervals_ms(&detect(&simulate(fs, 60.0, 20.0), fs), fs);
        assert!(sdnn_ms(&rr).unwrap() < 10.0);
        assert!(rmssd_ms(&rr).unwrap() < 10.0);
    }

    #[test]
    fn hrv_matches_hand_computed_values() {
        let rr = [800.0, 820.0, 780.0, 800.0];
        assert!((sdnn_ms(&rr).unwrap() - 16.3299).abs() < 1e-3);
        assert!((rmssd_ms(&rr).unwrap() - 28.2843).abs() < 1e-3);
        assert_eq!(heart_rate_bpm(&rr).unwrap(), 75.0);
        assert!(heart_rate_bpm(&[800.0]).is_none());
    }

    #[test]
    fn implausible_rr_intervals_are_dropped() {
        // 100 ms and 3 s gaps are noise or dropouts, not heartbeats.
        assert_eq!(rr_intervals_ms(&[0, 25, 250, 1000], 250.0), vec![900.0]);
    }

    #[test]
    fn lead_state_flags_rails_flat_and_ok() {
        assert_eq!(lead_state(&[]), LeadState::Unknown);
        assert_eq!(lead_state(&vec![3.30; 100]), LeadState::Saturated);
        assert_eq!(lead_state(&vec![0.0; 100]), LeadState::Saturated);
        assert_eq!(lead_state(&vec![1.5; 100]), LeadState::Flat);
        assert_eq!(lead_state(&simulate(250.0, 72.0, 4.0)), LeadState::Ok);
    }

    #[test]
    fn quality_is_zero_without_good_leads_or_beats() {
        assert_eq!(quality(LeadState::Flat, &[800.0, 800.0, 800.0]), 0.0);
        assert_eq!(quality(LeadState::Ok, &[800.0]), 0.0);
        assert!(quality(LeadState::Ok, &[800.0, 805.0, 795.0]) > 0.9);
    }

    #[test]
    fn display_filter_removes_dc_and_mains() {
        let fs = 250.0;
        let mut f = DisplayFilter::new(fs, 60.0);
        let out: Vec<f64> = (0..(fs as usize * 10))
            .map(|n| {
                let t = n as f64 / fs;
                f.step(1.5 + 0.05 * (2.0 * PI * 60.0 * t).sin())
            })
            .collect();
        let tail = &out[out.len() - 250..];
        let peak = tail.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
        assert!(
            peak < 2.0,
            "residual {peak:.2} mV of a 50 mV hum on a 1.5 V offset"
        );
    }
}
