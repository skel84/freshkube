//! The faults a soak run injects into the `stress` binary's synthetic API:
//! refused requests, dropped connections, slow answers and expired watches.
//! The plan is plain data and a seeded generator, so a run with the same
//! seed and the same requests meets the same faults.
//!
//! The knob is `FRESHKUBE_STRESS_FAULTS`, comma-separated `name=value`:
//!
//! | Name | Value | Fault |
//! | --- | --- | --- |
//! | `refuse` | chance 0–1 | the request is answered 503 Service Unavailable |
//! | `drop` | chance 0–1 | the connection closes with no answer |
//! | `slow` | `chance@milliseconds` | the answer waits that long |
//! | `gone` | seconds | a watch that starts after each interval is answered 410 Gone |
//! | `cut` | seconds | every watch that starts is ended by the server after this long, so the app opens the next one |
//! | `after` | seconds, default 20 | no fault before this, so the app can start |
//! | `seed` | integer, default 1 | the generator's seed |

use std::time::Duration;

/// What to do with one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    None,
    Refuse,
    Drop,
    Slow(Duration),
    Gone,
}

/// How many of each fault a run injected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub refused: u64,
    pub dropped: u64,
    pub slow: u64,
    pub gone: u64,
    pub cut: u64,
}

#[derive(Clone, Debug)]
pub struct FaultPlan {
    refuse: f64,
    drop: f64,
    slow: f64,
    slow_for: Duration,
    gone_every: Option<Duration>,
    cut_after: Option<Duration>,
    after: Duration,
    state: u64,
    next_gone: Option<Duration>,
    counts: Counts,
}

