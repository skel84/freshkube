use super::*;

/// Helper to create a TalosClient for testing without a real connection
fn create_test_client(nodes: Vec<String>) -> TalosClient {
    // Create a dummy channel - we won't actually use it for these tests
    // This is a bit of a hack, but it allows us to test the filtering logic
    let channel = tonic::transport::Channel::from_static("http://[::1]:50000").connect_lazy();

    TalosClient {
        channel,
        connection_id: NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed),
        nodes,
    }
}

mod decoders;
mod packet_capture;
mod targeting;
