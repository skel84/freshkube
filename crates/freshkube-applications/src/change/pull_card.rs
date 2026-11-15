//! The pull request card's words: what the strip's compact card and the
//! Inspector's full one say, derived when the read changes and never in
//! `render`. A pure function of what the cluster knows (the number and the
//! head commit) and what GitHub answered, so each state is a unit test.
//!
//! Unknown keeps its cause: a card for a pull request that wasn't read says
//! why (not set up, can't read), never "no reviews" or "checks passing".
use chrono::{DateTime, Utc};
use freshkube_delivery::Address;
use freshkube_delivery::github::{
    AvatarUrl, ChecksState, PullDetail, PullState, PullSummary, ReviewState, Reviewer, RunState,
    StatusState,
};

use super::*;

/// What the card knows without asking GitHub: the pull request's number
/// (from the build's annotations) and the commit the change deploys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Known {
    pub number: Option<u64>,
    pub commit: Option<String>,
}

/// Where the read of GitHub stands, for what the card says it lacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reading {
    /// Reading now, nothing held yet.
    Loading,
    /// The read answered, or its answer is kept.
    Held,
    /// The app has no client for GitHub (the token couldn't be had).
    NoClient,
    /// GitHub was asked and didn't give the answer (words in `error`).
    Failed,
    /// Nothing asked: example data without a pull request, or a commit
    /// GitHub names no pull request for.
    Nothing,
}

/// One person on the card and their part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Person {
    pub login: SharedString,
    pub avatar: Option<AvatarUrl>,
    /// `Author`, `Approved`, `Asked for changes`, `Review requested` …
    pub role: SharedString,
    pub tone: Tone,
}

/// One check as a segment of the bar and a line of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Check {
    pub name: SharedString,
    pub tone: Tone,
    pub word: &'static str,
    pub page: Option<Address>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Checks {
    /// `2 of 3 passed`, `100 of 112 checks read, 98 passed`.
    pub words: SharedString,
    pub tone: Tone,
    pub list: Vec<Check>,
}

/// A link the card opens in the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CardLink {
    pub id: &'static str,
    pub label: &'static str,
    pub address: Address,
}

