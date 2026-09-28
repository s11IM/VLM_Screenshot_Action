use std::collections::VecDeque;

use xcap::image::RgbaImage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrayImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

pub fn downscale_gray(rgba: &RgbaImage, long_edge: u32) -> GrayImage {
    let src_w = rgba.width();
    let src_h = rgba.height();
    let src_long = src_w.max(src_h);
    let (dst_w, dst_h) = if long_edge == 0 || src_long <= long_edge {
        (src_w.max(1), src_h.max(1))
    } else if src_w >= src_h {
        let w = long_edge.max(1);
        let h = ((src_h as u64 * w as u64 + src_w as u64 / 2) / src_w as u64).max(1) as u32;
        (w, h)
    } else {
        let h = long_edge.max(1);
        let w = ((src_w as u64 * h as u64 + src_h as u64 / 2) / src_h as u64).max(1) as u32;
        (w, h)
    };
    let raw = rgba.as_raw();
    let mut data = Vec::with_capacity((dst_w as usize) * (dst_h as usize));
    for y in 0..dst_h {
        let y0 = (y as u64 * src_h as u64) / dst_h as u64;
        let y1 = (((y as u64 + 1) * src_h as u64) / dst_h as u64).max(y0 + 1);
        for x in 0..dst_w {
            let x0 = (x as u64 * src_w as u64) / dst_w as u64;
            let x1 = (((x as u64 + 1) * src_w as u64) / dst_w as u64).max(x0 + 1);
            let mut sum: u64 = 0;
            let mut count: u64 = 0;
            for sy in y0..y1.min(src_h as u64) {
                for sx in x0..x1.min(src_w as u64) {
                    let i = ((sy as u32 * src_w + sx as u32) as usize) * 4;
                    let r = raw[i] as u64;
                    let g = raw[i + 1] as u64;
                    let b = raw[i + 2] as u64;
                    sum += (r * 299 + g * 587 + b * 114) / 1000;
                    count += 1;
                }
            }
            data.push(if count == 0 { 0 } else { (sum / count) as u8 });
        }
    }
    GrayImage {
        width: dst_w,
        height: dst_h,
        data,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ObserveParams {
    pub sample_interval_ms: u64,
    pub slow_interval_ms: u64,
    pub slowest_interval_ms: u64,
    pub sample_cost_threshold_ms: u64,
    pub long_edge: u32,
    pub grid_cols: u32,
    pub grid_rows_min: u32,
    pub grid_rows_max: u32,
    pub change_threshold: u8,
    pub change_area_min: f32,
    pub persistence_samples: u32,
    pub calibration_window_ms: u64,
    pub settle_duration_ms: u64,
    pub max_sample_gap_ms: u64,
}

impl Default for ObserveParams {
    fn default() -> Self {
        Self {
            sample_interval_ms: 150,
            slow_interval_ms: 250,
            slowest_interval_ms: 600,
            sample_cost_threshold_ms: 40,
            long_edge: 320,
            grid_cols: 24,
            grid_rows_min: 8,
            grid_rows_max: 24,
            change_threshold: 10,
            change_area_min: 0.03,
            persistence_samples: 2,
            calibration_window_ms: 1200,
            settle_duration_ms: 1800,
            max_sample_gap_ms: 900,
        }
    }
}

pub struct SampleMetrics {
    pub index: u32,
    pub changed_fraction: f32,
    pub motion_fraction: f32,
    pub top_decile_mean: f32,
    pub global_mean_shift: f32,
    pub window_mean: f32,
    pub window_peak: f32,
    pub noise_mean: f32,
    pub noise_peak: f32,
    pub noise_fraction: f32,
    pub mean_limit: f32,
    pub peak_limit: f32,
    pub fraction_limit: f32,
    pub calibrated: bool,
    pub noise_updated: bool,
    pub quietest_score: Option<f32>,
    pub stable_for_ms: u64,
    pub state: &'static str,
}

pub struct SampleOutcome {
    pub metrics: SampleMetrics,
    pub triggered: bool,
}

struct Block {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

pub struct SettleDetector {
    params: ObserveParams,
    baseline: GrayImage,
    blocks: Vec<Block>,
    history: VecDeque<(u64, GrayImage)>,
    quietest: Option<Motion>,
    noise: Motion,
    last_noise_update_ms: Option<u64>,
    unlocked: bool,
    triggered: bool,
    index: u32,
    change_run: u32,
    change_run_start: u32,
    changed_at: Option<u32>,
}

#[derive(Clone, Copy, Default)]
struct Motion {
    mean: f32,
    peak: f32,
    fraction: f32,
}

impl Motion {
    const PEAK_LIMIT_MAX: f32 = 60.0;
    const FRACTION_LIMIT_MAX: f32 = 0.02;

    fn score(self) -> f32 {
        // Keep calibration ranking unchanged; tolerance caps do not select the reference.
        (self.mean / 1.5)
            .max(self.peak / 5.0)
            .max(self.fraction / 0.003)
    }

    fn limits(self) -> Self {
        // Allow calibrated local activity, but keep the global motion budget strict.
        Self {
            mean: (self.mean * 1.5 + 0.2).clamp(0.4, 1.5),
            peak: (self.peak * 1.5 + 0.5).clamp(2.0, Self::PEAK_LIMIT_MAX),
            fraction: (self.fraction * 1.5 + 0.0005).clamp(0.001, Self::FRACTION_LIMIT_MAX),
        }
    }

    fn within(self, limit: Self) -> bool {
        self.mean <= limit.mean && self.peak <= limit.peak && self.fraction <= limit.fraction
    }

    fn adapt(self, candidate: Self) -> Self {
        // Limit the reference itself as well as the derived thresholds: a noisy
        // warmup must not leave a huge value that takes many windows to decay.
        Self {
            mean: self.mean.min(1.5) * 0.9 + candidate.mean.min(1.5) * 0.1,
            peak: self.peak.min(Self::PEAK_LIMIT_MAX) * 0.9
                + candidate.peak.min(Self::PEAK_LIMIT_MAX) * 0.1,
            fraction: self.fraction.min(Self::FRACTION_LIMIT_MAX) * 0.9
                + candidate.fraction.min(Self::FRACTION_LIMIT_MAX) * 0.1,
        }
    }
}

impl SettleDetector {
    pub fn new(params: ObserveParams, baseline: GrayImage) -> Self {
        let blocks = grid_blocks(
            baseline.width,
            baseline.height,
            params.grid_cols,
            params.grid_rows_min,
            params.grid_rows_max,
        );
        Self {
            params,
            baseline,
            blocks,
            history: VecDeque::new(),
            quietest: None,
            noise: Motion::default(),
            last_noise_update_ms: None,
            unlocked: false,
            triggered: false,
            index: 0,
            change_run: 0,
            change_run_start: 0,
            changed_at: None,
        }
    }

    pub fn on_sample(
        &mut self,
        thumb: &GrayImage,
        elapsed_ms: u64,
        unlock_at_ms: u64,
    ) -> SampleOutcome {
        let index = self.index;
        self.index += 1;

        let resized = thumb.width != self.baseline.width || thumb.height != self.baseline.height;
        if resized {
            *self = Self::new(self.params, thumb.clone());
            self.index = index + 1;
        }

        let stats = block_stats(
            thumb,
            &self.baseline,
            &self.blocks,
            self.params.change_threshold,
        );
        let n = self.blocks.len().max(1) as f32;
        let changed_fraction = stats.changed as f32 / n;
        let top_decile_mean = top_decile(&stats.diffs);
        let global_mean_shift = stats.signed_sum as f32 / stats.pixels.max(1) as f32;

        // A local result matters too; do not dilute a changing button/text area across the screen.
        let changed = changed_fraction >= self.params.change_area_min
            || stats
                .diffs
                .iter()
                .any(|diff| *diff > self.params.change_threshold as f32 * 2.0);
        if !self.triggered {
            if changed {
                if self.change_run == 0 {
                    self.change_run_start = index;
                }
                self.change_run += 1;
                if self.change_run >= self.params.persistence_samples && self.changed_at.is_none() {
                    self.changed_at = Some(self.change_run_start);
                }
            } else {
                self.change_run = 0;
                self.changed_at = None;
            }
        }

        let warming_up = elapsed_ms < unlock_at_ms;
        if !warming_up && !self.unlocked {
            // Calibration contributes noise estimates, never pre-earned stable time.
            self.history.clear();
            self.unlocked = true;
        }
        if self.history.back().is_some_and(|(at, _)| {
            elapsed_ms <= *at || elapsed_ms - *at > self.params.max_sample_gap_ms
        }) {
            self.history.clear();
        }
        self.history.push_back((elapsed_ms, thumb.clone()));
        let window_ms = if warming_up {
            self.params.calibration_window_ms
        } else {
            self.params.settle_duration_ms
        };
        while self.history.len() > 1 && elapsed_ms.saturating_sub(self.history[1].0) >= window_ms {
            self.history.pop_front();
        }
        let span_ms = elapsed_ms.saturating_sub(self.history.front().unwrap().0);
        let motion = window_motion(&self.history, &self.blocks);
        let complete_window = span_ms >= window_ms && self.history.len() >= 3;
        let new_quietest = complete_window
            && self
                .quietest
                .is_none_or(|best| motion.score() < best.score());
        if new_quietest {
            self.quietest = Some(motion);
        }
        let mut noise_updated = false;
        if warming_up && new_quietest {
            self.noise = motion;
            self.last_noise_update_ms = Some(elapsed_ms);
            noise_updated = true;
        }
        let noise = self.noise;
        let limits = noise.limits();
        let stable = motion.within(limits);
        let triggered = !self.triggered
            && !warming_up
            && self.changed_at.is_some()
            && self.history.len() >= 4
            && span_ms >= self.params.settle_duration_ms
            && stable;
        self.triggered |= triggered;

        // Assess each frame using the existing reference first. Only an already
        // quiet complete window may nudge the next reference, at most once per
        // non-overlapping confirmation interval. Motion cannot raise its own bar.
        if !warming_up
            && !self.triggered
            && complete_window
            && stable
            && self
                .last_noise_update_ms
                .is_none_or(|at| elapsed_ms.saturating_sub(at) >= self.params.settle_duration_ms)
        {
            self.noise = noise.adapt(motion);
            self.last_noise_update_ms = Some(elapsed_ms);
            noise_updated = true;
        }

        SampleOutcome {
            metrics: SampleMetrics {
                index,
                changed_fraction,
                motion_fraction: motion.fraction,
                top_decile_mean,
                global_mean_shift,
                window_mean: motion.mean,
                window_peak: motion.peak,
                noise_mean: noise.mean,
                noise_peak: noise.peak,
                noise_fraction: noise.fraction,
                mean_limit: limits.mean,
                peak_limit: limits.peak,
                fraction_limit: limits.fraction,
                calibrated: self.quietest.is_some(),
                noise_updated,
                quietest_score: self.quietest.map(Motion::score),
                stable_for_ms: if !warming_up && stable { span_ms } else { 0 },
                state: if resized {
                    "rebaselined"
                } else if self.triggered {
                    "triggered"
                } else if warming_up {
                    "calibrating"
                } else if self.changed_at.is_none() {
                    "armed"
                } else if stable {
                    "settling"
                } else {
                    "moving"
                },
            },
            triggered,
        }
    }

    pub fn params(&self) -> &ObserveParams {
        &self.params
    }

    pub fn changed_at_sample(&self) -> Option<u32> {
        self.changed_at
    }
}

struct Stats {
    changed: u32,
    diffs: Vec<f32>,
    signed_sum: i64,
    pixels: u64,
}

fn block_stats(
    thumb: &GrayImage,
    baseline: &GrayImage,
    blocks: &[Block],
    change_threshold: u8,
) -> Stats {
    let w = thumb.width as usize;
    let mut changed = 0u32;
    let mut diffs = Vec::with_capacity(blocks.len());
    let mut signed_sum: i64 = 0;
    let mut pixels: u64 = 0;
    for block in blocks {
        let mut abs_sum: u32 = 0;
        let mut count: u32 = 0;
        for y in block.y0..block.y1 {
            let row = y as usize * w;
            for x in block.x0..block.x1 {
                let i = row + x as usize;
                let t = thumb.data[i] as u16;
                let b = baseline.data[i] as u16;
                abs_sum += t.abs_diff(b) as u32;
                signed_sum += t as i64 - b as i64;
                count += 1;
            }
        }
        pixels += count as u64;
        let mean = abs_sum as f32 / count.max(1) as f32;
        diffs.push(mean);
        if mean > change_threshold as f32 {
            changed += 1;
        }
    }
    Stats {
        changed,
        diffs,
        signed_sum,
        pixels,
    }
}

fn window_motion(history: &VecDeque<(u64, GrayImage)>, blocks: &[Block]) -> Motion {
    let image = &history.front().unwrap().1;
    let mut low = image.data.clone();
    let mut high = low.clone();
    for (_, frame) in history.iter().skip(1) {
        for ((lo, hi), value) in low.iter_mut().zip(&mut high).zip(&frame.data) {
            *lo = (*lo).min(*value);
            *hi = (*hi).max(*value);
        }
    }
    let mut sum = 0u64;
    let mut active = 0u64;
    let mut peak = 0.0f32;
    for block in blocks {
        let mut block_sum = 0u64;
        for y in block.y0..block.y1 {
            for x in block.x0..block.x1 {
                let i = (y * image.width + x) as usize;
                let range = high[i] - low[i];
                block_sum += range as u64;
                active += u64::from(range >= 8);
            }
        }
        sum += block_sum;
        let count = (block.x1 - block.x0) * (block.y1 - block.y0);
        peak = peak.max(block_sum as f32 / count.max(1) as f32);
    }
    let pixels = image.data.len().max(1) as f32;
    Motion {
        mean: sum as f32 / pixels,
        peak,
        fraction: active as f32 / pixels,
    }
}

fn grid_blocks(width: u32, height: u32, cols: u32, rows_min: u32, rows_max: u32) -> Vec<Block> {
    let cols = cols.max(1).min(width.max(1));
    let width = width.max(1);
    let height = height.max(1);
    let raw = ((cols as u64 * height as u64) as f64 / width as f64).round() as u32;
    let rows = raw
        .clamp(rows_min.max(1), rows_max.max(rows_min.max(1)))
        .min(height);
    let mut blocks = Vec::with_capacity((cols as usize) * (rows as usize));
    for row in 0..rows {
        let y0 = row * height / rows;
        let y1 = if row + 1 == rows {
            height
        } else {
            (row + 1) * height / rows
        };
        for col in 0..cols {
            let x0 = col * width / cols;
            let x1 = if col + 1 == cols {
                width
            } else {
                (col + 1) * width / cols
            };
            blocks.push(Block { x0, y0, x1, y1 });
        }
    }
    blocks
}

fn top_decile(diffs: &[f32]) -> f32 {
    if diffs.is_empty() {
        return 0.0;
    }
    let mut sorted = diffs.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let k = ((diffs.len() as f32) * 0.1).ceil().max(1.0) as usize;
    let k = k.min(diffs.len());
    let slice = &sorted[diffs.len() - k..];
    slice.iter().sum::<f32>() / k as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use xcap::image::{Rgba, RgbaImage};

    fn flat(width: u32, height: u32, value: u8) -> GrayImage {
        GrayImage {
            width,
            height,
            data: vec![value; (width * height) as usize],
        }
    }

    fn paint_rect(img: &mut GrayImage, x0: u32, y0: u32, x1: u32, y1: u32, value: u8) {
        for y in y0..y1 {
            for x in x0..x1 {
                img.data[(y * img.width + x) as usize] = value;
            }
        }
    }

    fn params() -> ObserveParams {
        ObserveParams {
            grid_cols: 4,
            grid_rows_min: 4,
            grid_rows_max: 4,
            ..ObserveParams::default()
        }
    }

    #[test]
    fn warmup_calibrates_but_requires_fresh_confirmation_after_unlock() {
        let base = flat(8, 8, 40);
        let changed = flat(8, 8, 100);
        let mut det = SettleDetector::new(params(), base);
        for at in (0..4000).step_by(600) {
            let out = det.on_sample(&changed, at, 4000);
            assert!(!out.triggered);
            assert_eq!(out.metrics.state, "calibrating");
        }
        for at in [4000, 4600, 5200, 5799] {
            let out = det.on_sample(&changed, at, 4000);
            assert!(!out.triggered);
            assert!(out.metrics.calibrated);
        }
        assert!(det.on_sample(&changed, 5800, 4000).triggered);
        assert!(!det.on_sample(&changed, 6400, 4000).triggered);
        assert_eq!(det.changed_at_sample(), Some(0));
    }

    #[test]
    fn nothing_changes() {
        let base = flat(8, 8, 40);
        let mut det = SettleDetector::new(params(), base.clone());
        for at in (0..12000).step_by(600) {
            let out = det.on_sample(&base, at, 4000);
            assert!(!out.triggered);
            assert_eq!(out.metrics.changed_fraction, 0.0);
        }
    }

    #[test]
    fn game_opponent_motion_then_idle_settles_within_two_seconds() {
        let base = flat(8, 8, 40);
        let mut a = base.clone();
        paint_rect(&mut a, 0, 0, 8, 4, 200);
        let mut b = base.clone();
        paint_rect(&mut b, 0, 4, 8, 8, 200);
        let mut det = SettleDetector::new(params(), base);
        for i in 0..10 {
            let sample = if i % 2 == 0 { &a } else { &b };
            let out = det.on_sample(sample, i * 600, 4000);
            assert!(!out.triggered);
        }
        assert!(!det.on_sample(&b, 6000, 4000).triggered);
        assert!(!det.on_sample(&b, 6600, 4000).triggered);
        assert!(det.on_sample(&b, 7200, 4000).triggered);
    }

    #[test]
    fn flash_filtered() {
        let p = params();
        let base = flat(8, 8, 40);
        let mut flash = base.clone();
        paint_rect(&mut flash, 0, 0, 8, 8, 220);
        let mut det = SettleDetector::new(p, base.clone());
        let _ = det.on_sample(&base, 0, 0);
        let _ = det.on_sample(&base, 600, 0);
        assert!(!det.on_sample(&flash, 1200, 0).triggered);
        for at in (1800..6000).step_by(600) {
            let out = det.on_sample(&base, at, 0);
            assert!(!out.triggered);
            assert_eq!(out.metrics.state, "armed");
        }
    }

    #[test]
    fn slow_drift_and_small_active_region_never_count_as_stable() {
        for local in [false, true] {
            let mut det = SettleDetector::new(ObserveParams::default(), flat(320, 180, 40));
            for i in 0..20 {
                let mut frame = flat(320, 180, 100);
                if local {
                    paint_rect(&mut frame, 20, 20, 34, 34, if i % 2 == 0 { 0 } else { 200 });
                } else {
                    frame.data.fill(100 + i as u8 * 2);
                }
                assert!(!det.on_sample(&frame, i * 600, 4000).triggered);
            }
        }
    }

    #[test]
    fn browser_incremental_load_and_nonconsecutive_stability_do_not_trigger() {
        let mut det = SettleDetector::new(ObserveParams::default(), flat(320, 180, 40));
        let mut frame = flat(320, 180, 100);
        for i in 0..12 {
            if i % 2 == 0 {
                paint_rect(&mut frame, 0, i as u32 * 10, 160, i as u32 * 10 + 8, 20);
            }
            assert!(!det.on_sample(&frame, i * 600, 0).triggered);
        }
        assert!(!det.on_sample(&frame, 7200, 0).triggered);
        assert!(det.on_sample(&frame, 7800, 0).triggered);
    }

    #[test]
    fn startup_animation_is_not_learned_as_noise_and_quietest_window_wins() {
        let mut det = SettleDetector::new(params(), flat(8, 8, 40));
        for i in 0..3 {
            assert!(!det.on_sample(&flat(8, 8, 100), i * 600, 4000).triggered);
        }
        for i in 3..10 {
            let out = det.on_sample(
                &flat(8, 8, if i % 2 == 0 { 100 } else { 180 }),
                i * 600,
                4000,
            );
            assert!(!out.triggered);
            assert!(out.metrics.calibrated);
            assert_eq!(out.metrics.noise_mean, 0.0);
        }
        for at in [6000, 6600] {
            assert!(!det.on_sample(&flat(8, 8, 180), at, 4000).triggered);
        }
        assert!(det.on_sample(&flat(8, 8, 180), 7200, 4000).triggered);
    }

    #[test]
    fn noisy_calibration_is_capped_and_low_noise_is_tolerated() {
        assert!(!Motion {
            mean: 5.0,
            peak: 20.0,
            fraction: 0.1
        }
        .within(
            Motion {
                mean: 50.0,
                peak: 100.0,
                fraction: 1.0
            }
            .limits()
        ));
        let mut det = SettleDetector::new(params(), flat(8, 8, 40));
        for i in 0..7 {
            assert!(
                !det.on_sample(&flat(8, 8, 100 + (i % 2) as u8), i * 600, 4000)
                    .triggered
            );
        }
        for (i, at) in [4000, 4600, 5200, 5800].into_iter().enumerate() {
            assert_eq!(
                det.on_sample(&flat(8, 8, 100 + (i % 2) as u8), at, 4000)
                    .triggered,
                i == 3
            );
        }
    }

    #[test]
    fn calibrated_local_activity_has_independent_hard_limits() {
        let quiet = Motion::default().limits();
        assert_eq!(quiet.mean, 0.4);
        assert_eq!(quiet.peak, 2.0);
        assert_eq!(quiet.fraction, 0.001);

        // Rounded metrics from a post-action window in the Mahjong Soul test log.
        let noise = Motion {
            mean: 0.602066,
            peak: 35.48901,
            fraction: 0.01159722,
        };
        let settled = Motion {
            mean: 0.811059,
            peak: 53.04945,
            fraction: 0.01317708,
        };
        assert!(settled.within(noise.limits()));
        assert!(!settled.within(quiet));

        let noisy = Motion {
            mean: 50.0,
            peak: 200.0,
            fraction: 1.0,
        };
        let limits = noisy.limits();
        assert_eq!(limits.mean, 1.5);
        assert_eq!(limits.peak, 60.0);
        assert_eq!(limits.fraction, 0.02);
        assert!(settled.within(limits));
        for moving in [
            Motion {
                mean: 1.51,
                ..settled
            },
            Motion {
                peak: 60.1,
                ..settled
            },
            Motion {
                fraction: 0.0201,
                ..settled
            },
        ] {
            assert!(!moving.within(limits));
        }
        let adapted = noisy.adapt(settled);
        assert!(adapted.mean <= 1.5);
        assert!(adapted.peak <= 60.0);
        assert!(adapted.fraction <= 0.02);
    }

    #[test]
    fn opponent_motion_then_calibrated_local_activity_can_settle() {
        let mut det = SettleDetector::new(ObserveParams::default(), flat(320, 180, 40));
        let mut triggered_at = None;
        for i in 0..24 {
            let at = i * 300;
            let mut frame = flat(320, 180, 100);
            // A small recurring effect remains after the large opponent animation.
            paint_rect(&mut frame, 0, 0, 32, 18, 100 + (i % 2) as u8 * 40);
            if at < 5100 {
                paint_rect(&mut frame, 80, 40, 240, 140, 140 + (i % 2) as u8 * 60);
            }
            let out = det.on_sample(&frame, at, 4000);
            if at < 6900 {
                assert!(!out.triggered, "premature wake at {at}ms");
            } else {
                assert!(out.triggered);
                assert!(out.metrics.window_peak > 5.0);
                assert!(out.metrics.motion_fraction > 0.003);
                assert!(out.metrics.stable_for_ms >= 1800);
                triggered_at = Some(at);
            }
        }
        assert_eq!(triggered_at, Some(6900));
    }

    #[test]
    fn new_local_motion_after_quiet_calibration_still_blocks_wake() {
        let mut det = SettleDetector::new(ObserveParams::default(), flat(320, 180, 40));
        for at in (0..4000).step_by(600) {
            assert!(!det.on_sample(&flat(320, 180, 100), at, 4000).triggered);
        }
        for i in 0..20 {
            let mut frame = flat(320, 180, 100);
            paint_rect(&mut frame, 0, 0, 32, 18, 100 + (i % 2) as u8 * 40);
            let out = det.on_sample(&frame, 4000 + i * 600, 4000);
            assert!(!out.triggered);
            assert!(!out.metrics.noise_updated);
            assert_eq!(out.metrics.peak_limit, 2.0);
            assert_eq!(out.metrics.fraction_limit, 0.001);
        }
    }

    #[test]
    fn missing_samples_do_not_count_as_stable_time() {
        let frame = flat(8, 8, 100);
        let mut det = SettleDetector::new(params(), flat(8, 8, 40));
        for at in [0, 600, 1200, 5000, 5600, 6200] {
            assert!(!det.on_sample(&frame, at, 0).triggered);
        }
        assert!(det.on_sample(&frame, 6800, 0).triggered);
    }

    #[test]
    fn sampling_speed_does_not_shorten_confirmation_or_extend_short_deadlines() {
        for step in [150, 250, 600] {
            let mut det = SettleDetector::new(params(), flat(8, 8, 40));
            let frame = flat(8, 8, 100);
            for at in (0..4000).step_by(step) {
                assert!(!det.on_sample(&frame, at, 4000).triggered);
            }
            for offset in (0..1800).step_by(step) {
                assert!(!det.on_sample(&frame, 4000 + offset, 4000).triggered);
            }
            assert!(det.on_sample(&frame, 5800, 4000).triggered);
        }
        let mut det = SettleDetector::new(params(), flat(8, 8, 40));
        for at in (0..2000).step_by(600) {
            assert!(!det.on_sample(&flat(8, 8, 100), at, 4000).triggered);
        }
    }

    #[test]
    fn absent_or_short_calibration_uses_conservative_defaults() {
        for unlock in [0, 300] {
            let mut det = SettleDetector::new(params(), flat(8, 8, 40));
            if unlock > 0 {
                assert!(!det.on_sample(&flat(8, 8, 100), 0, unlock).triggered);
            }
            for offset in [0, 600, 1200, 1800] {
                let out = det.on_sample(&flat(8, 8, 100), unlock + offset, unlock);
                assert_eq!(out.metrics.calibrated, offset == 1800);
                assert_eq!(out.triggered, offset == 1800);
            }
        }
    }

    #[test]
    fn return_to_original_scene_does_not_reuse_an_old_change() {
        let base = flat(8, 8, 40);
        let mut det = SettleDetector::new(params(), base.clone());
        for at in [0, 600, 1200] {
            assert!(!det.on_sample(&flat(8, 8, 100), at, 0).triggered);
        }
        for at in (1800..6000).step_by(600) {
            assert!(!det.on_sample(&base, at, 0).triggered);
            assert!(det.changed_at_sample().is_none());
        }
    }

    #[test]
    fn noise_reference_keeps_evaluating_after_unlock_and_adjusts_slowly() {
        let base = flat(8, 8, 100);
        let mut det = SettleDetector::new(params(), base.clone());
        for at in (0..4000).step_by(600) {
            det.on_sample(&base, at, 4000);
        }
        assert_eq!(det.noise.mean, 0.0);
        let mut previous_update = 3600;
        let mut updates = 0;
        for i in 0..16 {
            let at = 4000 + i * 600;
            let mut frame = base.clone();
            frame.data[0] += (i % 2) as u8 * 2;
            let old = det.noise;
            let out = det.on_sample(&frame, at, 4000);
            assert!(!out.triggered);
            if out.metrics.noise_updated {
                assert!(at - previous_update >= 1800);
                previous_update = at;
                updates += 1;
                assert!(
                    (det.noise.mean - (old.mean * 0.9 + out.metrics.window_mean * 0.1)).abs()
                        < 0.00001
                );
            }
        }
        assert!(updates >= 3);
        assert!(det.noise.mean > 0.0 && det.noise.mean < 2.0 / 64.0);
        let old = det.noise.mean;
        for at in (13600..19600).step_by(600) {
            det.on_sample(&base, at, 4000);
        }
        assert!(det.noise.mean < old);
    }

    #[test]
    fn later_motion_cannot_train_itself_into_stability() {
        let mut det = SettleDetector::new(params(), flat(8, 8, 40));
        for at in (0..4000).step_by(600) {
            det.on_sample(&flat(8, 8, 100), at, 4000);
        }
        for i in 0..100 {
            let out = det.on_sample(&flat(8, 8, 100 + (i % 2) as u8 * 10), 4000 + i * 600, 4000);
            assert!(!out.triggered);
            assert!(!out.metrics.noise_updated);
            assert_eq!(det.noise.mean, 0.0);
        }
    }

    #[test]
    fn quietest_reference_can_improve_after_noisy_warmup() {
        let base = flat(8, 8, 100);
        let mut det = SettleDetector::new(params(), base.clone());
        for i in 0..7 {
            det.on_sample(&flat(8, 8, 100 + (i % 2) as u8), i * 600, 4000);
        }
        assert_eq!(det.noise.mean, 1.0);
        for at in [4000, 4600, 5200, 5800] {
            assert!(!det.on_sample(&base, at, 4000).triggered);
        }
        assert_eq!(det.quietest.unwrap().mean, 0.0);
        assert!((det.noise.mean - 0.9).abs() < 0.00001);
    }

    #[test]
    fn rebaselines_on_dimension_mismatch() {
        let base = flat(8, 8, 40);
        let mut det = SettleDetector::new(params(), base.clone());
        let _ = det.on_sample(&base, 0, 0);
        let other = flat(4, 4, 10);
        let out = det.on_sample(&other, 600, 0);
        assert_eq!(out.metrics.state, "rebaselined");
        assert!(!out.triggered);
        assert!(det.changed_at_sample().is_none());
        let next = det.on_sample(&other, 1200, 0);
        assert_eq!(next.metrics.state, "armed");
        assert_eq!(next.metrics.changed_fraction, 0.0);
    }

    #[test]
    fn downscale_gray_means_aspect_and_no_upscale() {
        let mut img = RgbaImage::new(2, 2);
        img.put_pixel(0, 0, Rgba([0, 0, 0, 255]));
        img.put_pixel(1, 0, Rgba([255, 0, 0, 0]));
        img.put_pixel(0, 1, Rgba([0, 255, 0, 128]));
        img.put_pixel(1, 1, Rgba([0, 0, 255, 255]));
        let small = downscale_gray(&img, 160);
        assert_eq!((small.width, small.height), (2, 2));
        assert_eq!(small.data[0], 0);
        assert_eq!(small.data[1], (255u32 * 299 / 1000) as u8);
        assert_eq!(small.data[2], (255u32 * 587 / 1000) as u8);
        assert_eq!(small.data[3], (255u16 * 114 / 1000) as u8);

        let mut wide = RgbaImage::new(4, 2);
        for x in 0..4 {
            wide.put_pixel(x, 0, Rgba([100, 0, 0, 255]));
            wide.put_pixel(x, 1, Rgba([0, 100, 0, 255]));
        }
        let scaled = downscale_gray(&wide, 2);
        assert_eq!((scaled.width, scaled.height), (2, 1));
        let expected = ((100u32 * 299 / 1000) + (100u32 * 587 / 1000)) / 2;
        assert_eq!(scaled.data[0] as u32, expected);
        assert_eq!(scaled.data[1] as u32, expected);

        let mut tall = RgbaImage::new(1, 5);
        for y in 0..5 {
            tall.put_pixel(0, y, Rgba([0, 0, 200, 255]));
        }
        let scaled = downscale_gray(&tall, 2);
        assert_eq!(scaled.width, 1);
        assert_eq!(scaled.height, 2);
        assert!(scaled
            .data
            .iter()
            .all(|&v| v == (200u16 * 114 / 1000) as u8));
    }

    #[test]
    fn top_decile_and_global_shift() {
        let base = flat(10, 1, 100);
        let mut thumb = base.clone();
        for x in 0..10 {
            thumb.data[x] = 100 + x as u8;
        }
        let mut p = params();
        p.grid_cols = 10;
        p.grid_rows_min = 1;
        p.grid_rows_max = 1;
        let mut det = SettleDetector::new(p, base);
        let out = det.on_sample(&thumb, 0, 4000);
        assert_eq!(out.metrics.top_decile_mean, 9.0);
        assert_eq!(out.metrics.global_mean_shift, 4.5);
    }

    #[test]
    fn defaults_match_spec() {
        let p = ObserveParams::default();
        assert_eq!(p.sample_interval_ms, 150);
        assert_eq!(p.slow_interval_ms, 250);
        assert_eq!(p.slowest_interval_ms, 600);
        assert_eq!(p.sample_cost_threshold_ms, 40);
        assert_eq!(p.long_edge, 320);
        assert_eq!(p.grid_cols, 24);
        assert_eq!(p.grid_rows_min, 8);
        assert_eq!(p.grid_rows_max, 24);
        assert_eq!(p.change_threshold, 10);
        assert_eq!(p.change_area_min, 0.03);
        assert_eq!(p.persistence_samples, 2);
        assert_eq!(p.calibration_window_ms, 1200);
        assert_eq!(p.settle_duration_ms, 1800);
        assert_eq!(p.max_sample_gap_ms, 900);
    }
}
