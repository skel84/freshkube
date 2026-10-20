//! App-lifetime summaries of completed change-page outcome reads. This
//! owner holds no charts, jobs or credentials and starts no Coroot work.
use std::collections::VecDeque;

use gpui_kit::*;

use super::{Identity, Summary};

#[derive(Default)]
pub(crate) struct SessionMemo {
    answers: VecDeque<(Identity, Summary)>,
}

struct Session(Entity<SessionMemo>);
impl Global for Session {}

pub(crate) fn session(cx: &mut App) -> Entity<SessionMemo> {
    if let Some(session) = cx.try_global::<Session>() {
        return session.0.clone();
    }
    let memo = cx.new(|_| SessionMemo::default());
    cx.set_global(Session(memo.clone()));
    memo
}

/// At most this many completed summaries survive in one app session.
const MAX_ANSWERS: usize = 256;

impl SessionMemo {
    pub(crate) fn keep(&mut self, identity: Identity, summary: Summary, cx: &mut Context<Self>) {
        let provider = identity.provider();
        self.answers.retain(|(key, _)| key.provider() == provider);
        // A completed refresh is the newest summary, too.
        if let Some(index) = self.answers.iter().position(|(key, _)| key == &identity) {
            self.answers.remove(index);
        }
        self.answers.push_back((identity, summary));
        if self.answers.len() > MAX_ANSWERS {
            self.answers.pop_front();
        }
        cx.notify();
    }

    pub(crate) fn get(&self, identity: &Identity) -> Option<&Summary> {
        self.answers
            .iter()
            .find(|(key, _)| key == identity)
            .map(|(_, summary)| summary)
    }
}
