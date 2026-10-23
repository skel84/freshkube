//! The follower: which of the agent's reads the page shows, and when.
//!
//! It is fed only what the endpoint reports of a read that actually ran (a
//! [`ReadEvent`]'s `view`, outcome and cluster), never anything the agent
//! wrote, and it takes `now` from the caller, so a test steps a clock. It
//! chooses; it moves nothing. The app shows a [`Move`] through
//! `Pilot::apply_view`, as a deep link would be shown.
//!
//! - A refused read, and a read with no view, never moves the page.
//! - A read on another cluster never moves it either; the follower says which
//!   cluster instead ([`Fed::Elsewhere`]).
//! - Each view holds for [`DWELL`]. The latest place waits for the hold to
//!   end and the places in between are skipped, except a read the agent cited
//!   or one that explains a failure ([`Follower::keep`]), which gets its full
//!   hold.
//! - After the person takes the page back ([`Follower::hand_back`]) nothing
//!   moves; the places that come are counted ([`Follower::newer`]) until
//!   [`Follower::jump`] or [`Follower::resume`].

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use freshkube_nav::ViewState;

use crate::endpoint::{Outcome, ReadEvent};

/// How long a view holds before the next place may replace it.
pub const DWELL: Duration = Duration::from_millis(1500);
/// The reads a session may make, and so the most places the trail keeps.
const MAX_TRAIL: usize = 500;
/// The most places that wait at once; cited ones are kept, the rest are
/// already reduced to the newest, so this only bounds a flood of cited reads.
const MAX_WAITING: usize = 64;

/// A place to show: the read it came from and the view the window would go
/// to.
#[derive(Clone, Debug, PartialEq)]
pub struct Move {
    /// The read's number (`r7` is 7).
    pub read: u32,
    pub view: ViewState,
}

/// What the follower did with a read.
#[derive(Clone, Debug, PartialEq)]
pub enum Fed {
    /// The page shows it now.
    Show(Move),
    /// It waits for the hold of the view shown, or for the person.
    Waiting,
    /// It never moves the page: refused, no view, or following is stopped.
    Ignored,
    /// The read was on this other cluster; the page stays where it is.
    Elsewhere(String),
}

#[derive(Debug)]
struct Waiting {
    place: Move,
    keep: bool,
}

#[derive(Debug)]
pub struct Follower {
    /// The session's own entry.
    own: String,
    following: bool,
    /// What the page shows from the agent, and since when.
    shown: Option<(Move, Instant)>,
    waiting: VecDeque<Waiting>,
    /// Every place a read could show, in the order the reads ran.
    trail: Vec<Move>,
    /// Where in the trail the page is, for ◂ ▸.
    at: Option<usize>,
    /// Reads kept by number before they arrived or after they were skipped.
    kept: Vec<u32>,
    /// Places that came since the person took the page back.
    newer: usize,
    elsewhere: Option<String>,
}

impl Follower {
    /// A follower for the session on entry `own`, following.
    pub fn new(own: &str) -> Self {
        Self {
            own: own.to_owned(),
            following: true,
            shown: None,
            waiting: VecDeque::new(),
            trail: Vec::new(),
            at: None,
            kept: Vec::new(),
            newer: 0,
            elsewhere: None,
        }
    }

    /// Takes a read the endpoint reported.
    pub fn feed(&mut self, event: &ReadEvent, now: Instant) -> Fed {
        if event.cluster != self.own {
            self.elsewhere = Some(event.cluster.clone());
            return Fed::Elsewhere(event.cluster.clone());
        }
        let Outcome::Answered { .. } = event.outcome else {
            return Fed::Ignored;
        };
        let Some(view) = &event.view else {
            return Fed::Ignored;
        };
        let place = Move {
            read: event.read.number(),
            view: view.clone(),
        };
        if self.trail.len() < MAX_TRAIL {
            self.trail.push(place.clone());
        }
        if !self.following {
            self.newer += 1;
            return Fed::Ignored;
        }
        let keep = self.kept.contains(&place.read);
        let free = match &self.shown {
            None => true,
            Some((_, since)) => self.waiting.is_empty() && held(*since, now),
        };
        if free {
            return Fed::Show(self.show(place, now));
        }
        // Latest wins: the places that wait and were not kept give way.
        self.waiting.retain(|waiting| waiting.keep);
        if self.waiting.len() >= MAX_WAITING {
            self.waiting.pop_front();
        }
        self.waiting.push_back(Waiting { place, keep });
        Fed::Waiting
    }

