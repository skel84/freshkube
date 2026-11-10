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
    teleport::{Proxy, TeleportAccess, TeleportError},
    wire::{Encryption, Target, TlsPlan, TokioPostgres},
};
use freshkube_reads::{Loader, OnShow};
use freshkube_resources::forwards::{self, ForwardSpec, ForwardView, State};
use gpui_kit::*;
use std::sync::Arc;
use tokio::runtime::Handle;

pub(super) use super::query_link::Shared;
use super::query_link::{Live, QueryRequest};

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
    /// Connected over TLS whose certificate was not checked.
    Unverified,
    /// Connected without TLS.
    Plain,
    /// Stays on the first step, as a capture of the loading state.
    Loading,
    /// Connected, and the Query tab opens with example statements run: rows,
    /// a refused write and a statement that did not run.
    Query,
    /// As `Query`, with the application's own user as the login, so the
    /// strip warns that the session is not a reader.
    QueryAppUser,
    /// Through Teleport, connected: the strip reads "Through Teleport (tsh tunnel)".
    Teleport,
    /// Teleport's states (E3), each with the command to run.
    TeleportNoTsh,
    TeleportNotLoggedIn,
    TeleportNoRole,
    TeleportNotFound,
    TeleportProxyFailed,
}

/// The invented Teleport settings the examples use.
pub(crate) fn example_teleport() -> TeleportAccess {
    TeleportAccess {
        proxy: "teleport.example.com".into(),
        name_pattern: "{name}-ro".into(),
        db_user: "reader".into(),
        database: "shop".into(),
        tsh: None,
    }
}

/// What the example Query tab opens with.
const EXAMPLE_QUERY: &str =
    "select * from orders;\nupdate orders set status = 'paid';\nselect * from customers";

impl Example {
    /// Whether the example reaches the database through Teleport.
    pub(crate) fn is_teleport(self) -> bool {
        matches!(
            self,
            Self::Teleport
                | Self::TeleportNoTsh
                | Self::TeleportNotLoggedIn
                | Self::TeleportNoRole
                | Self::TeleportNotFound
                | Self::TeleportProxyFailed
        )
    }

    /// The words `FRESHKUBE_DB_EXPLORE` takes (an empty one is `connected`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) const NAMES: [&'static str; 17] = [
        "connected",
        "no-secret",
        "forward-failed",
        "refused",
        "timed-out",
        "not-read-only",
        "unverified",
        "plain",
        "loading",
        "teleport",
        "teleport-no-tsh",
        "teleport-not-logged-in",
        "teleport-no-role",
        "teleport-not-found",
        "teleport-proxy-failed",
        "query",
        "query-app-user",
    ];

    pub(crate) fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "connected" | "" => Self::Connected,
            "no-secret" => Self::NoSecret,
            "forward-failed" => Self::ForwardFailed,
            "refused" => Self::Refused,
            "timed-out" => Self::TimedOut,
            "not-read-only" => Self::NotReadOnly,
            "unverified" => Self::Unverified,
            "plain" => Self::Plain,
            "loading" => Self::Loading,
            "teleport" => Self::Teleport,
            "teleport-no-tsh" => Self::TeleportNoTsh,
            "teleport-not-logged-in" => Self::TeleportNotLoggedIn,
            "teleport-no-role" => Self::TeleportNoRole,
            "teleport-not-found" => Self::TeleportNotFound,
            "teleport-proxy-failed" => Self::TeleportProxyFailed,
            "query" => Self::Query,
            "query-app-user" => Self::QueryAppUser,
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
    /// What protects the connection to the server.
    encryption: Encryption,
    /// Where it goes, when the route (Teleport) is known only once connected.
    route: Option<SharedString>,
    /// The Teleport child, owned by this connection: dropping the last
    /// handle, with the tab, kills it.
    #[allow(
        dead_code,
        reason = "held only so dropping the connection kills the child"
    )]
    proxy: Option<Arc<Proxy>>,
    /// The login is the application's own user, not a reader.
    app_user: bool,
}

