//! Results' invented transport has no listener, client, or socket. The fixture
//! pages keep their synthetic reads while the ordinary app owner models its
//! lifecycle, including an observable Starting interval and explicit Stop.
use super::ForwardSpec;
use super::example::{ExampleForward, ExampleGuard};
use freshkube_core::resources::{ForwardState, ForwardStatus, ForwardTarget};
use tokio::runtime::Handle;
use tokio::sync::watch;

pub(super) fn matches(spec: &ForwardSpec) -> bool {
    spec.access.is_example()
        && spec.identity.namespace == "acme"
        && spec.identity.name == "results"
        && spec.port == 8443
        && matches!(&spec.target, ForwardTarget::Service { name } if name == "results")
}

pub(super) fn start(runtime: &Handle) -> ExampleForward {
    let (status, receiver) = watch::channel(ForwardStatus {
        state: ForwardState::Listening {
            pod: "results-example-0".into(),
            port: 8443,
        },
        connections: 0,
        last_error: None,
    });
    ExampleForward {
        // A synthetic route number for the fixture's typed state, never bound
        // or advertised as an address. Fixture Results reads stay synthetic.
        local_port: 18443,
        status: receiver,
        guard: ExampleGuard {
            stop: None,
            task: Some(runtime.spawn(async {})),
            memory: Some(status),
        },
    }
}
