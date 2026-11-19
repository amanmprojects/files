//! File operations as job steps. Work on one machine is a short POSIX script run there (`sh -s`
//! locally, or over ssh); work between machines is `scp`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::jobs::{Direction, Step};
use crate::model::posix_join;
use crate::ssh::{Target, quote};

/// One file or folder, on this machine or a host.
#[derive(Clone, Debug, PartialEq)]
pub enum End {
    Local(PathBuf),
    Remote(Target, String),
}

impl End {
    pub fn name(&self) -> String {
        match self {
            End::Local(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into()),
            End::Remote(_, p) => p.trim_end_matches('/').rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("/").to_owned(),
        }
    }

    pub fn path(&self) -> String {
        match self {
            End::Local(p) => p.to_string_lossy().into_owned(),
            End::Remote(_, p) => p.clone(),
        }
    }

    /// The same place, `name` inside it.
    pub fn join(&self, name: &str) -> End {
        match self {
            End::Local(p) => End::Local(p.join(name)),
            End::Remote(t, p) => End::Remote(t.clone(), posix_join(p, name)),
        }
    }

    pub fn label(&self) -> String {
        match self {
            End::Local(p) => p.display().to_string(),
            End::Remote(t, p) => format!("{}:{}", t.id(), p),
        }
    }

    fn same_machine(&self, other: &End) -> bool {
        match (self, other) {
            (End::Local(_), End::Local(_)) => true,
            (End::Remote(a, _), End::Remote(b, _)) => a.dest == b.dest && a.user == b.user && a.port == b.port,
            _ => false,
        }
    }

    /// A script run on this end's machine.
    pub fn script(&self, script: String) -> Step {
        match self {
            End::Local(_) => local_script(script),
            End::Remote(t, _) => Step::remote(t, script),
        }
    }
}

pub fn local_script(script: String) -> Step {
    let mut cmd = Command::new("sh");
    cmd.arg("-s").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    Step::Cmd { cmd, stdin: Some(script) }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Copy,
    Move,
}

/// Fails the script if `p` exists (a file, folder or link, even a dangling one).
fn refuse_existing(p: &str) -> String {
    format!("if [ -e {q} ] || [ -L {q} ]; then echo \"“{n}” already exists\" >&2; exit 17; fi\n", q = quote(p), n = p.rsplit('/').next().unwrap_or(p))
}

pub struct Transfer {
    pub steps: Vec<Step>,
    pub cleanup: Vec<Step>,
    pub direction: Direction,
}

/// Copies or moves `src` to `dst` (the full destination path, which must not exist unless
/// `replace`, which removes it first). `stage` is a private folder for host-to-host copies.
pub fn transfer(src: &End, dst: &End, mode: Mode, replace: bool, stage: &Path) -> Transfer {
    let mut steps = Vec::new();
    let mut cleanup = Vec::new();
    let clear_dst = |steps: &mut Vec<Step>| {
        let d = dst.path();
        let script = if replace { format!("rm -rf -- {}\n", quote(&d)) } else { refuse_existing(&d) };
        steps.push(dst.script(script));
    };
    let direction = match (src, dst) {
        (End::Local(_), End::Local(_)) => Direction::Local,
        (End::Local(_), End::Remote(..)) => Direction::Upload,
        (End::Remote(..), End::Local(_)) => Direction::Download,
        (End::Remote(..), End::Remote(..)) => Direction::Remote,
    };
    if src.same_machine(dst) {
        let (s, d) = (quote(&src.path()), quote(&dst.path()));
        let head = if replace { format!("rm -rf -- {d}\n") } else { refuse_existing(&dst.path()) };
        let body = match mode {
            Mode::Copy => format!("cp -a -- {s} {d}\n"),
            Mode::Move => format!("mv -- {s} {d}\n"),
        };
        steps.push(src.script(format!("set -e\n{head}{body}")));
        return Transfer { steps, cleanup, direction };
    }
    clear_dst(&mut steps);
    match (src, dst) {
        (End::Local(s), End::Remote(t, d)) => {
            let mut cmd = t.scp();
            cmd.arg("--").arg(s).arg(t.remote_arg(d));
            steps.push(Step::cmd(cmd));
        }
        (End::Remote(t, s), End::Local(d)) => {
            let mut cmd = t.scp();
            cmd.arg("--").arg(t.remote_arg(s)).arg(d);
            steps.push(Step::cmd(cmd));
        }
        (End::Remote(a, s), End::Remote(b, d)) => {
            // Through this machine: hosts rarely reach each other, and each has its own login.
            let staged = stage.join(src.name());
            let dir = stage.to_path_buf();
            steps.push(Step::Func(Box::new(move || std::fs::create_dir_all(&dir).map_err(|e| e.to_string()))));
            let mut down = a.scp();
            down.arg("--").arg(a.remote_arg(s)).arg(&staged);
            steps.push(Step::cmd(down));
            let mut up = b.scp();
            up.arg("--").arg(&staged).arg(b.remote_arg(d));
            steps.push(Step::cmd(up));
            let dir = stage.to_path_buf();
            cleanup.push(Step::Func(Box::new(move || {
                let _ = std::fs::remove_dir_all(&dir);
                Ok(())
            })));
        }
        (End::Local(_), End::Local(_)) => unreachable!("same machine"),
    }
    if mode == Mode::Move {
        steps.push(src.script(format!("rm -rf -- {}\n", quote(&src.path()))));
    }
    Transfer { steps, cleanup, direction }
}

