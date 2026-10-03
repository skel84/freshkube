//! Serves the canned cluster's tools over MCP and prints the URL and token,
//! for poking at the endpoint by hand.
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let server = freshkube_agent_spike::mcp::serve(
        Arc::new(freshkube_agent_spike::scenario::Cluster::checkout_crash()),
        Arc::new(|tool, place| eprintln!("read {tool}: {place:?}")),
    )
    .await
    .unwrap();
    println!("{} {}", server.url, server.token);
    tokio::signal::ctrl_c().await.unwrap();
}
