//! The owner of the reviews that decide what is offered (#854, slice 2).
//!
//! Core's [`Offers`] holds what is known and what may land; this owns the
//! Tokio jobs and their delivery, one per question in flight, and drops them
//! with the answers when the access identity changes. One is shared by the
//! app ([`shared`]): Resources, the change page and Operations all read the
//! same answers, so a question asked on one page is not asked again on
//! another.
//!
//! A page calls [`OfferDesk::want`] with the questions its **selection's
//! actions** declare, when the selection changes or the page shows, never
//! from `render` and never for every row; it reads [`OfferDesk::offer`]
//! while it draws. Nothing here decides a write: the gate is core's
//! `write` preflight, which asks afresh.

use std::{collections::HashMap, time::Duration};

use freshkube_core::{
    access::{self, Offer, Offers, OfferTicket, Verdict},
    cluster_source::ClusterSource,
    job::{OwnedJob, spawn_job},
    write::{self, AccessQuestion},
};
use gpui_kit::*;
use tokio::runtime::Handle;

/// One review, or the question of who you are.
const DEADLINE: Duration = Duration::from_secs(15);

/// The most questions waiting to be asked at once; a selection has a handful.
const QUEUE: usize = 32;

/// Answers (or who you are) changed: a page that shows what is offered
/// draws again.
pub struct OfferChanged;

pub struct OfferDesk {
    runtime: Handle,
    source: Option<ClusterSource>,
    /// With `--fixture`: how the example answers (`FRESHKUBE_ACCESS`).
    example: access::example::Shape,
    offers: Offers,
    who: Option<(OwnedJob, Task<()>)>,
    jobs: HashMap<AccessQuestion, (OwnedJob, Task<()>)>,
    /// Wanted, and not yet asked: who you are is not known, or four
    /// reviews are already in flight.
    queue: Vec<AccessQuestion>,
    /// Every question asked, for tests that prove what is never asked.
    #[cfg(feature = "testing")]
    asked: Vec<AccessQuestion>,
}

impl EventEmitter<OfferChanged> for OfferDesk {}

struct Shared(Entity<OfferDesk>);

impl Global for Shared {}

/// The app's desk, made on first use.
pub fn shared(runtime: &Handle, cx: &mut App) -> Entity<OfferDesk> {
    if let Some(shared) = cx.try_global::<Shared>() {
        return shared.0.clone();
    }
    let desk = cx.new(|_| OfferDesk {
        runtime: runtime.clone(),
        source: None,
        example: access::example::Shape::default(),
        offers: Offers::default(),
        who: None,
        jobs: HashMap::new(),
        queue: Vec::new(),
        #[cfg(feature = "testing")]
        asked: Vec::new(),
    });
    cx.set_global(Shared(desk.clone()));
    desk
}

/// The app's desk if a page has made it: the Resources page does, and binds
/// it to the open cluster. A page that only reads what is offered (the
/// change page) takes it as it finds it.
pub fn existing(cx: &App) -> Option<Entity<OfferDesk>> {
    cx.try_global::<Shared>().map(|shared| shared.0.clone())
}

impl OfferDesk {
    /// How the example answers, from the debug knob.
    pub fn set_example(&mut self, shape: access::example::Shape) {
        self.example = shape;
    }

    /// Binds the desk to the access identity of the open cluster. Another
    /// identity drops every answer, every review in flight and who you are at
    /// once, and asks who you are again; the same one changes nothing. `None`
    /// is no cluster: nothing is asked.
    pub fn bind(&mut self, source: Option<&ClusterSource>, cx: &mut Context<Self>) {
        let id = source.map(|source| source.id.as_str());
        if self.source.as_ref().map(|source| source.id.as_str()) == id {
            self.source = source.cloned();
            return;
        }
        self.jobs.clear();
        self.who = None;
        self.queue.clear();
        self.source = source.cloned();
        self.offers.reset(id.unwrap_or_default());
        if let Some(source) = self.source.clone() {
            self.read_who(source, cx);
        }
        cx.emit(OfferChanged);
        cx.notify();
    }

