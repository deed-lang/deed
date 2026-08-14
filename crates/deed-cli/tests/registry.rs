use std::path::{Path, PathBuf};

use deed_lsp::{Json, json};

const NAME: &str = "io.github.deed-lang/deed";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root should be two directories up")
        .to_path_buf()
}

fn read(path: &str) -> String {
    std::fs::read_to_string(root().join(path)).unwrap_or_else(|why| panic!("{path}: {why}"))
}

#[test]
fn official_registry_metadata_describes_the_published_mcp_command() {
    let metadata = json::parse(&read("server.json")).expect("server.json should be JSON");
    assert_eq!(metadata.get("name").and_then(Json::as_str), Some(NAME));
    assert_eq!(
        metadata.get("version").and_then(Json::as_str),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(
        metadata.at(&["repository", "url"]).and_then(Json::as_str),
        Some(env!("CARGO_PKG_REPOSITORY"))
    );

    let packages = metadata
        .get("packages")
        .and_then(Json::as_array)
        .expect("server.json should describe a package");
    assert_eq!(packages.len(), 1);
    let package = &packages[0];
    assert_eq!(
        package.get("registryType").and_then(Json::as_str),
        Some("cargo")
    );
    assert_eq!(
        package.get("identifier").and_then(Json::as_str),
        Some(env!("CARGO_PKG_NAME"))
    );
    assert_eq!(
        package.get("version").and_then(Json::as_str),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(
        package.at(&["transport", "type"]).and_then(Json::as_str),
        Some("stdio")
    );

    let arguments = package
        .get("packageArguments")
        .and_then(Json::as_array)
        .expect("deed needs its mcp subcommand");
    assert_eq!(arguments.len(), 1);
    assert_eq!(
        arguments[0].get("type").and_then(Json::as_str),
        Some("positional")
    );
    assert_eq!(
        arguments[0].get("value").and_then(Json::as_str),
        Some("mcp")
    );

    assert_eq!(env!("CARGO_PKG_README"), "README.md");
    let marker = format!("mcp-name: {NAME}");
    assert_eq!(
        read("crates/deed-cli/README.md")
            .matches(marker.as_str())
            .count(),
        1
    );
}