    /// The next place, once the view shown has held long enough. Call it when
    /// [`next_due`](Self::next_due) passes.
    pub fn due(&mut self, now: Instant) -> Option<Move> {
        if !self.following {
            return None;
        }
        let (_, since) = self.shown.as_ref()?;
        if !held(*since, now) {
            return None;
        }
        let waiting = self.waiting.pop_front()?;
        Some(self.show(waiting.place, now))
    }

    /// When the next waiting place is due, if one waits.
    pub fn next_due(&self) -> Option<Instant> {
        if !self.following || self.waiting.is_empty() {
            return None;
        }
        self.shown.as_ref().map(|(_, since)| *since + DWELL)
    }

    /// Gives the read `number` its full hold: the agent cited it, or it
    /// explains a failure. It is not skipped by newer places.
    pub fn keep(&mut self, number: u32) {
        if !self.kept.contains(&number) && self.kept.len() < MAX_TRAIL {
            self.kept.push(number);
        }
        for waiting in &mut self.waiting {
            if waiting.place.read == number {
                waiting.keep = true;
            }
        }
    }

    /// The person took the page: nothing moves until [`jump`](Self::jump) or
    /// [`resume`](Self::resume).
    pub fn hand_back(&mut self) {
        if !self.following {
            return;
        }
        self.following = false;
        self.newer = 0;
        self.waiting.clear();
    }

    /// Follows again from the next read.
    pub fn resume(&mut self) {
        self.following = true;
        self.newer = 0;
    }

    /// Follows again at the latest place, which the page shows now.
    pub fn jump(&mut self, now: Instant) -> Option<Move> {
        self.following = true;
        self.newer = 0;
        self.waiting.clear();
        let latest = self.trail.last().cloned()?;
        Some(self.show(latest, now))
    }

    /// Steps through the trail (◂ is -1, ▸ is 1) and shows that place. The
    /// hold starts again, so the next waiting place comes after it.
    pub fn step(&mut self, by: isize, now: Instant) -> Option<Move> {
        let at = self.at?;
        let to = at.checked_add_signed(by)?;
        let place = self.trail.get(to)?.clone();
        Some(self.show(place, now))
    }

    /// Shows the trail's place for `number`, as a click on its row does.
    pub fn go_to(&mut self, number: u32, now: Instant) -> Option<Move> {
        let place = self
            .trail
            .iter()
            .find(|place| place.read == number)?
            .clone();
        Some(self.show(place, now))
    }

    fn show(&mut self, place: Move, now: Instant) -> Move {
        self.at = self.trail.iter().position(|p| p.read == place.read);
        self.shown = Some((place.clone(), now));
        place
    }

    pub fn following(&self) -> bool {
        self.following
    }

    /// The place the page shows from the agent.
    pub fn shown(&self) -> Option<&Move> {
        self.shown.as_ref().map(|(place, _)| place)
    }

    /// When the view shown started its hold, for the strip's hairline.
    pub fn since(&self) -> Option<Instant> {
        self.shown.as_ref().map(|(_, since)| *since)
    }

    /// "n of m": the place's position in the trail, and the trail's length.
    pub fn position(&self) -> Option<(usize, usize)> {
        Some((self.at? + 1, self.trail.len()))
    }

    /// How many places came since the person took the page back.
    pub fn newer(&self) -> usize {
        self.newer
    }

