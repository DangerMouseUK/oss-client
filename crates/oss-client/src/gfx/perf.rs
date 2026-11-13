//! Optional per-frame timing, switched on by the `OSS_PERF` environment variable.
//!
//! Frame rate alone cannot say *where* a frame goes, and on a software adapter the
//! answer is rarely where it would be on a GPU. This records how long each stage of
//! [`super::Renderer::render`] took, and how much the frame asked of the renderer
//! (vertices, draw calls, and the pixels its quads cover), then reports a
//! distribution rather than a mean: a mean hides the frame that stalled.
//!
//! Off by default and cheap when off: one boolean test per stage, no clock reads.
//!
//! Any value but `0` or empty switches it on. A value that parses as a number is
//! *also* a reporting interval in seconds, so `OSS_PERF=1` reports every second
//! and `OSS_PERF=5` every five; a value that does not, such as `OSS_PERF=on`,
//! reports only once, at exit. The summary is cumulative either way, so a
//! periodic run's last report and an `on` run's only report say the same thing.

use std::time::{Duration, Instant};

/// A stage of a frame, in the order a frame passes through them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The game building its vertex list on the CPU (outside the renderer).
    Draw,
    /// Uploading egui's texture deltas.
    UiTex,
    /// Waiting for room under the in-flight bound.
    Wait,
    /// `get_current_texture`: waiting for the swapchain to free a buffer, which is
    /// where a renderer that cannot keep up shows itself on a presenting surface.
    Acquire,
    /// Writing the uniform and vertex buffers.
    Write,
    /// Recording the render passes.
    Encode,
    /// `queue.submit`.
    Submit,
    /// `queue.present`.
    Present,
    /// Everything above, plus whatever else the frame did.
    Frame,
}

impl Stage {
    const ALL: [Stage; 9] = [
        Stage::Draw,
        Stage::UiTex,
        Stage::Wait,
        Stage::Acquire,
        Stage::Write,
        Stage::Encode,
        Stage::Submit,
        Stage::Present,
        Stage::Frame,
    ];

    fn index(self) -> usize {
        Stage::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    fn name(self) -> &'static str {
        match self {
            Stage::Draw => "draw",
            Stage::UiTex => "ui-tex",
            Stage::Wait => "wait",
            Stage::Acquire => "acquire",
            Stage::Write => "write",
            Stage::Encode => "encode",
            Stage::Submit => "submit",
            Stage::Present => "present",
            Stage::Frame => "FRAME",
        }
    }
}

/// What a frame asked of the renderer.
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameWork {
    /// Vertices written to the vertex buffer.
    pub vertices: u32,
    /// `draw` calls issued for sprites.
    pub draws: u32,
    /// Pixels in the surface, for the fill-rate question.
    pub pixels: u32,
    /// Pixels the frame's quads cover, counting overdraw: the fill the sprite
    /// pass actually asks for, which is the other half of that question.
    pub quad_px: u64,
    /// Whether the starfield pass ran.
    pub stars: bool,
}

/// Microsecond samples for one stage.
#[derive(Default)]
struct Samples {
    us: Vec<u32>,
}

impl Samples {
    /// Enough for half an hour at 60 Hz, or ten minutes at 200.
    const CAP: usize = 128 * 1024;

    fn push(&mut self, us: u32) {
        if self.us.len() < Self::CAP {
            self.us.push(us);
        }
    }

    /// count, min, median, p90, p99, max, mean — all in microseconds.
    ///
    /// Percentiles are the nearest rank at or below the asked-for fraction, so
    /// with few samples several of them land on the same one; the maximum is
    /// reported separately and is the one to read for the worst frame.
    fn summary(&self) -> Option<(usize, u32, u32, u32, u32, u32, u32)> {
        if self.us.is_empty() {
            return None;
        }
        let mut v = self.us.clone();
        v.sort_unstable();
        let at = |f: f64| v[((v.len() - 1) as f64 * f) as usize];
        let sum: u64 = v.iter().map(|&x| u64::from(x)).sum();
        Some((
            v.len(),
            v[0],
            at(0.5),
            at(0.9),
            at(0.99),
            v[v.len() - 1],
            (sum / v.len() as u64) as u32,
        ))
    }
}