    fn read_who(&mut self, source: ClusterSource, cx: &mut Context<Self>) {
        if source.access.is_example() {
            self.offers.set_person(Some("alice@example.test".into()));
            return;
        }
        let (job, receiver) = spawn_job(
            &self.runtime,
            DEADLINE,
            "Asking who you are timed out".to_owned(),
            async move {
                let client = source
                    .access
                    .client()
                    .await
                    .map_err(|failure| failure.message)?;
                access::who(&client).await
            },
        );
        let identity = self.source.as_ref().map(|source| source.id.clone());
        let task = cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("Asking who you are stopped unexpectedly".into()));
            _ = this.update(cx, |desk, cx| {
                // Another access since: this was somebody else's answer.
                if desk.source.as_ref().map(|source| &source.id) != identity.as_ref() {
                    return;
                }
                // A server that doesn't serve the review can't say: the
                // answers are kept for "someone", and still asked.
                desk.offers.set_person(result.ok().map(|who| who.username));
                desk.pump(cx);
                cx.emit(OfferChanged);
                cx.notify();
            });
        });
        self.who = Some((job, task));
    }

    /// Asks for the answers an action needs, if they are not known: the
    /// questions its selection declares. Cheap to call again: a question that
    /// has a fresh answer or is being asked costs nothing.
    pub fn want(&mut self, questions: &[AccessQuestion], cx: &mut Context<Self>) {
        for question in questions {
            if self.queue.len() < QUEUE && !self.queue.contains(question) {
                self.queue.push(question.clone());
            }
        }
        self.pump(cx);
    }

    fn pump(&mut self, cx: &mut Context<Self>) {
        let (Some(source), true) = (self.source.clone(), self.offers.is_ready()) else {
            return;
        };
        let now = cx.background_executor().now();
        let queue = std::mem::take(&mut self.queue);
        self.jobs
            .retain(|question, _| self.offers.is_in_flight(question));
        let tickets = self.offers.wants(&queue, now);
        // What is still unanswered and not on its way waits for a slot.
        self.queue = queue
            .into_iter()
            .filter(|question| {
                !self.offers.is_in_flight(question)
                    && self.offers.offer(std::slice::from_ref(question), now) == Offer::Pending
            })
            .collect();
        for ticket in tickets {
            #[cfg(feature = "testing")]
            self.asked.push(ticket.question.clone());
            if source.access.is_example() {
                let verdict = access::example::offer_verdict(self.example, &ticket.question);
                self.landed(&ticket, verdict, cx);
            } else {
                self.ask(source.clone(), ticket, cx);
            }
        }
    }

    fn ask(&mut self, source: ClusterSource, ticket: OfferTicket, cx: &mut Context<Self>) {
        let question = ticket.question.clone();
        let asked = question.clone();
        let (job, receiver) = spawn_job(
            &self.runtime,
            DEADLINE,
            "The review timed out".to_owned(),
            async move {
                let client = source
                    .access
                    .client()
                    .await
                    .map_err(|failure| failure.message)?;
                Ok(write::ask_access(&client, &asked).await)
            },
        );
        let task = cx.spawn(async move |this, cx| {
            let verdict = match receiver.await {
                Ok(Ok(access)) => Verdict::from(access),
                Ok(Err(message)) => Verdict::Failed(message),
                Err(_) => Verdict::Failed("The review stopped unexpectedly".into()),
            };
            _ = this.update(cx, |desk, cx| desk.landed(&ticket, verdict, cx));
        });
        self.jobs.insert(question, (job, task));
    }

    fn landed(&mut self, ticket: &OfferTicket, verdict: Verdict, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        if self.offers.land(ticket, verdict, now) {
            cx.emit(OfferChanged);
            cx.notify();
            self.pump(cx);
        }
    }

    /// What an action that needs all of `questions` reads now.
    pub fn offer(&self, questions: &[AccessQuestion], cx: &App) -> Offer {
        self.offers.offer(questions, cx.background_executor().now())
    }

    /// Reviews in flight, for tests.
    #[cfg(feature = "testing")]
    pub fn in_flight(&self) -> usize {
        self.offers.in_flight()
    }

    /// Every question asked of the cluster so far, for tests.
    #[cfg(feature = "testing")]
    pub fn asked(&self) -> &[AccessQuestion] {
        &self.asked
    }

    /// Forgets every answer, so the next `want` asks again, for tests.
    #[cfg(feature = "testing")]
    pub fn forget_answers(&mut self) {
        self.offers.forget();
        self.asked.clear();
    }
}
