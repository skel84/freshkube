//! Telling steady growth from noise in a long run's samples. A soak run
//! (`scripts/soak.sh`) samples the app's memory, threads and open handles
//! every so often; [`check`] says which of them kept growing.
//!
//! The run after its warm-up is cut into three equal parts and each part
//! reduced to its median. A series grows steadily when each step between
//! neighbouring medians is at least half of what the series must gain, and
//! the whole gain passes both an absolute and a relative limit. So a leak
//! (every part higher than the last) fails, while noise, a sawtooth that
//! returns to its baseline and a single step up (the cache that fills once)
//! pass. A step late in the run is not told from the start of a leak: run
//! longer when in doubt.

/// One sample of the process.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub second: u64,
    pub rss_mb: f64,
    pub threads: f64,
    pub fds: f64,
}

/// What a series may gain before it counts as growth: it must pass both.
#[derive(Clone, Copy, Debug)]
pub struct Limit {
    pub absolute: f64,
    /// A share of the first part's median, 0.1 for 10%.
    pub relative: f64,
}

/// Limits for memory in MB, threads and handles.
pub const RSS: Limit = Limit {
    absolute: 20.,
    relative: 0.10,
};
pub const THREADS: Limit = Limit {
    absolute: 8.,
    relative: 0.10,
};
pub const FDS: Limit = Limit {
    absolute: 16.,
    relative: 0.10,
};

/// Fewest samples after the warm-up that [`check`] will judge.
pub const MIN_SAMPLES: usize = 30;

/// A series that kept growing.
#[derive(Clone, Debug, PartialEq)]
pub struct Growth {
    pub name: &'static str,
    /// The medians of the three parts.
    pub parts: [f64; 3],
}

impl std::fmt::Display for Growth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [a, b, c] = self.parts;
        write!(
            f,
            "{} grew steadily: {a:.1}, then {b:.1}, then {c:.1}",
            self.name
        )
    }
}

/// Why a run can't be judged.
#[derive(Clone, Debug, PartialEq)]
pub enum Unjudged {
    TooFew { have: usize },
}

impl std::fmt::Display for Unjudged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Unjudged::TooFew { have } = self;
        write!(
            f,
            "{have} samples after the warm-up; at least {MIN_SAMPLES} are needed to judge a run"
        )
    }
}

/// The series that grew, from `samples` in time order, leaving out those in
/// the first `warmup` seconds.
pub fn check(samples: &[Sample], warmup: u64) -> Result<Vec<Growth>, Unjudged> {
    let kept: Vec<&Sample> = samples.iter().filter(|s| s.second > warmup).collect();
    if kept.len() < MIN_SAMPLES {
        return Err(Unjudged::TooFew { have: kept.len() });
    }
    let series: [(&'static str, Limit, fn(&Sample) -> f64); 3] = [
        ("memory (MB)", RSS, |s| s.rss_mb),
        ("threads", THREADS, |s| s.threads),
        ("open handles", FDS, |s| s.fds),
    ];
    Ok(series
        .into_iter()
        .filter_map(|(name, limit, value)| {
            let values: Vec<f64> = kept.iter().map(|s| value(s)).collect();
            grows(&values, limit).map(|parts| Growth { name, parts })
        })
        .collect())
}

fn grows(values: &[f64], limit: Limit) -> Option<[f64; 3]> {
    let third = values.len() / 3;
    // The remainder goes to the last part, so the run's end is always seen.
    let parts = [
        median(&values[..third]),
        median(&values[third..2 * third]),
        median(&values[2 * third..]),
    ];
    let gain = parts[2] - parts[0];
    let needed = limit.absolute.max(limit.relative * parts[0].abs());
    let steady = parts[1] - parts[0] >= needed / 2. && parts[2] - parts[1] >= needed / 2.;
    (gain >= needed && steady).then_some(parts)
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.
    } else {
        sorted[mid]
    }
}