/// Records stage timings for the frames of one run.
pub struct Perf {
    enabled: bool,
    stages: [Samples; 9],
    /// Set at the start of each stage by [`Perf::begin`].
    mark: Instant,
    /// When the run started, for the overall frame rate.
    started: Instant,
    /// When the last periodic report went out.
    last_report: Instant,
    /// How often to report while running, if at all.
    interval: Option<Duration>,
    frames: u64,
    skipped: u64,
    work: FrameWork,
    /// Totals over the run, for per-frame averages of the work counters.
    total_vertices: u64,
    total_draws: u64,
    total_quad_px: u64,
}

impl Perf {
    /// Reads `OSS_PERF`: absent or `0` disables everything here.
    pub fn from_env() -> Self {
        let (enabled, interval) = Self::settings(&std::env::var("OSS_PERF").unwrap_or_default());
        let now = Instant::now();
        Self {
            enabled,
            stages: Default::default(),
            mark: now,
            started: now,
            last_report: now,
            interval,
            frames: 0,
            skipped: 0,
            work: FrameWork::default(),
            total_vertices: 0,
            total_draws: 0,
            total_quad_px: 0,
        }
    }

    /// What a value of `OSS_PERF` means: whether to record, and how often to
    /// report while running.
    ///
    /// Kept apart from [`Perf::from_env`] and free of the environment so the
    /// module's own description of the variable can be checked by a test rather
    /// than trusted -- the two had already drifted once, the documentation
    /// promising that `1` reported only at exit when it asks for a report every
    /// second.
    fn settings(var: &str) -> (bool, Option<Duration>) {
        let enabled = !var.is_empty() && var != "0";
        let interval = var
            .parse::<f64>()
            .ok()
            .filter(|s| *s > 0.0)
            .map(Duration::from_secs_f64);
        (enabled, interval)
    }

    /// Whether anything here is recording. Callers can skip gathering counters.
    pub fn on(&self) -> bool {
        self.enabled
    }

    /// Starts the clock for the next stage.
    pub fn begin(&mut self) {
        if self.enabled {
            self.mark = Instant::now();
        }
    }

    /// Ends a stage begun by [`Perf::begin`] and starts the next one, so stages
    /// can be timed back to back with one clock read between them.
    pub fn end(&mut self, stage: Stage) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        let us = now.saturating_duration_since(self.mark).as_micros();
        self.stages[stage.index()].push(us.min(u128::from(u32::MAX)) as u32);
        self.mark = now;
    }

    /// Records a stage whose duration was measured elsewhere.
    pub fn record(&mut self, stage: Stage, took: Duration) {
        if self.enabled {
            let us = took.as_micros().min(u128::from(u32::MAX)) as u32;
            self.stages[stage.index()].push(us);
        }
    }

    /// What this frame asked of the renderer.
    pub fn work(&mut self, work: FrameWork) {
        if self.enabled {
            self.work = work;
            self.total_vertices += u64::from(work.vertices);
            self.total_draws += u64::from(work.draws);
            self.total_quad_px += work.quad_px;
        }
    }

    /// Counts a frame the renderer declined to build.
    pub fn skipped(&mut self) {
        if self.enabled {
            self.skipped += 1;
        }
    }

    /// Ends a frame: counts it and reports if the interval has come round.
    pub fn frame_done(&mut self, took: Duration) {
        if !self.enabled {
            return;
        }
        self.frames += 1;
        self.record(Stage::Frame, took);
        if let Some(interval) = self.interval
            && self.last_report.elapsed() >= interval
        {
            self.last_report = Instant::now();
            self.report("running");
        }
    }

    /// Writes the distribution to the log, at info level so a plain run shows it.
    pub fn report(&self, label: &str) {
        if !self.enabled {
            return;
        }
        let secs = self.started.elapsed().as_secs_f64().max(1e-9);
        log::info!(
            "perf [{label}] {} frames in {secs:.2}s = {:.1} fps ({} skipped); \
             {}x{} px, {:.0} vertices, {:.0} draws and {:.0} quad px a frame, stars {}",
            self.frames,
            self.frames as f64 / secs,
            self.skipped,
            self.work.pixels_w(),
            self.work.pixels_h(),
            self.total_vertices as f64 / self.frames.max(1) as f64,
            self.total_draws as f64 / self.frames.max(1) as f64,
            self.total_quad_px as f64 / self.frames.max(1) as f64,
            if self.work.stars { "on" } else { "off" },
        );
        log::info!(
            "perf [{label}] {:>8} {:>7} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
            "stage",
            "n",
            "min",
            "median",
            "p90",
            "p99",
            "max",
            "mean"
        );
        for stage in Stage::ALL {
            let Some((n, min, med, p90, p99, max, mean)) = self.stages[stage.index()].summary()
            else {
                continue;
            };
            log::info!(
                "perf [{label}] {:>8} {n:>7} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
                stage.name(),
                us(min),
                us(med),
                us(p90),
                us(p99),
                us(max),
                us(mean),
            );
        }
    }
}

