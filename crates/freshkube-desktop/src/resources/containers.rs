//! How a pod's containers are named in its pickers: the pane's Logs menu,
//! the shell's Start menu and the pod log's container choice.

use freshkube_core::resources::{Container, ContainerRole, ContainerState};

/// A container picker's entry: `app · Running`, `migrate · Not started`.
pub fn choice_label(container: &Container) -> String {
    let state = match &container.state {
        ContainerState::Running(_) => "Running".to_owned(),
        ContainerState::Waiting(reason)
            if reason.is_empty()
                || (container.role == ContainerRole::Init && !container.started()) =>
        {
            "Not started".to_owned()
        }
        ContainerState::Waiting(reason) => reason.clone(),
        ContainerState::Terminated(ended) => match ended.reason.as_str() {
            "" => format!("Exited {}", ended.exit_code),
            reason => reason.to_owned(),
        },
    };
    format!("{} · {state}", container.name)
}

/// The heading over a container picker's entries of one role.
pub fn role_heading(role: ContainerRole) -> &'static str {
    match role {
        ContainerRole::Init => "INIT CONTAINERS",
        ContainerRole::App => "CONTAINERS",
        ContainerRole::Ephemeral => "EPHEMERAL",
    }
}
