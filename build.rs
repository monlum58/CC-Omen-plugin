fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The version CoolerControl shows is "<version>+<build>" from version.properties,
    // whose build number the pre-commit hook raises on every code change.
    println!("cargo:rerun-if-changed=version.properties");
    let props = std::fs::read_to_string("version.properties")?;
    let get = |key: &str| {
        props
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")))
            .map(|v| v.trim().to_string())
            .unwrap_or_else(|| panic!("version.properties: missing {key}"))
    };
    println!("cargo:rustc-env=CC_OMEN_VERSION={}+{}", get("version"), get("build"));

    tonic_prost_build::configure()
        .build_server(true)
        // The client is only used by the integration tests.
        .build_client(true)
        .protoc_arg("--experimental_allow_proto3_optional")
        .compile_protos(
            &[
                "proto/coolercontrol/models/v1/device.proto",
                "proto/coolercontrol/device_service/v1/device_service.proto",
            ],
            &["proto"],
        )?;
    Ok(())
}