pub fn mkdir(at: &End) -> Step {
    at.script(format!("set -e\n{}mkdir -- {}\n", refuse_existing(&at.path()), quote(&at.path())))
}

/// An empty file; fails if the name is taken.
pub fn touch(at: &End) -> Step {
    at.script(format!("set -e\n{}: > {}\n", refuse_existing(&at.path()), quote(&at.path())))
}

pub fn rename(from: &End, to: &End) -> Step {
    from.script(format!("set -e\n{}mv -- {} {}\n", refuse_existing(&to.path()), quote(&from.path()), quote(&to.path())))
}

/// Deletes for good. Every item must be on the same machine.
pub fn delete(items: &[End]) -> Option<Step> {
    let first = items.first()?;
    let args: Vec<String> = items.iter().map(|e| quote(&e.path())).collect();
    Some(first.script(format!("rm -rf -- {}\n", args.join(" "))))
}

/// Moves local items to the desktop's trash (`gio trash`, the freedesktop.org trash spec).
pub fn trash(items: &[PathBuf]) -> Step {
    let mut cmd = Command::new("gio");
    cmd.arg("trash").arg("--").args(items).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    Step::cmd(cmd)
}

/// Where a remote file is downloaded to be opened: `~/.cache/files/open/<host>/<path>`.
pub fn open_cache_path(cache: &Path, t: &Target, remote_path: &str) -> PathBuf {
    let host: String = t.id().chars().map(|c| if c.is_ascii_alphanumeric() || "@.-_".contains(c) { c } else { '_' }).collect();
    cache.join("open").join(host).join(remote_path.trim_start_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::run_steps;
    use std::sync::atomic::AtomicBool;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("files-ops-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn run(t: Transfer) -> Result<(), String> {
        let no = AtomicBool::new(false);
        let r = run_steps(t.steps, &no);
        run_steps(t.cleanup, &no).unwrap();
        r
    }

    #[test]
    fn local_copy_move_and_conflicts() {
        let d = tmp("local");
        std::fs::create_dir_all(d.join("src/inner")).unwrap();
        std::fs::write(d.join("src/inner/f it's.txt"), b"x").unwrap();
        std::fs::create_dir_all(d.join("dst")).unwrap();
        let src = End::Local(d.join("src"));
        let dst = End::Local(d.join("dst/src"));
        run(transfer(&src, &dst, Mode::Copy, false, &d)).unwrap();
        assert!(d.join("dst/src/inner/f it's.txt").is_file());
        // A second copy refuses to overwrite...
        let err = run(transfer(&src, &dst, Mode::Copy, false, &d)).unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        // ...and replaces when told, rather than nesting a copy inside.
        run(transfer(&src, &dst, Mode::Copy, true, &d)).unwrap();
        assert!(!d.join("dst/src/src").exists());
        run(transfer(&src, &End::Local(d.join("moved")), Mode::Move, false, &d)).unwrap();
        assert!(!d.join("src").exists() && d.join("moved/inner").is_dir());
        let no = AtomicBool::new(false);
        run_steps(vec![mkdir(&End::Local(d.join("new $dir")))], &no).unwrap();
        assert!(d.join("new $dir").is_dir());
        run_steps(vec![rename(&End::Local(d.join("new $dir")), &End::Local(d.join("renamed")))], &no).unwrap();
        assert!(d.join("renamed").is_dir());
        assert!(run_steps(vec![rename(&End::Local(d.join("renamed")), &End::Local(d.join("moved")))], &no).is_err());
        run_steps(vec![delete(&[End::Local(d.join("renamed")), End::Local(d.join("moved"))]).unwrap()], &no).unwrap();
        assert!(!d.join("renamed").exists() && !d.join("moved").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