impl Drop for Perf {
    /// The run's own summary, however it ends.
    fn drop(&mut self) {
        if self.enabled && self.frames > 0 {
            self.report("final");
        }
    }
}

impl FrameWork {
    fn pixels_w(&self) -> u32 {
        self.pixels >> 16
    }

    fn pixels_h(&self) -> u32 {
        self.pixels & 0xffff
    }

    /// Packs a surface size into [`FrameWork::pixels`].
    pub fn size(width: u32, height: u32) -> u32 {
        (width << 16) | (height & 0xffff)
    }
}

/// Microseconds as milliseconds to two places, which is the scale a frame lives at.
fn us(v: u32) -> String {
    format!("{:.2}ms", f64::from(v) / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The module documentation describes this variable to whoever uses the
    /// harness, and it had already described it wrongly. These are the cases it
    /// names.
    #[test]
    fn oss_perf_means_what_the_module_says_it_means() {
        let secs = |d: Option<Duration>| d.map(|d| d.as_secs_f64());

        let (on, every) = Perf::settings("");
        assert!(!on, "unset is off");
        assert_eq!(secs(every), None);

        let (on, every) = Perf::settings("0");
        assert!(!on, "zero is off, not a zero-second interval");
        assert_eq!(secs(every), None);

        let (on, every) = Perf::settings("1");
        assert!(on);
        assert_eq!(secs(every), Some(1.0), "a number is an interval in seconds");

        let (on, every) = Perf::settings("5");
        assert!(on);
        assert_eq!(secs(every), Some(5.0));

        let (on, every) = Perf::settings("on");
        assert!(on, "a non-numeric value still switches it on");
        assert_eq!(
            secs(every),
            None,
            "and asks for no periodic report, so the only one comes at exit"
        );

        let (on, every) = Perf::settings("-2");
        assert!(on);
        assert_eq!(secs(every), None, "a negative interval is no interval");
    }

    #[test]
    fn a_disabled_probe_records_nothing() {
        let mut p = Perf::from_env();
        p.enabled = false;
        p.begin();
        p.end(Stage::Draw);
        p.record(Stage::Frame, Duration::from_millis(5));
        assert!(
            p.stages[Stage::Draw.index()].summary().is_none(),
            "an off probe must not pay for samples"
        );
        assert!(p.stages[Stage::Frame.index()].summary().is_none());
    }

    #[test]
    fn percentiles_come_from_the_sorted_samples() {
        let mut s = Samples::default();
        // Out of order on purpose: the summary sorts.
        for v in [600u32, 100, 900, 300, 1000, 200, 800, 400, 700, 500] {
            s.push(v);
        }
        let (n, min, med, p90, p99, max, mean) = s.summary().expect("ten samples");
        assert_eq!((n, min, max, mean), (10, 100, 1000, 550));
        assert_eq!(med, 500, "median of 100..1000 by nearest rank below");
        assert_eq!(
            (p90, p99),
            (900, 900),
            "both tails land on the ninth of ten samples; the max is reported apart from them"
        );
    }

    #[test]
    fn samples_stop_at_the_cap_rather_than_growing_without_bound() {
        let mut s = Samples::default();
        for _ in 0..Samples::CAP + 1000 {
            s.push(1);
        }
        assert_eq!(s.us.len(), Samples::CAP, "a long run must not eat memory");
    }

    #[test]
    fn a_surface_size_survives_the_round_trip() {
        let w = FrameWork {
            pixels: FrameWork::size(1280, 800),
            ..Default::default()
        };
        assert_eq!((w.pixels_w(), w.pixels_h()), (1280, 800));
    }
}
