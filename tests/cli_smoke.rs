use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;

fn hal() -> Command {
    Command::cargo_bin("hal").unwrap()
}

#[test]
fn build_nonexistent_file_errors_clearly() {
    hal()
        .arg("build")
        .arg("definitely_missing.hal")
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
}

#[test]
fn help_exits_zero() {
    hal().arg("--help").assert().success();
}

#[test]
fn version_exits_zero() {
    hal().arg("--version").assert().success();
}

/// The layout side-effect is exercised directly against the library so the
/// offline test suite doesn't need a live LLM. `hal build` itself is
/// covered by the ignored live test in `live_xai.rs`.
#[test]
fn project_layout_is_created_for_real_source() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("demo.hal");
    std::fs::write(&src, b"print \"hi\"\n").unwrap();

    let p = hal::project::Project::open(&src).unwrap();
    p.ensure_layout().unwrap();

    assert!(p.artifacts_dir().is_dir());
    assert!(p.runs_dir().is_dir());
    assert!(p.memory_db().parent().unwrap().is_dir());
}