    /// The other cluster the agent last read, if it read one.
    pub fn elsewhere(&self) -> Option<&str> {
        self.elsewhere.as_deref()
    }

    /// A new question starts a new trail.
    pub fn clear(&mut self) {
        let following = self.following;
        *self = Self::new(&self.own.clone());
        self.following = following;
    }
}

fn held(since: Instant, now: Instant) -> bool {
    now.saturating_duration_since(since) >= DWELL
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::Outcome;
    use crate::session::{ReadIds, SessionId};
    use freshkube_core::agent_reads::Code;
    use freshkube_nav::{KindKey, LinkPage, ViewState};

    struct Run {
        ids: ReadIds,
        session: SessionId,
        start: Instant,
    }

    impl Run {
        fn new() -> Self {
            Self {
                ids: ReadIds::default(),
                session: SessionId::new(),
                start: Instant::now(),
            }
        }

        fn at(&self, millis: u64) -> Instant {
            self.start + Duration::from_millis(millis)
        }

        fn event(&self, cluster: &str, kind: &str, outcome: Outcome, viewed: bool) -> ReadEvent {
            let mut view = ViewState::page(LinkPage::Resources);
            view.kind = Some(KindKey::parse(kind).unwrap());
            ReadEvent {
                session: self.session,
                read: self.ids.next(),
                tool: "list_objects",
                cluster: cluster.to_owned(),
                view: viewed.then_some(view),
                outcome,
            }
        }

        fn read(&self, kind: &str) -> ReadEvent {
            self.event("a", kind, Outcome::Answered { cut: None }, true)
        }
    }

    fn kind(place: &Move) -> &str {
        place.view.kind.as_ref().unwrap().as_str()
    }

    #[test]
    fn each_view_holds_at_least_one_and_a_half_seconds() {
        let run = Run::new();
        let mut follower = Follower::new("a");
        let Fed::Show(first) = follower.feed(&run.read("pods"), run.at(0)) else {
            panic!("the first read shows at once");
        };
        assert_eq!(kind(&first), "pods");
        assert_eq!(
            follower.feed(&run.read("services"), run.at(400)),
            Fed::Waiting
        );
        assert_eq!(
            follower.feed(&run.read("deployments.apps"), run.at(800)),
            Fed::Waiting
        );
        assert_eq!(follower.due(run.at(1499)), None);
        assert_eq!(follower.next_due(), Some(run.at(1500)));
        let shown = follower.due(run.at(1500)).expect("the hold has ended");
        assert_eq!(kind(&shown), "deployments.apps", "the second was skipped");
        assert_eq!(follower.due(run.at(3000)), None);
    }

    #[test]
    fn a_read_after_the_hold_shows_at_once() {
        let run = Run::new();
        let mut follower = Follower::new("a");
        follower.feed(&run.read("pods"), run.at(0));
        assert!(matches!(
            follower.feed(&run.read("services"), run.at(1500)),
            Fed::Show(_)
        ));
    }

    #[test]
    fn cited_evidence_gets_its_full_hold() {
        let run = Run::new();
        let mut follower = Follower::new("a");
        follower.feed(&run.read("pods"), run.at(0));
        let cited = run.read("services");
        follower.feed(&cited, run.at(300));
        follower.keep(cited.read.number());
        follower.feed(&run.read("deployments.apps"), run.at(600));
        follower.feed(&run.read("nodes"), run.at(900));
        let second = follower.due(run.at(1500)).unwrap();
        assert_eq!(kind(&second), "services", "the cited read was not skipped");
        assert_eq!(follower.due(run.at(2999)), None, "and it holds in full");
        let third = follower.due(run.at(3000)).unwrap();
        assert_eq!(kind(&third), "nodes", "the uncited one between was skipped");
    }

    #[test]
    fn a_read_kept_before_it_arrives_is_kept() {
        let run = Run::new();
        let mut follower = Follower::new("a");
        follower.feed(&run.read("pods"), run.at(0));
        let early = run.read("services");
        follower.keep(early.read.number());
        follower.feed(&early, run.at(300));
        follower.feed(&run.read("nodes"), run.at(600));
        assert_eq!(kind(&follower.due(run.at(1500)).unwrap()), "services");
    }

    #[test]
    fn refused_and_viewless_reads_never_move_the_page() {
        let run = Run::new();
        let mut follower = Follower::new("a");
        let refused = run.event("a", "pods", Outcome::Refused(Code::Refused), true);
        let viewless = run.event("a", "pods", Outcome::Answered { cut: None }, false);
        assert_eq!(follower.feed(&refused, run.at(0)), Fed::Ignored);
        assert_eq!(follower.feed(&viewless, run.at(0)), Fed::Ignored);
        assert_eq!(follower.shown(), None);
        assert_eq!(follower.position(), None);
        follower.feed(&run.read("pods"), run.at(10));
        assert_eq!(follower.feed(&refused, run.at(20)), Fed::Ignored);
        assert_eq!(follower.due(run.at(5000)), None, "nothing waits");
    }

    #[test]
    fn a_read_on_another_cluster_never_moves_the_page() {
        let run = Run::new();
        let mut follower = Follower::new("a");
        let other = run.event("b", "pods", Outcome::Answered { cut: None }, true);
        assert_eq!(
            follower.feed(&other, run.at(0)),
            Fed::Elsewhere("b".to_owned())
        );
        assert_eq!(follower.shown(), None);
        assert_eq!(follower.elsewhere(), Some("b"));
        follower.feed(&run.read("nodes"), run.at(10));
        follower.feed(&other, run.at(20));
        assert_eq!(follower.due(run.at(5000)), None);
        assert_eq!(follower.position(), Some((1, 1)));
    }

    #[test]
    fn handing_back_stops_moves_and_counts_the_newer_places() {
        let run = Run::new();
        let mut follower = Follower::new("a");
        follower.feed(&run.read("pods"), run.at(0));
        follower.feed(&run.read("services"), run.at(100));
        follower.hand_back();
        assert!(!follower.following());
        assert_eq!(follower.due(run.at(5000)), None, "what waited is dropped");
        follower.feed(&run.read("nodes"), run.at(6000));
        follower.feed(&run.read("events"), run.at(6100));
        assert_eq!(follower.newer(), 2);
        let jumped = follower.jump(run.at(7000)).unwrap();
        assert_eq!(kind(&jumped), "events", "Jump shows the latest place");
        assert!(follower.following());
        assert_eq!(follower.newer(), 0);
    }

    #[test]
    fn resuming_follows_the_next_read() {
        let run = Run::new();
        let mut follower = Follower::new("a");
        follower.feed(&run.read("pods"), run.at(0));
        follower.hand_back();
        follower.feed(&run.read("services"), run.at(2000));
        follower.resume();
        assert!(matches!(
            follower.feed(&run.read("nodes"), run.at(3000)),
            Fed::Show(_)
        ));
    }

    #[test]
    fn the_trail_steps_and_goes_to_a_read() {
        let run = Run::new();
        let mut follower = Follower::new("a");
        let reads: Vec<_> = ["pods", "services", "nodes"]
            .into_iter()
            .map(|k| run.read(k))
            .collect();
        for (n, read) in reads.iter().enumerate() {
            follower.feed(read, run.at(n as u64 * 100));
        }
        assert_eq!(follower.position(), Some((1, 3)));
        assert_eq!(
            follower.step(-1, run.at(2000)),
            None,
            "nothing before the first"
        );
        assert_eq!(kind(&follower.step(1, run.at(2000)).unwrap()), "services");
        assert_eq!(follower.position(), Some((2, 3)));
        let third = follower
            .go_to(reads[2].read.number(), run.at(2100))
            .unwrap();
        assert_eq!(kind(&third), "nodes");
        assert_eq!(follower.step(1, run.at(2200)), None);
        assert_eq!(follower.go_to(99, run.at(2300)), None);
    }
}
