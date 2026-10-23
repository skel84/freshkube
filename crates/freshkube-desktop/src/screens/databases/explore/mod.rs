//! The database explorer's tab (#948): one connection to one PostgreSQL
//! database, read-only, with a schema browser and a table's rows.
//!
//! Connecting is four steps, each a state the tab can stay in: choose a pod
//! (a ready replica by default), forward to it through the app's
//! `ForwardList`, read the login Secret once and connect, and verify the
//! session is read-only. The Secret is read inside the connect job and
//! lives only there. Each step is a `Loader`: its job and delivery task are
//! dropped when the tab hides or connects again, and an answer lands only
//! for the epoch it was asked in. Example data (`--fixture`) uses
//! `freshkube_db::fake` and no forward.
mod browser;
mod data;
mod fail;
mod reading;
#[cfg(test)]
mod tests;
mod view;

use crate::resources::ClusterSource;
use browser::Browser;
use data::DataView;
pub(super) use fail::Fail;
use freshkube_core::databases::{
    Database,
    explore::{self, POSTGRES_PORT, PodChoice},
};
use freshkube_core::resources::{ForwardEnd, ForwardTarget};
use freshkube_db::{
    Limits, Session,
    credentials::Password,
    fake,
    postgres::Postgres,
    wire::{Target, TokioPostgres},
};
use freshkube_reads::{Loader, OnShow};
use freshkube_resources::forwards::{self, ForwardSpec, ForwardView, State};
use gpui_kit::*;
use std::sync::Arc;
use tokio::runtime::Handle;

/// The session every read of this tab goes through, one at a time.
pub(super) type Shared = Arc<tokio::sync::Mutex<Box<dyn Session>>>;

pub(super) async fn session_of(shared: &Shared) -> tokio::sync::MutexGuard<'_, Box<dyn Session>> {
    shared.lock().await
}

/// How a connection ends in `--fixture`, from `FRESHKUBE_DB_EXPLORE`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Example {
    #[default]
    Connected,
    NoSecret,
    ForwardFailed,
    Refused,
    TimedOut,
    NotReadOnly,
    /// Stays on the first step, as a capture of the loading state.
    Loading,
}

impl Example {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "connected" | "" => Self::Connected,
            "no-secret" => Self::NoSecret,
            "forward-failed" => Self::ForwardFailed,
            "refused" => Self::Refused,
            "timed-out" => Self::TimedOut,
            "not-read-only" => Self::NotReadOnly,
            "loading" => Self::Loading,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Picking,
    Forwarding,
    Connecting,
    Connected,
    Failed(Fail),
}

#[derive(Clone)]
struct Connected {
    session: Shared,
    /// Who the session is, for the header: no secret in it.
    who: SharedString,
}

struct Link {
    view: Entity<ForwardView>,
    _observer: Subscription,
}

pub(super) struct ExploreView {
    runtime: Handle,
    source: ClusterSource,
    database: Database,
    example: Option<Example>,
    visible: bool,
    epoch: u64,
    phase: Phase,
    pick: Loader<Result<PodChoice, Fail>, u64>,
    connect: Loader<Result<Connected, Fail>, u64>,
    link: Option<Link>,
    /// The pod used and the local port, for the header.
    route: Option<SharedString>,
    /// A forward that listened while the tab was hidden; connecting waits.
    waiting_port: Option<u16>,
    browser: Entity<Browser>,
    data: Entity<DataView>,
    _browser: Subscription,
    _release: Subscription,
}

impl ExploreView {
    pub(super) fn new(
        runtime: Handle,
        source: ClusterSource,
        database: Database,
        example: Option<Example>,
        cx: &mut Context<Self>,
    ) -> Self {
        let browser = cx.new(|cx| Browser::new(runtime.clone(), cx));
        let data = cx.new(|cx| DataView::new(runtime.clone(), cx));
        let subscription = cx.subscribe(&browser, |this, _, open: &browser::Open, cx| {
            this.data.update(cx, |data, cx| {
                data.open(open.schema.clone(), open.table.clone(), cx)
            });
            cx.notify();
        });
        let release = cx.on_release(|this, cx| this.release_forward(cx));
        Self {
            runtime,
            source,
            database,
            example,
            visible: false,
            epoch: 0,
            phase: Phase::Picking,
            pick: Loader::default(),
            connect: Loader::default(),
            link: None,
            route: None,
            waiting_port: None,
            browser,
            data,
            _browser: subscription,
            _release: release,
        }
    }

    pub(super) fn title(&self) -> String {
        self.database.identity.name.clone()
    }

    pub(super) fn key(&self) -> &str {
        &self.database.identity.uid
    }

    #[cfg(test)]
    pub(super) fn phase(&self) -> &Phase {
        &self.phase
    }

