//! The install scripts, held against the release that feeds them and run.
//!
//! An installer is the first thing that runs on a machine that has nothing, and
//! it is also the code least likely to be executed before somebody needs it. So
//! two claims are held here rather than described.
//!
//! The first is that the two scripts and the release workflow agree about which
//! platforms exist and what the assets are called. Those names live in three
//! files, and a platform added or dropped in one of them is a download that
//! 404s for exactly the people who have not installed the thing yet.
//!
//! The second is that the script works. `install.sh` is run against a release
//! this test builds, through `DEED_DOWNLOAD_BASE`, and asked to install a
//! binary and then to refuse the same binary once a byte of it has changed.
//! Reading a shell script and calling it correct is how #776 happened.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root should be two directories up")
        .to_path_buf()
}

fn read(relative: &str) -> String {
    std::fs::read_to_string(root().join(relative))
        .unwrap_or_else(|_| panic!("{relative} should be there"))
}

/// The platform triples `.github/workflows/release.yml` builds a binary for.
///
/// From the build matrix rather than from a list kept here, because a list kept
/// here is the second place the answer lives and this whole file exists because
/// of what happens when there are two.
fn released_targets() -> Vec<String> {
    let found: Vec<String> = read(".github/workflows/release.yml")
        .lines()
        .filter_map(|line| line.trim().strip_prefix("target: "))
        .map(|target| target.trim().to_string())
        .collect();
    assert!(
        found.len() >= 2,
        "the release matrix should name several targets, found {found:?}"
    );
    found
}

/// The triples the two scripts will actually ask a release for.
///
/// Read out of the assignments rather than by looking for the strings anywhere
/// in the file, so a triple that only appears in a comment does not count as
/// offered and a triple that is assigned cannot hide.
fn offered_targets() -> Vec<String> {
    fn assignments(text: &str, opening: &str, closing: char) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = text;
        while let Some(at) = rest.find(opening) {
            rest = &rest[at + opening.len()..];
            let Some(end) = rest.find(closing) else { break };
            found.push(rest[..end].to_string());
            rest = &rest[end..];
        }
        found
    }

    let mut found = assignments(&read("install.sh"), "target=\"", '"');
    found.extend(assignments(&read("install.ps1"), "$target = '", '\''));
    found.sort();
    found
}

/// What the release builds and what the installers ask for are one list.
///
/// Two directions, and both of them matter. A platform the release builds and
/// no script offers is a machine that is told to clone and build Rust; a
/// platform a script offers and the release does not build is a download that
/// 404s, and both land on somebody who has no compiler yet.
#[test]
fn the_platforms_the_installers_offer_are_the_ones_the_release_builds() {
    let mut released = released_targets();
    released.sort();
    assert_eq!(released, offered_targets());
}

/// One filename, spelled the same in the three places that have to agree.
///
/// The workflow writes it, and both installers refuse to proceed without it.
/// Spelling it differently in any one of them is a release that installs
/// nowhere, and the failure would land on somebody who has no compiler yet.
#[test]
fn the_checksum_list_has_one_name() {
    let name = "deed-$version-checksums.txt";
    for file in [".github/workflows/release.yml", "install.sh", "install.ps1"] {
        assert!(
            read(file).contains(name),
            "{file} does not write or read `{name}`"
        );
    }
}

#[test]
fn the_readme_points_at_a_script_that_is_there() {
    let readme = read("README.md");
    for script in ["install.sh", "install.ps1"] {
        assert!(
            readme.contains(script),
            "README.md does not mention {script}"
        );
        assert!(root().join(script).is_file(), "{script} is not there");
    }
}