impl FaultPlan {
    /// Parses the knob's text; an unknown name, a chance outside 0–1 or a
    /// value that isn't a number is an error naming it.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut plan = Self {
            refuse: 0.,
            drop: 0.,
            slow: 0.,
            slow_for: Duration::ZERO,
            gone_every: None,
            cut_after: None,
            after: Duration::from_secs(20),
            state: 1,
            next_gone: None,
            counts: Counts::default(),
        };
        let chance = |name: &str, value: &str| -> Result<f64, String> {
            value
                .parse::<f64>()
                .ok()
                .filter(|c| (0. ..=1.).contains(c))
                .ok_or_else(|| format!("{name}: `{value}` is not a chance from 0 to 1"))
        };
        let whole = |name: &str, value: &str| -> Result<u64, String> {
            value
                .parse::<u64>()
                .map_err(|_| format!("{name}: `{value}` is not a whole number"))
        };
        for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (name, value) = part
                .split_once('=')
                .ok_or_else(|| format!("`{part}` is not name=value"))?;
            match name {
                "refuse" => plan.refuse = chance(name, value)?,
                "drop" => plan.drop = chance(name, value)?,
                "slow" => {
                    let (chance_text, ms) = value
                        .split_once('@')
                        .ok_or_else(|| format!("slow: `{value}` is not chance@milliseconds"))?;
                    plan.slow = chance(name, chance_text)?;
                    plan.slow_for = Duration::from_millis(whole(name, ms)?);
                }
                "gone" => {
                    let seconds = whole(name, value)?;
                    plan.gone_every = (seconds > 0).then(|| Duration::from_secs(seconds));
                }
                "cut" => {
                    let seconds = whole(name, value)?;
                    plan.cut_after = (seconds > 0).then(|| Duration::from_secs(seconds));
                }
                "after" => plan.after = Duration::from_secs(whole(name, value)?),
                "seed" => plan.state = whole(name, value)?,
                _ => return Err(format!("unknown fault `{name}`")),
            }
        }
        plan.next_gone = plan.gone_every.map(|every| plan.after + every);
        Ok(plan)
    }

    /// The fault for a request made `elapsed` into the run. Only a watch can
    /// be answered Gone.
    pub fn decide(&mut self, elapsed: Duration, watching: bool) -> Fault {
        if elapsed < self.after {
            return Fault::None;
        }
        if watching
            && let (Some(due), Some(every)) = (self.next_gone, self.gone_every)
            && elapsed >= due
        {
            self.next_gone = Some(elapsed + every);
            self.counts.gone += 1;
            return Fault::Gone;
        }
        // One draw per fault, in a fixed order, so a plan's sequence doesn't
        // shift when another fault's chance is changed.
        let (drop, refuse, slow) = (self.draw(), self.draw(), self.draw());
        if drop < self.drop {
            self.counts.dropped += 1;
            Fault::Drop
        } else if refuse < self.refuse {
            self.counts.refused += 1;
            Fault::Refuse
        } else if slow < self.slow {
            self.counts.slow += 1;
            Fault::Slow(self.slow_for)
        } else {
            Fault::None
        }
    }

    /// How long a watch that starts `elapsed` into the run may stay open, if
    /// the plan cuts watches. Only a watch is cut, and not before `after`.
    pub fn cut(&mut self, elapsed: Duration, watching: bool) -> Option<Duration> {
        let cut = self.cut_after?;
        (watching && elapsed >= self.after).then(|| {
            self.counts.cut += 1;
            cut
        })
    }

    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// A SplitMix64 step, as a number in [0, 1).
    fn draw(&mut self) -> f64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LATER: Duration = Duration::from_secs(100);

    fn run(plan: &mut FaultPlan, requests: usize) -> Vec<Fault> {
        (0..requests).map(|_| plan.decide(LATER, false)).collect()
    }

    #[test]
    fn an_empty_plan_injects_nothing() {
        let mut plan = FaultPlan::parse("").unwrap();
        assert!(run(&mut plan, 1000).iter().all(|f| *f == Fault::None));
        assert_eq!(plan.counts(), Counts::default());
    }

    #[test]
    fn nothing_happens_before_the_grace_period() {
        let mut plan = FaultPlan::parse("drop=1,after=30").unwrap();
        assert_eq!(plan.decide(Duration::from_secs(29), false), Fault::None);
        assert_eq!(plan.decide(Duration::from_secs(30), false), Fault::Drop);
    }

    #[test]
    fn the_same_seed_meets_the_same_faults() {
        let text = "drop=0.1,refuse=0.1,slow=0.1@500,seed=7";
        let a = run(&mut FaultPlan::parse(text).unwrap(), 500);
        let b = run(&mut FaultPlan::parse(text).unwrap(), 500);
        let c = run(
            &mut FaultPlan::parse("drop=0.1,refuse=0.1,slow=0.1@500,seed=8").unwrap(),
            500,
        );
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn chances_come_out_near_what_was_asked() {
        let mut plan = FaultPlan::parse("drop=0.05,refuse=0.1,slow=0.2@250").unwrap();
        run(&mut plan, 20_000);
        let c = plan.counts();
        // Each later fault draws from what an earlier one left.
        assert!((850..1150).contains(&c.dropped), "{c:?}");
        assert!((1700..2100).contains(&c.refused), "{c:?}");
        assert!((3200..3800).contains(&c.slow), "{c:?}");
    }

    #[test]
    fn a_slow_answer_carries_its_delay() {
        let mut plan = FaultPlan::parse("slow=1@1500").unwrap();
        assert_eq!(
            plan.decide(LATER, false),
            Fault::Slow(Duration::from_millis(1500))
        );
    }

    #[test]
    fn only_a_watch_is_ever_gone_and_once_per_interval() {
        let mut plan = FaultPlan::parse("gone=60,after=10").unwrap();
        let at = |s| Duration::from_secs(s);
        assert_eq!(
            plan.decide(at(80), false),
            Fault::None,
            "a list is never Gone"
        );
        assert_eq!(
            plan.decide(at(69), true),
            Fault::None,
            "not due before after + interval"
        );
        assert_eq!(plan.decide(at(80), true), Fault::Gone);
        assert_eq!(
            plan.decide(at(81), true),
            Fault::None,
            "the next is an interval later"
        );
        assert_eq!(plan.decide(at(140), true), Fault::Gone);
        assert_eq!(plan.counts().gone, 2);
    }

    #[test]
    fn only_a_watch_after_the_grace_period_is_cut() {
        let mut plan = FaultPlan::parse("cut=90,after=10").unwrap();
        let at = |s| Duration::from_secs(s);
        assert_eq!(plan.cut(at(5), true), None, "before the grace period");
        assert_eq!(plan.cut(at(50), false), None, "a list is never cut");
        assert_eq!(plan.cut(at(50), true), Some(at(90)));
        assert_eq!(plan.counts().cut, 1);
        assert_eq!(FaultPlan::parse("").unwrap().cut(at(50), true), None);
    }

    #[test]
    fn bad_text_is_refused_with_its_name() {
        for (text, wanted) in [
            ("drop=2", "drop"),
            ("refuse=x", "refuse"),
            ("slow=0.5", "chance@milliseconds"),
            ("gone=-1", "gone"),
            ("cut=x", "cut"),
            ("flood=1", "unknown fault `flood`"),
            ("drop", "name=value"),
        ] {
            let error = FaultPlan::parse(text).unwrap_err();
            assert!(error.contains(wanted), "{text}: {error}");
        }
    }
}