/// What the card lacks, with its cause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Lack {
    pub tone: Tone,
    /// `Not set up`, `Can't read`.
    pub lead: &'static str,
    pub text: SharedString,
    /// Offers Settings, where GitHub is connected.
    pub connect: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct PullCard {
    /// `#418 Retry idempotency keys`, `#418`, or `Commit a1f3c9e`.
    pub title: SharedString,
    pub repo: Option<SharedString>,
    /// `Merged`, `Open` …; none until GitHub answered.
    pub state: Option<(&'static str, Tone)>,
    pub people: Vec<Person>,
    pub checks: Option<Checks>,
    /// `Merged by mira, 21 Oct 09:24 UTC`.
    pub merged: Option<SharedString>,
    /// `+84 −12 · 5 files`.
    pub changed: Option<SharedString>,
    /// The labels, joined once here: `reliability, urgent`.
    pub labels: Option<SharedString>,
    /// Set when what the card shows is old, or couldn't be read again: its
    /// tone is Warn and the compact line says so (never green and current).
    pub old: Option<SharedString>,
    pub links: Vec<CardLink>,
    /// `checks are of the head commit, not the deployed one`, old answers
    /// and the like.
    pub notes: Vec<SharedString>,
    pub lack: Option<Lack>,
    /// The strip's second line: state, reviews, checks, or what is lacking.
    pub compact: SharedString,
    pub tone: Tone,
}

fn tone_of(state: PullState) -> Tone {
    match state {
        PullState::Merged => Tone::Good,
        PullState::Open | PullState::Draft => Tone::Info,
        PullState::Closed => Tone::Warn,
    }
}

fn at(time: DateTime<Utc>) -> String {
    time.format("%-d %b %H:%M UTC").to_string()
}

fn short(commit: &str) -> String {
    commit.chars().take(7).collect()
}

fn same_commit(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    a.starts_with(&b) || b.starts_with(&a)
}

/// The merge commit's page, built from the checked repository and a hex
/// commit; nothing in an answer is followed.
fn commit_page(repo: &str, sha: &str) -> Option<Address> {
    let hex = sha.len() >= 7 && sha.bytes().all(|b| b.is_ascii_hexdigit());
    let repo_ok = repo.split('/').count() == 2
        && repo
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.'));
    (hex && repo_ok)
        .then(|| Address::parse(&format!("https://github.com/{repo}/commit/{sha}")).ok())
        .flatten()
}

fn run_look(state: RunState) -> (Tone, &'static str) {
    match state {
        RunState::Passed => (Tone::Good, "passed"),
        RunState::Failed => (Tone::Crit, "failed"),
        RunState::Queued => (Tone::Waiting, "queued"),
        RunState::Running => (Tone::Waiting, "running"),
        RunState::Neutral => (Tone::Unknown, "skipped"),
        RunState::Cancelled => (Tone::Unknown, "cancelled"),
    }
}

fn checks_of(detail: &PullDetail) -> Option<Checks> {
    let checks = &detail.checks;
    let mut list: Vec<Check> = checks
        .runs
        .iter()
        .map(|run| {
            let (tone, word) = run_look(run.state);
            Check {
                name: run.name.clone().into(),
                tone,
                word,
                page: run.page.clone(),
            }
        })
        .collect();
    list.extend(checks.statuses.iter().map(|status| {
        let (tone, word) = match status.state {
            StatusState::Passed => (Tone::Good, "passed"),
            StatusState::Failed => (Tone::Crit, "failed"),
            StatusState::Pending => (Tone::Waiting, "pending"),
        };
        Check {
            name: status.context.clone().into(),
            tone,
            word,
            page: status.page.clone(),
        }
    }));
    if checks.state == ChecksState::None || list.is_empty() {
        // Read, and there are none: said, not left out.
        return Some(Checks {
            words: "No checks".into(),
            tone: Tone::Outline,
            list,
        });
    }
    let passed = list.iter().filter(|check| check.tone == Tone::Good).count();
    let cut = checks.capped.or(checks.statuses_capped);
    let words = match cut {
        Some(cut) => format!("{} of {} checks read, {passed} passed", cut.read, cut.total),
        None => format!("{passed} of {} passed", list.len()),
    };
    let tone = match checks.state {
        ChecksState::Passing => Tone::Good,
        ChecksState::Failing => Tone::Crit,
        ChecksState::Pending => Tone::Waiting,
        ChecksState::Ended | ChecksState::Partial | ChecksState::None => Tone::Unknown,
    };
    Some(Checks {
        words: words.into(),
        tone,
        list,
    })
}

fn people_of(summary: &PullSummary, detail: Option<&PullDetail>) -> Vec<Person> {
    let mut people = Vec::new();
    if let Some(author) = &summary.author {
        people.push(Person {
            login: author.login.clone().into(),
            avatar: author.avatar,
            role: "Author".into(),
            tone: Tone::Outline,
        });
    }
    let head = summary.head.as_deref();
    let reviews = detail
        .map(|detail| detail.reviews.as_slice())
        .unwrap_or(&[]);
    for review in reviews {
        let (role, tone) = match review.state {
            ReviewState::Approved => ("Approved", Tone::Good),
            ReviewState::ChangesRequested => ("Asked for changes", Tone::Crit),
            ReviewState::Commented => ("Commented", Tone::Outline),
            ReviewState::Dismissed => ("Review dismissed", Tone::Unknown),
        };
        // A review of an older commit says so: it isn't of what merged.
        let older = matches!(
            (review.commit.as_deref(), head),
            (Some(given), Some(head)) if !same_commit(given, head)
        );
        people.push(Person {
            login: review.who.login.clone().into(),
            avatar: review.who.avatar,
            role: if older {
                format!("{role}, an older commit").into()
            } else {
                role.into()
            },
            tone,
        });
    }
    for requested in &summary.requested {
        let (login, avatar) = match requested {
            Reviewer::User(person) => (person.login.clone(), person.avatar),
            Reviewer::Team { slug, name } => (name.clone().unwrap_or_else(|| slug.clone()), None),
        };
        if people.iter().any(|seen| seen.login == login) {
            continue;
        }
        people.push(Person {
            login: login.into(),
            avatar,
            role: "Review requested".into(),
            tone: Tone::Waiting,
        });
    }
    people
}

fn compact_of(card: &PullCard, detail: Option<&PullDetail>) -> String {
    let Some((state, _)) = card.state else {
        return card
            .lack
            .as_ref()
            .map(|lack| format!("{} · {}", lack.lead, lack.text))
            .unwrap_or_default();
    };
    let mut parts = vec![state.to_owned()];
    // Old answers say so first: a narrow strip cuts the end of the line.
    if let Some(old) = &card.old {
        parts.push(old.to_string());
    }
    if let Some(detail) = detail {
        let approved = detail
            .reviews
            .iter()
            .filter(|review| review.state == ReviewState::Approved)
            .count();
        let asked = detail
            .reviews
            .iter()
            .filter(|review| review.state == ReviewState::ChangesRequested)
            .count();
        parts.push(match (approved, asked) {
            (0, 0) => "no decision".to_owned(),
            (a, 0) => format!("{a} approved"),
            (0, c) => format!("{c} asked for changes"),
            (a, c) => format!("{a} approved, {c} asked for changes"),
        });
    }
    if let Some(checks) = &card.checks {
        parts.push(if checks.list.is_empty() {
            "no checks".to_owned()
        } else {
            format!("checks {}", checks.words)
        });
    }
    if let Some(lack) = &card.lack {
        parts.push(format!("{}: {}", lack.lead, lack.text));
    }
    parts.join(" · ")
}

/// The card from what is held. `summary` is the change's own (from the
/// list read), `detail` the card's read, `error` why that read failed.
pub(super) fn derive(
    known: &Known,
    summary: Option<&PullSummary>,
    detail: Option<&PullDetail>,
    error: Option<&str>,
    reading: Reading,
) -> PullCard {
    let summary = detail.map(|detail| &detail.summary).or(summary);
    let title: SharedString = match (summary, known) {
        (Some(pull), _) => match &pull.title {
            Some(title) => format!("#{} {title}", pull.number),
            None => format!("#{}", pull.number),
        },
        (
            None,
            Known {
                number: Some(number),
                ..
            },
        ) => format!("#{number}"),
        (
            None,
            Known {
                commit: Some(commit),
                ..
            },
        ) => format!("Commit {}", short(commit)),
        _ => "Pull request".to_owned(),
    }
    .into();
    let mut card = PullCard {
        title,
        ..PullCard::default()
    };
    if let Some(summary) = summary {
        card.repo = Some(summary.repo.clone().into());
        card.state = Some((summary.state.label(), tone_of(summary.state)));
        card.people = people_of(summary, detail);
        card.links
            .extend(summary.page.clone().map(|address| CardLink {
                id: "open",
                label: "Open on GitHub",
                address,
            }));
        if let (Some(at_), Some(by)) = (summary.merged_at, summary.merged_by.as_ref()) {
            card.merged = Some(format!("Merged by {}, {}", by.login, at(at_)).into());
        } else if let Some(at_) = summary.merged_at {
            card.merged = Some(format!("Merged {}", at(at_)).into());
        }
    }
    if let Some(detail) = detail {
        card.checks = checks_of(detail);
        card.changed = detail.changed.map(|changed| {
            format!(
                "+{} −{} · {} file{}",
                changed.additions,
                changed.deletions,
                changed.files,
                if changed.files == 1 { "" } else { "s" }
            )
            .into()
        });
        card.labels = (!detail.labels.is_empty()).then(|| {
            detail
                .labels
                .iter()
                .map(|label| label.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
                .into()
        });
        if let Some(address) = detail
            .merge_commit
            .as_deref()
            .and_then(|sha| commit_page(&detail.summary.repo, sha))
        {
            card.links.push(CardLink {
                id: "merge-commit",
                label: "Merge commit",
                address,
            });
        }
        if let (Some(head), Some(deployed)) =
            (detail.summary.head.as_deref(), known.commit.as_deref())
            && !same_commit(head, deployed)
        {
            card.notes
                .push("Checks are of the head commit, not the deployed one.".into());
        }
        if error.is_some() {
            card.notes
                .push(format!("Couldn't read again: {}", error.unwrap_or_default()).into());
        }
    }
    // An old answer says only that it is old; why is the banner of what
    // may be missing's alone, so the two never say it twice.
    let stale = detail
        .and_then(|detail| detail.stale.as_ref())
        .or(summary.and_then(|pull| pull.stale.as_ref()));
    if let Some(stale) = stale {
        card.notes.push(stale.read_words().into());
        card.old = Some(stale.read_words().into());
    } else if detail.is_some() && error.is_some() {
        card.old = Some("couldn't read again".into());
    }
    // What can't be had says why, whatever else is held.
    card.lack = match (detail, reading) {
        (Some(_), _) => None,
        (None, Reading::NoClient) => Some(Lack {
            tone: Tone::Integration,
            lead: "Not set up",
            text: "GitHub isn't connected, so only the number and the commit the cluster \
                   records are shown. Connect GitHub in Settings."
                .into(),
            connect: true,
        }),
        (None, Reading::Failed) => Some(Lack {
            tone: Tone::Warn,
            lead: "Can't read",
            text: error.unwrap_or("GitHub didn't answer").to_owned().into(),
            connect: false,
        }),
        (None, Reading::Loading) => None,
        (None, Reading::Held | Reading::Nothing) => summary.is_none().then(|| Lack {
            tone: Tone::Unknown,
            lead: "Not read",
            text: match known.number {
                Some(_) => "GitHub wasn't read for this pull request; what may be missing \
                            says why when it is known."
                    .into(),
                None => "GitHub named no pull request in what was read for this commit.".into(),
            },
            connect: false,
        }),
    };
    card.tone = match (&card.lack, card.state, &card.old) {
        (Some(lack), ..) => lack.tone,
        // Old or unread-again data is never drawn as current and fine.
        (None, Some(_), Some(_)) => Tone::Warn,
        (None, Some((_, tone)), None) => tone,
        (None, None, _) => Tone::Unknown,
    };
    card.compact = match (reading, detail, &card.lack) {
        (Reading::Loading, None, _) => "reading GitHub…".to_owned(),
        _ => compact_of(&card, detail),
    }
    .into();
    if let Some(commit) = known.commit.as_deref()
        && card.state.is_none()
        && card.lack.is_some()
    {
        card.notes
            .push(format!("Deployed commit {}", short(commit)).into());
    }
    card
}

#[cfg(test)]
mod tests {
    // Not `super::*`: it brings in gpui's `test` macro, which hides `#[test]`.
    use super::{Known, Reading, commit_page, derive};
    use freshkube_delivery::github::{PullDetail, PullSummary, example_detail, example_pull};
    use freshkube_ui::ui::Tone;

    fn known() -> Known {
        Known {
            number: Some(418),
            commit: Some("a1f3c9e".into()),
        }
    }

    fn held(number: u64) -> (PullSummary, PullDetail) {
        let summary = example_pull(number).expect("an example pull request");
        let detail = example_detail(&summary, None).expect("example detail");
        (summary, detail)
    }

    #[test]
    fn a_merged_pull_request_has_its_people_checks_and_links() {
        let (summary, detail) = held(418);
        let card = derive(&known(), Some(&summary), Some(&detail), None, Reading::Held);
        assert_eq!(&*card.title, "#418 Retry idempotency keys");
        assert_eq!(card.state, Some(("Merged", Tone::Good)));
        let people: Vec<_> = card
            .people
            .iter()
            .map(|p| (p.login.to_string(), p.role.to_string()))
            .collect();
        assert_eq!(
            people,
            [
                ("ken".to_owned(), "Author".to_owned()),
                ("mira".to_owned(), "Approved".to_owned()),
                ("zoe".to_owned(), "Commented".to_owned()),
            ]
        );
        assert!(card.people.iter().all(|p| p.avatar.is_some()));
        let checks = card.checks.as_ref().expect("checks");
        assert_eq!(&*checks.words, "3 of 3 passed");
        assert_eq!(checks.list.len(), 3);
        assert_eq!(card.changed.as_deref(), Some("+84 −12 · 5 files"));
        assert_eq!(card.labels.as_deref(), Some("reliability"));
        assert!(card.old.is_none() && card.tone == Tone::Good);
        assert!(
            card.merged
                .as_deref()
                .unwrap()
                .starts_with("Merged by mira, ")
        );
        let links: Vec<_> = card.links.iter().map(|l| l.id).collect();
        assert_eq!(links, ["open", "merge-commit"]);
        assert!(
            card.links[1]
                .address
                .as_str()
                .starts_with("https://github.com/acme/shop/commit/")
        );
        assert_eq!(&*card.compact, "Merged · 1 approved · checks 3 of 3 passed");
        assert!(card.lack.is_none());
    }

    #[test]
    fn an_open_pull_request_waits_on_its_checks_and_names_who_was_asked() {
        let (summary, detail) = held(421);
        let card = derive(&known(), Some(&summary), Some(&detail), None, Reading::Held);
        assert_eq!(card.state.map(|(word, _)| word), Some("Open"));
        let checks = card.checks.as_ref().expect("checks");
        assert_eq!(&*checks.words, "2 of 3 passed");
        assert_eq!(checks.tone, Tone::Waiting);
        assert!(
            card.compact.starts_with("Open · no decision"),
            "{}",
            card.compact
        );
        assert!(card.merged.is_none() && card.links.iter().all(|l| l.id != "merge-commit"));
    }

    #[test]
    fn without_a_client_it_says_not_set_up_and_what_the_cluster_knows() {
        let card = derive(&known(), None, None, None, Reading::NoClient);
        assert_eq!(&*card.title, "#418");
        let lack = card.lack.as_ref().expect("a lack");
        assert_eq!((lack.lead, lack.connect), ("Not set up", true));
        assert!(lack.text.contains("Connect GitHub in Settings"));
        assert!(card.compact.starts_with("Not set up"), "{}", card.compact);
        assert!(card.people.is_empty() && card.checks.is_none());
        assert!(card.notes.iter().any(|n| n.contains("a1f3c9e")));
    }

    #[test]
    fn a_refused_read_is_cant_read_with_its_words_and_never_none() {
        let words = "GitHub's rate limit is reached until 14:05 UTC";
        let card = derive(&known(), None, None, Some(words), Reading::Failed);
        let lack = card.lack.as_ref().expect("a lack");
        assert_eq!(
            (lack.lead, lack.tone, lack.connect),
            ("Can't read", Tone::Warn, false)
        );
        assert_eq!(&*lack.text, words);
        assert!(card.checks.is_none(), "unread checks are not none or fine");
        assert!(card.people.is_empty());
    }

    #[test]
    fn a_commit_with_no_pull_request_read_is_a_commit_card() {
        let only_commit = Known {
            number: None,
            commit: Some("a1f3c9e0123".into()),
        };
        let card = derive(&only_commit, None, None, None, Reading::Held);
        assert_eq!(&*card.title, "Commit a1f3c9e");
        assert_eq!(card.lack.as_ref().map(|l| l.lead), Some("Not read"));
    }

    #[test]
    fn an_old_answer_and_a_head_that_isnt_the_deployed_commit_are_noted() {
        let (summary, detail) = held(418);
        let other = Known {
            number: Some(418),
            commit: Some("beefbee".into()),
        };
        let card = derive(&other, Some(&summary), Some(&detail), None, Reading::Held);
        assert!(
            card.notes
                .iter()
                .any(|n| n.contains("head commit, not the deployed one"))
        );
        let card = derive(
            &known(),
            Some(&summary),
            Some(&detail),
            Some("GitHub didn't answer"),
            Reading::Held,
        );
        assert!(
            card.notes
                .iter()
                .any(|n| n.starts_with("Couldn't read again: GitHub didn't"))
        );
        assert!(card.lack.is_none(), "the held answer stands");
    }

    #[test]
    fn only_a_hex_commit_on_a_checked_repository_is_linked() {
        assert!(commit_page("acme/shop", "c0ffee0123").is_some());
        for (repo, sha) in [
            ("acme/shop", "../../x"),
            ("acme/shop", "zzzzzzz"),
            ("acme", "c0ffee0123"),
            ("acme/shop/extra", "c0ffee0123"),
            ("acme/sh op", "c0ffee0123"),
        ] {
            assert!(commit_page(repo, sha).is_none(), "{repo} {sha}");
        }
    }

    #[test]
    fn every_kind_of_review_is_a_person_and_an_old_approval_says_so() {
        use freshkube_delivery::github::{Review, ReviewState};
        let (summary, detail) = held(418);
        let head = summary.head.clone();
        let review = detail.reviews[0].clone();
        let with = |state: ReviewState, commit: Option<String>| {
            let mut detail = detail.clone();
            detail.reviews = vec![Review {
                state,
                commit,
                ..review.clone()
            }];
            let card = derive(&known(), Some(&summary), Some(&detail), None, Reading::Held);
            card.people[1].role.to_string()
        };
        assert_eq!(with(ReviewState::Commented, head.clone()), "Commented");
        assert_eq!(
            with(ReviewState::Dismissed, head.clone()),
            "Review dismissed"
        );
        assert_eq!(
            with(ReviewState::ChangesRequested, head.clone()),
            "Asked for changes"
        );
        assert_eq!(
            with(ReviewState::Approved, Some("f".repeat(40))),
            "Approved, an older commit"
        );
        // No review at all says nothing is decided, not that it is fine.
        let mut detail = detail.clone();
        detail.reviews.clear();
        let card = derive(&known(), Some(&summary), Some(&detail), None, Reading::Held);
        assert!(card.compact.contains("no decision"), "{}", card.compact);
    }

    #[test]
    fn checks_that_were_not_all_read_never_say_passing() {
        use freshkube_delivery::github::{Capped, ChecksState};
        let (summary, mut detail) = held(418);
        let words = |detail: &PullDetail| {
            let card = derive(&known(), Some(&summary), Some(detail), None, Reading::Held);
            let checks = card.checks.expect("a checks line");
            (checks.words.to_string(), checks.tone)
        };
        assert_eq!(words(&detail), ("3 of 3 passed".to_owned(), Tone::Good));
        detail.checks.state = ChecksState::Failing;
        assert_eq!(words(&detail).1, Tone::Crit);
        detail.checks.state = ChecksState::Ended;
        assert_eq!(words(&detail).1, Tone::Unknown);
        detail.checks.state = ChecksState::Partial;
        detail.checks.capped = Some(Capped {
            read: 3,
            total: 112,
        });
        assert_eq!(
            words(&detail),
            ("3 of 112 checks read, 3 passed".to_owned(), Tone::Unknown)
        );
        detail.checks.state = ChecksState::None;
        detail.checks.runs.clear();
        assert_eq!(words(&detail).0, "No checks");
    }

    #[test]
    fn a_kept_answer_is_never_green_and_current() {
        use freshkube_delivery::github::{Stale, example_detail};
        let (summary, mut detail) = held(418);
        detail.stale = Some(Stale {
            read_at: chrono::DateTime::from_timestamp(1_760_000_000, 0).unwrap(),
            why: "GitHub's rate limit is reached".into(),
        });
        let card = derive(&known(), Some(&summary), Some(&detail), None, Reading::Held);
        assert_eq!(card.tone, Tone::Warn);
        assert!(
            card.compact
                .starts_with("Merged · stale, read at 08:53 UTC · "),
            "{}",
            card.compact
        );
        // Why is the banner's, not the card's.
        assert!(!card.compact.contains("rate limit"));
        // An error beside held data reads the same way.
        let (summary, detail) = held(418);
        let _ = example_detail;
        let card = derive(
            &known(),
            Some(&summary),
            Some(&detail),
            Some("GitHub didn't answer"),
            Reading::Held,
        );
        assert_eq!(card.tone, Tone::Warn);
        assert!(
            card.compact.starts_with("Merged · couldn't read again · "),
            "{}",
            card.compact
        );
    }
}