/// Reads the CSV `scripts/soak.sh` writes: `second,rss_mb,threads,fds`, with
/// a header line. A line that doesn't parse is an error naming its number.
pub fn parse_csv(text: &str) -> Result<Vec<Sample>, String> {
    let mut samples = Vec::new();
    for (ix, line) in text.lines().enumerate().skip(1) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        let bad = || format!("line {}: expected second,rss_mb,threads,fds", ix + 1);
        let [second, rss, threads, fds] = fields[..] else {
            return Err(bad());
        };
        samples.push(Sample {
            second: second.trim().parse().map_err(|_| bad())?,
            rss_mb: rss.trim().parse().map_err(|_| bad())?,
            threads: threads.trim().parse().map_err(|_| bad())?,
            fds: fds.trim().parse().map_err(|_| bad())?,
        });
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 90 samples a minute apart, after a 600 s warm-up that the shape of
    /// `f` never sees, with `f` giving each sample's value from its index.
    fn series(f: impl Fn(usize) -> f64) -> Vec<Sample> {
        (0..100)
            .map(|ix| {
                let at = ix as u64 * 60;
                let value = if at <= 600 { 1000. } else { f(ix - 10) };
                Sample {
                    second: at,
                    rss_mb: value,
                    threads: 40.,
                    fds: 60.,
                }
            })
            .collect()
    }

    fn grew(samples: &[Sample]) -> Vec<&'static str> {
        check(samples, 600)
            .expect("enough samples")
            .iter()
            .map(|g| g.name)
            .collect()
    }

    /// A tiny deterministic wobble, so no series is a clean line.
    fn noise(ix: usize) -> f64 {
        ((ix * 7919) % 13) as f64 - 6.
    }

    #[test]
    fn a_flat_series_with_noise_passes() {
        assert!(grew(&series(|ix| 300. + noise(ix))).is_empty());
    }

    #[test]
    fn a_steady_leak_fails_even_under_noise() {
        let samples = series(|ix| 300. + ix as f64 * 2. + noise(ix));
        assert_eq!(grew(&samples), ["memory (MB)"]);
    }

    #[test]
    fn a_single_step_up_early_passes() {
        // The cache that fills once, then holds.
        assert!(grew(&series(|ix| if ix < 20 { 300. } else { 400. } + noise(ix))).is_empty());
    }

    #[test]
    fn a_sawtooth_that_returns_to_its_baseline_passes() {
        assert!(grew(&series(|ix| 300. + (ix % 20) as f64 * 5.)).is_empty());
    }

    #[test]
    fn growth_under_the_absolute_limit_passes() {
        // 5 MB over the run on a 300 MB process.
        assert!(grew(&series(|ix| 300. + ix as f64 / 18.)).is_empty());
    }

    #[test]
    fn growth_under_the_relative_limit_passes() {
        // 30 MB over the run, but on a 1 GB process.
        assert!(grew(&series(|ix| 1000. + ix as f64 / 3.)).is_empty());
    }

    #[test]
    fn threads_and_handles_are_judged_too() {
        let samples: Vec<Sample> = series(|_| 300.)
            .into_iter()
            .enumerate()
            .map(|(ix, s)| Sample {
                threads: 40. + ix as f64,
                fds: 60. + ix as f64 * 2.,
                ..s
            })
            .collect();
        assert_eq!(grew(&samples), ["threads", "open handles"]);
    }

    #[test]
    fn a_run_too_short_to_judge_says_so() {
        let few = &series(|ix| ix as f64 * 10.)[..30];
        assert_eq!(check(few, 600), Err(Unjudged::TooFew { have: 19 }));
    }

    #[test]
    fn the_warm_up_is_left_out() {
        // Memory climbs during the warm-up only.
        let samples: Vec<Sample> = (0..100)
            .map(|ix| Sample {
                second: ix * 60,
                rss_mb: if ix <= 10 {
                    100. + ix as f64 * 50.
                } else {
                    600.
                },
                threads: 40.,
                fds: 60.,
            })
            .collect();
        assert!(grew(&samples).is_empty());
    }

    #[test]
    fn the_csv_round_trips_and_a_bad_line_is_named() {
        let text = "second,rss_mb,threads,fds\n30,210.5,41,63\n\n60,211,41,63\n";
        let samples = parse_csv(text).unwrap();
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].rss_mb, 210.5);
        let err = parse_csv("second,rss_mb,threads,fds\n30,210,x,63\n").unwrap_err();
        assert!(err.starts_with("line 2"), "{err}");
    }
}
