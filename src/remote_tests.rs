//! Against a real sshd: set `FILES_TEST_SSH_CONFIG` to an ssh config whose `testhost` logs in
//! with a key (the README's "Testing" section starts one on 127.0.0.1:2299 without root).
//! Skipped when it isn't set.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use crate::jobs::run_steps;
use crate::model::Kind;
use crate::ops::{self, End, Mode};
use crate::ssh::{self, Target};

fn target(alias: &str) -> Option<Target> {
    let config = std::env::var("FILES_TEST_SSH_CONFIG").ok()?;
    Some(Target { dest: alias.into(), config: Some(config), accept_new: true, ..Target::default() })
}

fn run(steps: Vec<crate::jobs::Step>) -> Result<(), String> {
    run_steps(steps, &AtomicBool::new(false))
}

fn transfer(src: &End, dst: &End, mode: Mode, replace: bool, stage: &std::path::Path) -> Result<(), String> {
    let t = ops::transfer(src, dst, mode, replace, stage);
    let r = run(t.steps);
    run(t.cleanup).unwrap();
    r
}

#[test]
fn ssh_round_trip() {
    let Some(t) = target("testhost") else {
        eprintln!("FILES_TEST_SSH_CONFIG not set; skipping");
        return;
    };
    let home = ssh::connect(&t).expect("connect");
    assert!(home.starts_with('/'));

    let base = format!("/tmp/files-remote-test-{}", std::process::id());
    let local = PathBuf::from(format!("/tmp/files-local-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&local);
    std::fs::create_dir_all(local.join("up dir/nested")).unwrap();
    std::fs::write(local.join("up dir/nested/it's a file.txt"), b"payload").unwrap();
    std::fs::write(local.join("single.bin"), vec![7u8; 100_000]).unwrap();

    let remote = |p: &str| End::Remote(t.clone(), format!("{base}/{p}"));
    run(vec![ops::local_script(String::new()), End::Remote(t.clone(), "/".into()).script(format!("rm -rf {base}; mkdir -p {base}\n"))]).unwrap();

    // mkdir and rename, with names a shell would mangle.
    run(vec![ops::mkdir(&remote("new $(folder)"))]).unwrap();
    run(vec![ops::rename(&remote("new $(folder)"), &remote("renamed 'x'"))]).unwrap();
    assert!(run(vec![ops::mkdir(&remote("renamed 'x'"))]).unwrap_err().contains("already exists"));

    // Upload a folder and a file with scp.
    transfer(&End::Local(local.join("up dir")), &remote("up dir"), Mode::Copy, false, &local).unwrap();
    transfer(&End::Local(local.join("single.bin")), &remote("single.bin"), Mode::Copy, false, &local).unwrap();
    let listed = ssh::list(&t, &base).unwrap();
    let mut names: Vec<_> = listed.iter().map(|e| (e.name.as_str(), e.kind, e.size)).collect();
    names.sort_by(|a, b| a.0.cmp(b.0));
    assert_eq!(names, [("renamed 'x'", Kind::Dir, None), ("single.bin", Kind::File, Some(100_000)), ("up dir", Kind::Dir, None)]);
    let nested = ssh::list(&t, &format!("{base}/up dir/nested")).unwrap();
    assert_eq!(nested[0].name, "it's a file.txt");

    // Uploading onto an existing folder without replace is refused; with replace it doesn't nest.
    assert!(transfer(&End::Local(local.join("up dir")), &remote("up dir"), Mode::Copy, false, &local).is_err());
    transfer(&End::Local(local.join("up dir")), &remote("up dir"), Mode::Copy, true, &local).unwrap();
    assert!(ssh::list(&t, &format!("{base}/up dir")).unwrap().iter().all(|e| e.name != "up dir"));

    // Copy on the host, then download.
    transfer(&remote("up dir"), &remote("up dir (copy)"), Mode::Copy, false, &local).unwrap();
    let down = local.join("down");
    std::fs::create_dir_all(&down).unwrap();
    transfer(&remote("up dir (copy)"), &End::Local(down.join("got")), Mode::Copy, false, &local).unwrap();
    assert_eq!(std::fs::read(down.join("got/nested/it's a file.txt")).unwrap(), b"payload");

    // Host to host through a staging folder: a second alias for the same server counts as
    // another machine.
    let other = target("testhost-alias").unwrap();
    let stage = local.join("stage");
    transfer(&remote("single.bin"), &End::Remote(other.clone(), format!("{base}/via-stage.bin")), Mode::Move, false, &stage).unwrap();
    assert!(!stage.exists(), "staging is cleaned up");
    let after: Vec<_> = ssh::list(&t, &base).unwrap().into_iter().map(|e| e.name).collect();
    assert!(after.contains(&"via-stage.bin".to_owned()) && !after.contains(&"single.bin".to_owned()));

    // Missing folders say so.
    let err = ssh::list(&t, &format!("{base}/nope")).unwrap_err();
    assert!(err.message().contains("nope"), "{err:?}");

    run(vec![ops::delete(&[remote("")]).unwrap()]).unwrap();
    assert!(ssh::list(&t, &base).is_err());
    std::fs::remove_dir_all(&local).unwrap();
    t.close_master();
    other.close_master();
}