// By hand: the session is not printable, and nothing here is a secret.
impl std::fmt::Debug for Connected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connected")
            .field("who", &self.who)
            .field("encryption", &self.encryption)
            .finish_non_exhaustive()
    }
}

struct Link {
    view: Entity<ForwardView>,
    _observer: Option<Subscription>,
}

impl EventEmitter<QueryRequest> for ExploreView {}

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
    /// Teleport's settings when the database is reached through it.
    teleport: Option<TeleportAccess>,
    /// The pod used and the local port, for the header.
    route: Option<SharedString>,
    /// Whether the pod chosen is a replica, which decides the Service name
    /// its certificate is checked against.
    replica: bool,
    /// A forward that listened while the tab was hidden; connecting waits.
    waiting_port: Option<u16>,
    browser: Entity<Browser>,
    data: Entity<DataView>,
    _browser: Subscription,
    _release: Subscription,
    /// Quitting drops the connection, and the Teleport child with it.
    _quit: Subscription,
}

impl ExploreView {
    pub(super) fn new(
        runtime: Handle,
        source: ClusterSource,
        database: Database,
        example: Option<Example>,
        teleport: Option<TeleportAccess>,
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
        let quit = cx.on_app_quit(|this, _| {
            // The app is ending: nothing else will drop the connection, so
            // drop it here, which kills the Teleport child it holds.
            this.connect.reset();
            async {}
        });
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
            teleport,
            route: None,
            replica: true,
            waiting_port: None,
            browser,
            data,
            _browser: subscription,
            _release: release,
            _quit: quit,
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
            match self.teleport.clone() {
                Some(access) => self.start_teleport(access, cx),
                None => self.start_pick(cx),
            }
        }
        cx.notify();
    }

    fn connect_example(&mut self, example: Example, cx: &mut Context<Self>) {
        if example.is_teleport() {
            return self.connect_teleport_example(example, cx);
        }
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
            Example::Connected
            | Example::Unverified
            | Example::Plain
            | Example::Query
            | Example::QueryAppUser => fake::Shape::Connected,
            Example::Refused => fake::Shape::Refused,
            Example::TimedOut => fake::Shape::TimedOut,
            Example::NotReadOnly => fake::Shape::NotReadOnly,
            _ => unreachable!("Teleport's examples are handled above"),
        };
        self.route = Some("example · no forward".into());
        match fake::connect(shape, Limits::default()) {
            Ok(session) => {
                let app_user = example == Example::QueryAppUser;
                let who = if app_user {
                    "shop_app · application user · example".into()
                } else {
                    "shop_reader · reader Secret · example".into()
                };
                let encryption = match example {
                    Example::Unverified => Encryption::Unverified,
                    Example::Plain => Encryption::Plain,
                    _ => Encryption::Verified,
                };
                let connected = Connected {
                    session: Arc::new(tokio::sync::Mutex::new(Box::new(session))),
                    who,
                    encryption,
                    route: None,
                    proxy: None,
                    app_user,
                };
                self.connect.resolve(self.epoch, Ok(Ok(connected)));
                self.connected(cx);
                if matches!(example, Example::Query | Example::QueryAppUser) {
                    self.open_query_with(Some(EXAMPLE_QUERY.to_owned()), cx);
                }
            }
            Err(error) => self.fail(Fail::of_connect(error), cx),
        }
    }

    /// Teleport's examples: connected over the tunnel, or one of its states.
    /// No `tsh` runs and nothing listens.
    fn connect_teleport_example(&mut self, example: Example, cx: &mut Context<Self>) {
        let access = self.teleport.clone().unwrap_or_else(example_teleport);
        let name = access
            .database_name(&self.database.identity.name)
            .unwrap_or_default();
        let error = match example {
            Example::TeleportNoTsh => TeleportError::NotInstalled,
            Example::TeleportNotLoggedIn => TeleportError::NotLoggedIn {
                proxy: access.proxy.clone(),
            },
            Example::TeleportNoRole => TeleportError::NoRole {
                name,
                user: access.db_user.clone(),
                proxy: access.proxy.clone(),
            },
            Example::TeleportNotFound => TeleportError::NotFound {
                name,
                pattern: access.name_pattern.clone(),
                proxy: access.proxy.clone(),
            },
            Example::TeleportProxyFailed => TeleportError::ProxyFailed {
                why: "ERROR: listen tcp 127.0.0.1:41234: bind: address already in use".into(),
                command: format!("tsh {}", access.proxy_args(&name).join(" ")),
            },
            _ => {
                let route = format!("Teleport · {name} · localhost:41234 · example");
                let session = fake::connect(fake::Shape::Connected, Limits::default())
                    .expect("the example database connects");
                let connected = Connected {
                    session: Arc::new(tokio::sync::Mutex::new(Box::new(session))),
                    who: "reader · Teleport · example".into(),
                    encryption: Encryption::Tunnel,
                    route: Some(route.into()),
                    proxy: None,
                    app_user: false,
                };
                self.connect.resolve(self.epoch, Ok(Ok(connected)));
                return self.connected(cx);
            }
        };
        self.fail(Fail::Teleport(error), cx);
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
        self.replica = choice.replica;
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
            _observer: Some(observer),
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
        let replica = self.replica;
        reading::read(
            self,
            |this: &mut Self| &mut this.connect,
            epoch,
            &runtime,
            "the database",
            async move { Ok(connect_live(access, database, port, replica).await) },
            |this, cx| this.connected(cx),
            cx,
        );
    }

    /// The Teleport route: one job checks `tsh`, the login and the name,
    /// starts the proxy and connects over it. There is no pod, forward or
    /// Secret. The proxy child goes with the connection, or with the job
    /// if it is dropped before it lands.
    fn start_teleport(&mut self, access: TeleportAccess, cx: &mut Context<Self>) {
        self.phase = Phase::Connecting;
        let cluster = self.database.identity.name.clone();
        let runtime = self.runtime.clone();
        let epoch = self.epoch;
        reading::read_within(
            reading::TELEPORT_DEADLINE,
            self,
            |this: &mut Self| &mut this.connect,
            epoch,
            &runtime,
            "the database",
            async move { Ok(connect_teleport(access, cluster).await) },
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
                if connected.route.is_some() {
                    self.route = connected.route.clone();
                }
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

    pub(super) fn encryption(&self) -> Option<Encryption> {
        self.connect
            .data()
            .and_then(|r| r.as_ref().ok())
            .map(|c| c.encryption)
    }

    /// Makes `view` the forward this tab owns, as connecting would.
    #[cfg(test)]
    pub(super) fn adopt_forward(&mut self, view: Entity<ForwardView>) {
        self.link = Some(Link {
            _observer: None,
            view,
        });
    }

    /// The connected session as the Query tab asks for it; `None` while the
    /// tab is not connected.
    pub(super) fn live(&self) -> Option<Live> {
        if self.phase != Phase::Connected {
            return None;
        }
        let connected = self.connect.data()?.as_ref().ok()?;
        Some(Live {
            session: connected.session.clone(),
            who: connected.who.clone(),
            route: self.route.clone(),
            encryption: connected.encryption,
            app_user: connected.app_user,
        })
    }

    /// Opens this database's Query tab in the dock. The request follows this
    /// tab: reconnecting is followed, and closing the tab ends it.
    pub(super) fn open_query(&mut self, cx: &mut Context<Self>) {
        self.open_query_with(None, cx);
    }

    fn open_query_with(&mut self, start: Option<String>, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        cx.emit(QueryRequest {
            start,
            key: self.database.identity.uid.clone(),
            title: self.database.identity.name.clone(),
            live: std::rc::Rc::new(move |cx| this.upgrade()?.read(cx).live()),
        });
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
    replica: bool,
) -> Result<Connected, Fail> {
    let app = explore::app_secret_name(&database.identity.name);
    let client = access
        .client()
        .await
        .map_err(|f| Fail::Cluster(format!("{}: {}", f.kind.label(), f.message)))?;
    let login = explore::read_login(&client, &database, None)
        .await
        .map_err(|e| Fail::of_explore(e, &app))?;
    // Only the CA's certificate, to check who answers; without it the
    // connection is still encrypted, and says it was not verified.
    let tls = TlsPlan {
        server_name: explore::server_name(&database, replica),
        ca_pem: explore::read_ca(&client, &database).await,
    };
    let limits = Limits::default();
    let password = Password::new(login.password());
    let (wire, encryption) = TokioPostgres::connect(
        Target {
            address: std::net::Ipv4Addr::LOCALHOST.into(),
            port,
            user: &login.user,
            database: &login.database,
        },
        &password,
        &limits,
        &tls,
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
        encryption,
        route: None,
        proxy: None,
        app_user: login.kind == explore::LoginKind::AppUser,
    })
}

/// A person's chosen `tsh` is kept as it is; with none, `find` looks (core's
/// lookup, the one a kubeconfig's login program gets, so a Dock launch finds a
/// Homebrew `tsh`) and its absolute path replaces `None`. The explorer's child
/// then runs only that path. A miss is [`Fail::TshNotFound`].
async fn with_resolved_tsh<F, Fut>(
    mut access: TeleportAccess,
    find: F,
) -> Result<TeleportAccess, Fail>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<std::path::PathBuf, freshkube_core::failure::Failure>>,
{
    if access.tsh.is_none() {
        let found = find()
            .await
            .map_err(|failure| Fail::TshNotFound(failure.message))?;
        access.tsh = Some(found);
    }
    Ok(access)
}

/// The Teleport route, off the UI thread: `tsh`'s checks, the proxy and the
/// same read-only session as ever. No Secret is read and no password exists:
/// the tunnel signs in with Teleport's certificate, which Freshkube never
/// sees. A failure after the proxy started drops it, which kills the child.
async fn connect_teleport(access: TeleportAccess, cluster: String) -> Result<Connected, Fail> {
    let access = with_resolved_tsh(access, || {
        freshkube_core::exec_plugin::locate_program(
            "Teleport database access",
            "tsh",
            "choose its file in the Teleport settings",
        )
    })
    .await?;
    let proxy = Proxy::start(&access, &cluster)
        .await
        .map_err(Fail::Teleport)?;
    // There is no cluster CA on this route; the leg is loopback to `tsh`.
    let tls = TlsPlan {
        server_name: proxy.name().to_owned(),
        ca_pem: None,
    };
    let limits = Limits::default();
    let (wire, _seen) = TokioPostgres::connect(
        Target {
            address: proxy.address(),
            port: proxy.port(),
            user: &access.db_user,
            database: &access.database,
        },
        &Password::new(""),
        &limits,
        &tls,
    )
    .await
    .map_err(Fail::of_connect)?;
    let session = Postgres::open(wire, limits)
        .await
        .map_err(Fail::of_connect)?;
    let route = format!("Teleport · {} · localhost:{}", proxy.name(), proxy.port());
    Ok(Connected {
        session: Arc::new(tokio::sync::Mutex::new(Box::new(session))),
        who: format!("{} · Teleport", access.db_user).into(),
        // What Freshkube's own leg is, not what `pg_stat_ssl` says: behind
        // Teleport that backend's client is Teleport's database agent, whose
        // hop to Postgres is Teleport's to secure. Ours is loopback to `tsh`,
        // and `tsh` carries it over its own mutual TLS.
        encryption: Encryption::Tunnel,
        route: Some(route.into()),
        proxy: Some(Arc::new(proxy)),
        app_user: false,
    })
}

fn end_words(end: &ForwardEnd) -> String {
    match end {
        ForwardEnd::Stopped => "it was stopped".into(),
        ForwardEnd::PodGone(why) | ForwardEnd::TargetDeleted(why) => why.to_string(),
        ForwardEnd::Failed(failure) => failure.to_string(),
    }
}
