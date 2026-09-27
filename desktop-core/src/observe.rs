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
    pub motion_threshold: u8,
    pub settle_enter_fraction: f32,
    pub settle_exit_fraction: f32,
    pub settle_samples: u32,
    pub min_age_samples: u32,
}

impl Default for ObserveParams {
    fn default() -> Self {
        Self {
            sample_interval_ms: 150,
            slow_interval_ms: 250,
            slowest_interval_ms: 400,
            sample_cost_threshold_ms: 40,
            long_edge: 160,
            grid_cols: 12,
            grid_rows_min: 4,
            grid_rows_max: 12,
            change_threshold: 10,
            change_area_min: 0.03,
            persistence_samples: 2,
            motion_threshold: 8,
            settle_enter_fraction: 0.02,
            settle_exit_fraction: 0.15,
            settle_samples: 3,
            min_age_samples: 2,
        }
    }
}

pub struct SampleMetrics {
    pub index: u32,
    pub changed_fraction: f32,
    pub motion_fraction: f32,
    pub top_decile_mean: f32,
    pub global_mean_shift: f32,
    pub state: &'static str,
}

pub struct SampleOutcome {
    pub metrics: SampleMetrics,
    pub triggered: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Armed,
    Settling,
    Triggered,
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
    previous: Option<GrayImage>,
    blocks: Vec<Block>,
    phase: Phase,
    index: u32,
    change_run: u32,
    change_run_start: u32,
    stability: u32,
    changed_at: Option<u32>,
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
            previous: None,
            blocks,
            phase: Phase::Armed,
            index: 0,
            change_run: 0,
            change_run_start: 0,
            stability: 0,
            changed_at: None,
        }
    }

    pub fn on_sample(&mut self, thumb: &GrayImage) -> SampleOutcome {
        let index = self.index;
        self.index += 1;

        if thumb.width != self.baseline.width || thumb.height != self.baseline.height {
            self.baseline = thumb.clone();
            self.previous = None;
            self.blocks = grid_blocks(
                thumb.width,
                thumb.height,
                self.params.grid_cols,
                self.params.grid_rows_min,
                self.params.grid_rows_max,
            );
            self.phase = Phase::Armed;
            self.change_run = 0;
            self.stability = 0;
            self.changed_at = None;
            return SampleOutcome {
                metrics: SampleMetrics {
                    index,
                    changed_fraction: 0.0,
                    motion_fraction: 0.0,
                    top_decile_mean: 0.0,
                    global_mean_shift: 0.0,
                    state: "rebaselined",
                },
                triggered: false,
            };
        }

        let stats = block_stats(
            thumb,
            &self.baseline,
            self.previous.as_ref(),
            &self.blocks,
            self.params.change_threshold,
            self.params.motion_threshold,
        );
        let n = self.blocks.len().max(1) as f32;
        let changed_fraction = stats.changed as f32 / n;
        let motion_fraction = if self.previous.is_none() {
            0.0
        } else {
            stats.moving as f32 / n
        };
        let top_decile_mean = top_decile(&stats.diffs);
        let global_mean_shift = stats.signed_sum as f32 / stats.pixels.max(1) as f32;

        self.previous = Some(thumb.clone());

        let mut triggered = false;
        if self.phase != Phase::Triggered && index >= self.params.min_age_samples {
            match self.phase {
                Phase::Armed => {
                    if changed_fraction >= self.params.change_area_min {
                        if self.change_run == 0 {
                            self.change_run_start = index;
                        }
                        self.change_run += 1;
                        if self.change_run >= self.params.persistence_samples {
                            self.phase = Phase::Settling;
                            self.changed_at = Some(self.change_run_start);
                            self.stability = 0;
                        }
                    } else {
                        self.change_run = 0;
                    }
                }
                Phase::Settling => {
                    if motion_fraction > self.params.settle_exit_fraction {
                        self.stability = 0;
                    } else if motion_fraction <= self.params.settle_enter_fraction {
                        self.stability += 1;
                        if self.stability >= self.params.settle_samples {
                            self.phase = Phase::Triggered;
                            triggered = true;
                        }
                    }
                }
                Phase::Triggered => {}
            }
        }

        SampleOutcome {
            metrics: SampleMetrics {
                index,
                changed_fraction,
                motion_fraction,
                top_decile_mean,
                global_mean_shift,
                state: match self.phase {
                    Phase::Armed => "armed",
                    Phase::Settling => "settling",
                    Phase::Triggered => "triggered",
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
    moving: u32,
    diffs: Vec<f32>,
    signed_sum: i64,
    pixels: u64,
}

fn block_stats(
    thumb: &GrayImage,
    baseline: &GrayImage,
    previous: Option<&GrayImage>,
    blocks: &[Block],
    change_threshold: u8,
    motion_threshold: u8,
) -> Stats {
    let w = thumb.width as usize;
    let mut changed = 0u32;
    let mut moving = 0u32;
    let mut diffs = Vec::with_capacity(blocks.len());
    let mut signed_sum: i64 = 0;
    let mut pixels: u64 = 0;
    for block in blocks {
        let mut abs_sum: u32 = 0;
        let mut prev_sum: u32 = 0;
        let mut count: u32 = 0;
        for y in block.y0..block.y1 {
            let row = y as usize * w;
            for x in block.x0..block.x1 {
                let i = row + x as usize;
                let t = thumb.data[i] as u16;
                let b = baseline.data[i] as u16;
                abs_sum += t.abs_diff(b) as u32;
                signed_sum += t as i64 - b as i64;
                if let Some(prev) = previous {
                    prev_sum += t.abs_diff(prev.data[i] as u16) as u32;
                }
                count += 1;
            }
        }
        pixels += count as u64;
        let mean = if count == 0 { 0 } else { abs_sum / count };
        diffs.push(mean as f32);
        if mean > change_threshold as u32 {
            changed += 1;
        }
        if previous.is_some() {
            let prev_mean = if count == 0 { 0 } else { prev_sum / count };
            if prev_mean > motion_threshold as u32 {
                moving += 1;
            }
        }
    }
    Stats {
        changed,
        moving,
        diffs,
        signed_sum,
        pixels,
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
            change_threshold: 10,
            change_area_min: 0.25,
            persistence_samples: 2,
            motion_threshold: 8,
            settle_enter_fraction: 0.02,
            settle_exit_fraction: 0.15,
            settle_samples: 3,
            min_age_samples: 2,
            ..ObserveParams::default()
        }
    }

    #[test]
    fn menu_appears() {
        let p = params();
        let base = flat(8, 8, 40);
        let mut det = SettleDetector::new(p, base.clone());
        for _ in 0..3 {
            let out = det.on_sample(&base);
            assert!(!out.triggered);
            assert_eq!(out.metrics.state, "armed");
        }
        let mut changed = base.clone();
        paint_rect(&mut changed, 0, 0, 4, 4, 200);
        let mut settling_at = None;
        for _ in 0..p.persistence_samples {
            let out = det.on_sample(&changed);
            assert!(!out.triggered);
            if out.metrics.state == "settling" {
                settling_at = Some(out.metrics.index);
            }
        }
        assert_eq!(settling_at, Some(p.min_age_samples + p.persistence_samples));
        assert_eq!(det.changed_at_sample(), Some(p.min_age_samples + 1));
        let mut triggered_at = None;
        for _ in 0..p.settle_samples {
            let out = det.on_sample(&changed);
            if out.triggered {
                triggered_at = Some(out.metrics.index);
            }
        }
        assert_eq!(
            triggered_at,
            Some(p.min_age_samples + p.persistence_samples + p.settle_samples)
        );
    }

    #[test]
    fn nothing_changes() {
        let base = flat(8, 8, 40);
        let mut det = SettleDetector::new(params(), base.clone());
        for _ in 0..12 {
            let out = det.on_sample(&base);
            assert!(!out.triggered);
            assert_eq!(out.metrics.state, "armed");
            assert_eq!(out.metrics.changed_fraction, 0.0);
        }
    }

    #[test]
    fn constant_motion() {
        let base = flat(8, 8, 40);
        let mut a = base.clone();
        paint_rect(&mut a, 0, 0, 8, 4, 200);
        let mut b = base.clone();
        paint_rect(&mut b, 0, 4, 8, 8, 200);
        let mut det = SettleDetector::new(params(), base);
        let mut saw_settling = false;
        for i in 0..10 {
            let sample = if i % 2 == 0 { &a } else { &b };
            let out = det.on_sample(sample);
            assert!(!out.triggered);
            if out.metrics.state == "settling" {
                saw_settling = true;
                assert!(out.metrics.motion_fraction > params().settle_exit_fraction);
            }
        }
        assert!(saw_settling);
        assert_eq!(det.on_sample(&a).metrics.state, "settling");
    }

    #[test]
    fn flash_filtered() {
        let p = params();
        let base = flat(8, 8, 40);
        let mut flash = base.clone();
        paint_rect(&mut flash, 0, 0, 8, 8, 220);
        let mut det = SettleDetector::new(p, base.clone());
        let _ = det.on_sample(&base);
        let _ = det.on_sample(&base);
        let out = det.on_sample(&flash);
        assert_eq!(out.metrics.state, "armed");
        for _ in 0..6 {
            let out = det.on_sample(&base);
            assert!(!out.triggered);
            assert_eq!(out.metrics.state, "armed");
        }
    }

    #[test]
    fn min_age_respected() {
        let p = params();
        let base = flat(8, 8, 40);
        let mut changed = base.clone();
        paint_rect(&mut changed, 0, 0, 8, 8, 220);
        let mut det = SettleDetector::new(p, base);
        for i in 0..p.min_age_samples {
            let out = det.on_sample(&changed);
            assert_eq!(out.metrics.index, i);
            assert_eq!(out.metrics.state, "armed");
            assert!(out.metrics.changed_fraction >= p.change_area_min);
        }
        let out = det.on_sample(&changed);
        assert_eq!(out.metrics.state, "armed");
        let out = det.on_sample(&changed);
        assert_eq!(out.metrics.state, "settling");
        assert_eq!(det.changed_at_sample(), Some(p.min_age_samples));
    }

    #[test]
    fn hysteresis_dead_zone() {
        let mut p = params();
        p.grid_cols = 10;
        p.grid_rows_min = 1;
        p.grid_rows_max = 1;
        p.change_area_min = 0.5;
        p.persistence_samples = 1;
        p.settle_samples = 5;
        p.min_age_samples = 0;
        p.motion_threshold = 8;
        p.settle_enter_fraction = 0.05;
        p.settle_exit_fraction = 0.25;
        let base = flat(10, 1, 0);
        let mut changed = base.clone();
        for x in 0..10 {
            changed.data[x] = 100;
        }
        let mut det = SettleDetector::new(p, base);
        let enter = det.on_sample(&changed);
        assert_eq!(enter.metrics.state, "settling");
        let mut dead = changed.clone();
        dead.data[0] = 0;
        let out = det.on_sample(&dead);
        assert!(out.metrics.motion_fraction > p.settle_enter_fraction);
        assert!(out.metrics.motion_fraction < p.settle_exit_fraction);
        assert_eq!(out.metrics.state, "settling");
        let still = det.on_sample(&dead);
        assert_eq!(still.metrics.motion_fraction, 0.0);
        assert_eq!(still.metrics.state, "settling");
        let _ = det.on_sample(&dead);
        let done = det.on_sample(&dead);
        assert!(!done.triggered);
        let _ = det.on_sample(&dead);
        let done = det.on_sample(&dead);
        assert!(done.triggered);
    }

    #[test]
    fn one_shot_terminal() {
        let mut p = params();
        p.min_age_samples = 0;
        p.persistence_samples = 1;
        p.settle_samples = 1;
        let base = flat(8, 8, 40);
        let mut changed = base.clone();
        paint_rect(&mut changed, 0, 0, 8, 8, 200);
        let mut det = SettleDetector::new(p, base);
        let first = det.on_sample(&changed);
        assert_eq!(first.metrics.state, "settling");
        let trig = det.on_sample(&changed);
        assert!(trig.triggered);
        assert_eq!(trig.metrics.state, "triggered");
        let later = det.on_sample(&changed);
        assert!(!later.triggered);
        assert_eq!(later.metrics.state, "triggered");
    }

    #[test]
    fn rebaselines_on_dimension_mismatch() {
        let base = flat(8, 8, 40);
        let mut det = SettleDetector::new(params(), base.clone());
        let _ = det.on_sample(&base);
        let other = flat(4, 4, 10);
        let out = det.on_sample(&other);
        assert_eq!(out.metrics.state, "rebaselined");
        assert!(!out.triggered);
        assert!(det.changed_at_sample().is_none());
        let next = det.on_sample(&other);
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
        p.min_age_samples = 99;
        let mut det = SettleDetector::new(p, base);
        let out = det.on_sample(&thumb);
        assert_eq!(out.metrics.top_decile_mean, 9.0);
        assert_eq!(out.metrics.global_mean_shift, 4.5);
    }

    #[test]
    fn defaults_match_spec() {
        let p = ObserveParams::default();
        assert_eq!(p.sample_interval_ms, 150);
        assert_eq!(p.slow_interval_ms, 250);
        assert_eq!(p.slowest_interval_ms, 400);
        assert_eq!(p.sample_cost_threshold_ms, 40);
        assert_eq!(p.long_edge, 160);
        assert_eq!(p.grid_cols, 12);
        assert_eq!(p.grid_rows_min, 4);
        assert_eq!(p.grid_rows_max, 12);
        assert_eq!(p.change_threshold, 10);
        assert_eq!(p.change_area_min, 0.03);
        assert_eq!(p.persistence_samples, 2);
        assert_eq!(p.motion_threshold, 8);
        assert_eq!(p.settle_enter_fraction, 0.02);
        assert_eq!(p.settle_exit_fraction, 0.15);
        assert_eq!(p.settle_samples, 3);
        assert_eq!(p.min_age_samples, 2);
    }
}