#[cfg(unix)]
mod running_it {
    use super::*;
    use std::process::Command;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!("deed-install-{tag}-{nanos}"));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const VERSION: &str = "v9.9.9";

    /// The line the release workflow uses to write the checksum list.
    ///
    /// Lifted out of the workflow and run here rather than reimplemented, so
    /// the format the installer parses is the format the release produces.
    fn checksum_command() -> String {
        read(".github/workflows/release.yml")
            .lines()
            .find(|line| line.contains("sha256sum"))
            .expect("the release workflow should write a checksum list")
            .trim()
            .to_string()
    }

    fn sh(dir: &Path, script: &str) -> std::process::Output {
        Command::new("sh")
            .arg("-c")
            .arg(script)
            // `version` is set two lines above the checksum line in the
            // workflow, out of the tag. The environment stands in for that so
            // the line can be lifted rather than reimplemented.
            .env("version", VERSION)
            .current_dir(dir)
            .env("GITHUB_REF_NAME", VERSION)
            .output()
            .expect("a POSIX shell should run")
    }

    /// Builds the release a Linux machine would be offered, and returns it.
    fn a_release(at: &Path) -> PathBuf {
        let target = released_targets()
            .into_iter()
            .find(|target| target.contains("linux"))
            .expect("the release should build a Linux binary");
        let name = format!("deed-{VERSION}-{target}");

        let dist = at.join("dist");
        std::fs::create_dir_all(dist.join(&name)).unwrap();
        std::fs::write(
            dist.join(&name).join("deed"),
            "#!/bin/sh\necho \"deed 9.9.9\"\n",
        )
        .unwrap();

        // The other two assets exist so the workflow's own glob has something
        // to hash for every platform, the way it will on a real release.
        std::fs::write(dist.join(format!("deed-{VERSION}-other.zip")), "zip").unwrap();
        std::fs::write(dist.join(format!("deed-{VERSION}-other.wasm")), "wasm").unwrap();

        let packed = sh(at, &format!("cd dist && tar czf '{name}.tar.gz' '{name}'"));
        assert!(packed.status.success(), "{packed:?}");
        std::fs::remove_dir_all(dist.join(&name)).unwrap();

        let summed = sh(at, &checksum_command());
        assert!(
            summed.status.success(),
            "the workflow's checksum line failed: {}",
            String::from_utf8_lossy(&summed.stderr)
        );
        assert!(
            dist.join(format!("deed-{VERSION}-checksums.txt")).is_file(),
            "the checksum line wrote nothing"
        );

        dist
    }

    fn install(scratch: &Path, dist: &Path) -> std::process::Output {
        Command::new("sh")
            .arg(root().join("install.sh"))
            .env("DEED_VERSION", VERSION)
            .env("DEED_DOWNLOAD_BASE", format!("file://{}", dist.display()))
            .env("DEED_INSTALL_DIR", scratch.join("bin"))
            .output()
            .expect("a POSIX shell should run")
    }

    #[test]
    fn it_installs_a_binary_it_hashed() {
        let scratch = Scratch::new("green");
        let dist = a_release(&scratch.0);

        let out = install(&scratch.0, &dist);
        assert!(
            out.status.success(),
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("sha256 ok"),
            "it did not say it checked the hash"
        );

        let installed = scratch.0.join("bin").join("deed");
        assert!(installed.is_file(), "nothing was installed");

        let ran = Command::new(&installed).output().expect("it should run");
        assert_eq!(String::from_utf8_lossy(&ran.stdout).trim(), "deed 9.9.9");
    }

    #[test]
    fn a_download_that_does_not_hash_to_what_the_release_says_is_refused() {
        let scratch = Scratch::new("tampered");
        let dist = a_release(&scratch.0);

        // One byte, after the list was written. Nothing demotes this to a
        // warning, which is the whole reason the list is fetched at all.
        let asset = std::fs::read_dir(&dist)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.to_string_lossy().ends_with(".tar.gz"))
            .expect("the release should carry a tarball");
        let mut bytes = std::fs::read(&asset).unwrap();
        bytes.push(0);
        std::fs::write(&asset, bytes).unwrap();

        let out = install(&scratch.0, &dist);
        assert!(!out.status.success(), "a tampered download was installed");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("hashes to"),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !scratch.0.join("bin").join("deed").exists(),
            "it installed the binary anyway"
        );
    }

    #[test]
    fn a_release_with_no_checksum_list_is_refused() {
        let scratch = Scratch::new("unlisted");
        let dist = a_release(&scratch.0);
        std::fs::remove_file(dist.join(format!("deed-{VERSION}-checksums.txt"))).unwrap();

        let out = install(&scratch.0, &dist);
        assert!(!out.status.success(), "it installed without checking");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("checksum"),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