    pub(super) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.pick.set_visible(visible, OnShow::Never, cx);
        self.connect.set_visible(visible, OnShow::Never, cx);
        self.browser.update(cx, |b, cx| b.set_visible(visible, cx));
        self.data.update(cx, |d, cx| d.set_visible(visible, cx));
        if !visible {
            return;
        }
        match self.phase {
            Phase::Picking | Phase::Connecting
                if self.link.is_none() || !self.connect.is_loading() =>
            {
                self.connect(cx)
            }
            Phase::Forwarding => {
                if let Some(port) = self.waiting_port.take() {
                    self.start_connect(port, cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Starts, or starts again, the whole connection.
    pub(super) fn connect(&mut self, cx: &mut Context<Self>) {
        self.epoch += 1;
        self.release_forward(cx);
        self.link = None;
        self.route = None;
        self.waiting_port = None;
        self.pick.reset();
        self.connect.reset();
        self.browser.update(cx, |b, cx| b.set_session(None, cx));
        self.data.update(cx, |d, cx| d.set_session(None, cx));
        self.phase = Phase::Picking;
        if let Some(example) = self.example {
            self.connect_example(example, cx);
        } else if self.visible {
            self.start_pick(cx);
        }
        cx.notify();
    }

    fn connect_example(&mut self, example: Example, cx: &mut Context<Self>) {
        let shape = match example {
            Example::Loading => {
                // A read that is never answered: the tab waits on its first step.
                self.pick.start(self.epoch);
                return;
            }
            Example::NoSecret => return self.fail(Fail::NoSecret("orders-db-app".into()), cx),
            Example::ForwardFailed => {
                return self.fail(Fail::ForwardFailed("Port 15432 is in use".into()), cx);
            }
            Example::Connected => fake::Shape::Connected,
            Example::Refused => fake::Shape::Refused,
            Example::TimedOut => fake::Shape::TimedOut,
            Example::NotReadOnly => fake::Shape::NotReadOnly,
        };
        self.route = Some("example · no forward".into());
        match fake::connect(shape, Limits::default()) {
            Ok(session) => {
                let who = "shop_reader · reader Secret · example".into();
                let connected = Connected {
                    session: Arc::new(tokio::sync::Mutex::new(Box::new(session))),
                    who,
                };
                self.connect.resolve(self.epoch, Ok(Ok(connected)));
                self.connected(cx);
            }
            Err(error) => self.fail(Fail::of_connect(error), cx),
        }
    }

    fn fail(&mut self, fail: Fail, cx: &mut Context<Self>) {
        self.phase = Phase::Failed(fail);
        cx.notify();
    }

    fn start_pick(&mut self, cx: &mut Context<Self>) {
        let access = self.source.access.clone();
        let database = self.database.clone();
        let app = explore::app_secret_name(&database.identity.name);
        let runtime = self.runtime.clone();
        let epoch = self.epoch;
        reading::read(
            self,
            |this: &mut Self| &mut this.pick,
            epoch,
            &runtime,
            "the replica",
            async move {
                let client = access
                    .client()
                    .await
                    .map_err(|f| Fail::Cluster(format!("{}: {}", f.kind.label(), f.message)));
                let picked = match client {
                    Ok(client) => explore::pick_pod(&client, &database)
                        .await
                        .map_err(|e| Fail::of_explore(e, &app)),
                    Err(fail) => Err(fail),
                };
                Ok(picked)
            },
            |this, cx| this.picked(cx),
            cx,
        );
    }

    fn picked(&mut self, cx: &mut Context<Self>) {
        let Some(result) = self.pick.data().cloned() else {
            // The deadline or a lost job: say so as a timeout.
            let error = self
                .pick
                .error()
                .map(ToString::to_string)
                .unwrap_or_default();
            return self.fail(Fail::Cluster(error), cx);
        };
        match result {
            Err(fail) => self.fail(fail, cx),
            Ok(choice) => self.start_forward(choice, cx),
        }
    }

    fn start_forward(&mut self, choice: PodChoice, cx: &mut Context<Self>) {
        self.phase = Phase::Forwarding;
        let identity = freshkube_nav::ResourceIdentity {
            connection: self.source.id.clone(),
            resource: "pods".into(),
            namespace: self.database.identity.namespace.clone(),
            name: choice.name.clone(),
            uid: choice.uid.clone(),
        };
        let spec = ForwardSpec {
            runtime: self.runtime.clone(),
            access: self.source.access.clone(),
            identity,
            target: ForwardTarget::Pod {
                name: choice.name.clone(),
                uid: choice.uid,
            },
            context: self.source.context.clone(),
            port: POSTGRES_PORT,
            local_port: None,
        };
        let list = forwards::list(cx);
        let view = list.update(cx, |list, cx| list.start(spec, cx));
        let epoch = self.epoch;
        let observer = cx.observe(&view, move |this, view, cx| {
            if this.epoch == epoch {
                this.forward_changed(&view, choice.name.clone(), choice.replica, cx);
            }
        });
        self.link = Some(Link {
            view: view.clone(),
            _observer: observer,
        });
        cx.notify();
    }

    fn forward_changed(
        &mut self,
        view: &Entity<ForwardView>,
        pod: String,
        replica: bool,
        cx: &mut Context<Self>,
    ) {
        match view.read(cx).snapshot().state {
            State::Starting => {}
            State::NoReadyPod => self.fail(Fail::ForwardFailed("No ready pod".into()), cx),
            State::Ended(end) => {
                let why = end_words(&end);
                if matches!(self.phase, Phase::Connected | Phase::Connecting) {
                    self.session_lost(cx);
                    self.fail(Fail::Unreachable(format!("The forward ended: {why}")), cx);
                } else {
                    self.fail(Fail::ForwardFailed(why), cx);
                }
            }
            State::Listening(port) => {
                if self.phase != Phase::Forwarding {
                    return;
                }
                let role = if replica { "replica" } else { "primary" };
                self.route = Some(format!("{role} {pod} · localhost:{port}").into());
                if self.visible {
                    self.start_connect(port.get(), cx);
                } else {
                    self.waiting_port = Some(port.get());
                }
            }
        }
    }

    fn start_connect(&mut self, port: u16, cx: &mut Context<Self>) {
        self.phase = Phase::Connecting;
        let access = self.source.access.clone();
        let database = self.database.clone();
        let runtime = self.runtime.clone();
        let epoch = self.epoch;
        reading::read(
            self,
            |this: &mut Self| &mut this.connect,
            epoch,
            &runtime,
            "the database",
            async move { Ok(connect_live(access, database, port).await) },
            |this, cx| this.connected(cx),
            cx,
        );
    }

    fn connected(&mut self, cx: &mut Context<Self>) {
        let Some(result) = self.connect.data().cloned() else {
            let error = self
                .connect
                .error()
                .map(ToString::to_string)
                .unwrap_or_default();
            return self.fail(Fail::Unreachable(error), cx);
        };
        match result {
            Err(fail) => self.fail(fail, cx),
            Ok(connected) => {
                self.phase = Phase::Connected;
                let session = connected.session.clone();
                self.browser
                    .update(cx, |b, cx| b.set_session(Some(session.clone()), cx));
                self.data
                    .update(cx, |d, cx| d.set_session(Some(session), cx));
                cx.notify();
            }
        }
    }

    fn session_lost(&mut self, cx: &mut Context<Self>) {
        self.browser.update(cx, |b, cx| b.set_session(None, cx));
        self.data.update(cx, |d, cx| d.set_session(None, cx));
    }

    /// Stops and takes off the list the forward this tab started.
    fn release_forward(&mut self, cx: &mut App) {
        let Some(link) = self.link.take() else {
            return;
        };
        let id = link.view.read(cx).id;
        forwards::list(cx).update(cx, |list, cx| list.remove(id, cx));
    }

    pub(super) fn who(&self) -> Option<SharedString> {
        self.connect
            .data()
            .and_then(|r| r.as_ref().ok())
            .map(|c| c.who.clone())
    }
}

/// The whole connection, off the UI thread: the cluster client, the login
/// (read once, here, and dropped with this function), the connect and the
/// read-only check. Nothing returned or logged holds the password.
async fn connect_live(
    access: freshkube_core::cluster_source::ClusterAccess,
    database: Database,
    port: u16,
) -> Result<Connected, Fail> {
    let app = explore::app_secret_name(&database.identity.name);
    let client = access
        .client()
        .await
        .map_err(|f| Fail::Cluster(format!("{}: {}", f.kind.label(), f.message)))?;
    let login = explore::read_login(&client, &database, None)
        .await
        .map_err(|e| Fail::of_explore(e, &app))?;
    let limits = Limits::default();
    let password = Password::new(login.password());
    let wire = TokioPostgres::connect(
        Target {
            host: "127.0.0.1",
            port,
            user: &login.user,
            database: &login.database,
        },
        &password,
        &limits,
    )
    .await
    .map_err(Fail::of_connect)?;
    drop(password);
    let session = Postgres::open(wire, limits)
        .await
        .map_err(Fail::of_connect)?;
    Ok(Connected {
        session: Arc::new(tokio::sync::Mutex::new(Box::new(session))),
        who: format!("{} · {}", login.user, login.kind.label()).into(),
    })
}

fn end_words(end: &ForwardEnd) -> String {
    match end {
        ForwardEnd::Stopped => "it was stopped".into(),
        ForwardEnd::PodGone(why) | ForwardEnd::TargetDeleted(why) => why.to_string(),
        ForwardEnd::Failed(failure) => failure.to_string(),
    }
}
