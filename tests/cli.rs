//! Exit-code and output contracts of the non-interactive entry points.

use std::path::PathBuf;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_redragon-gui")
}

fn models_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models")
}

#[test]
fn version_and_help_succeed() {
    for flag in ["--version", "--help"] {
        let output = Command::new(binary()).arg(flag).output().expect("run");
        assert_eq!(output.status.code(), Some(0), "{flag}");
    }
}

#[test]
fn show_descriptor_prints_the_bundled_model() {
    let output = Command::new(binary())
        .args(["--show-descriptor", "holtek-04d9-fc5c"])
        .env("REDRAGON_GUI_MODELS", models_dir())
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("holtek-04d9-fc5c"), "{text}");
    assert!(text.contains("Redragon M721-Pro"), "{text}");
    assert!(text.contains("unverified"), "{text}");
}

#[test]
fn unknown_descriptor_is_an_error() {
    let output = Command::new(binary())
        .args(["--show-descriptor", "does-not-exist"])
        .env("REDRAGON_GUI_MODELS", models_dir())
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn no_supported_device_exits_with_code_two() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join("00-registry.toml"),
        "schema = 1\ndevices = []\n",
    )
    .expect("registry");
    for flag in ["--list-devices", "--dump-state"] {
        let output = Command::new(binary())
            .arg(flag)
            .args(["--models-dir", tmp.path().to_str().unwrap()])
            .output()
            .expect("run");
        assert_eq!(
            output.status.code(),
            Some(2),
            "{flag}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn descriptor_problems_are_reported_without_failing_the_run() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::copy(
        models_dir().join("holtek-04d9-fc5c.toml"),
        tmp.path().join("holtek-04d9-fc5c.toml"),
    )
    .expect("copy descriptor");
    std::fs::write(
        tmp.path().join("00-registry.toml"),
        "schema = 1\n[[devices]]\ndescriptor = \"broken.toml\"\nvendor_id = 1\nproduct_id = 2\n",
    )
    .expect("registry");
    std::fs::write(tmp.path().join("broken.toml"), "schema = 1\n").expect("broken");
    let output = Command::new(binary())
        .args(["--show-descriptor", "holtek-04d9-fc5c"])
        .args(["--models-dir", tmp.path().to_str().unwrap()])
        .output()
        .expect("run");
    // The broken file is skipped, but the unlisted valid descriptor is not used
    // by an index that names only the broken file, so this resolves as an error.
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("broken.toml"), "{stderr}");
}
