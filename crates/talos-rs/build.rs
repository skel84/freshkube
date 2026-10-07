//! Build script for talos-rs protobuf code generation

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // tonic-build writes to Cargo's OUT_DIR, isolated per target/build.
    tonic_build::configure()
        .build_server(false) // We only need the client
        .build_client(true)
        .compile_protos(
            &[
                "proto/google/rpc/status.proto",
                "proto/common/common.proto",
                "proto/machine/machine.proto",
                "proto/time/time.proto",
            ],
            &["proto"],
        )?;

    // Tell Cargo to re-run if proto files change
    println!("cargo:rerun-if-changed=proto/");
    println!("cargo:rerun-if-env-changed=PROTOC");
    println!("cargo:rerun-if-env-changed=PROTOC_INCLUDE");

    Ok(())
}
