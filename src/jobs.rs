//! Background work: each job runs its steps in order on its own thread (a `cp`, an `scp`, a
//! remote script), and the UI hears back through a channel and the window's waker.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use rune::Waker;

use crate::model::Location;
use crate::ssh;

/// Once a download or extraction lands: open the file, or browse the archive.
#[derive(Clone, Debug)]
pub enum After {
    Open(PathBuf),
    Browse(Location),
}

#[allow(clippy::large_enum_variant)]
pub enum Step {
    /// A program; `stdin` is written to it first (a remote script).
    Cmd { cmd: Command, stdin: Option<String> },
    /// Work in this process (making a staging folder).
    Func(Box<dyn FnOnce() -> Result<(), String> + Send>),
}

impl Step {
    pub fn cmd(cmd: Command) -> Step {
        Step::Cmd { cmd, stdin: None }
    }
    pub fn remote(t: &ssh::Target, script: String) -> Step {
        let mut cmd = t.sh();
        cmd.stdout(std::process::Stdio::null());
        Step::Cmd { cmd, stdin: Some(script) }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    Upload,
    Download,
    Local,
    Remote,
}

#[derive(Clone, PartialEq, Debug)]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed(String),
    Cancelled,
}

impl JobState {
    pub fn finished(&self) -> bool {
        matches!(self, JobState::Done | JobState::Failed(_) | JobState::Cancelled)
    }
}

pub struct Job {
    pub id: u64,
    pub title: String,
    /// The title once it has succeeded ("Downloaded a.txt" for "Downloading a.txt").
    pub done_title: Option<String>,
    pub detail: String,
    pub direction: Direction,
    /// Shown in the transfers pane (copies and moves); other work only reports failures.
    pub transfer: bool,
    pub state: JobState,
    /// Folders to reload when it ends.
    pub refresh: Vec<Location>,
    /// What to do when it succeeds.
    pub after: Option<After>,
    steps: Vec<Step>,
    cleanup: Vec<Step>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    pub fn new(title: impl Into<String>, direction: Direction, steps: Vec<Step>) -> Job {
        Job {
            id: 0,
            title: title.into(),
            done_title: None,
            detail: String::new(),
            direction,
            transfer: false,
            state: JobState::Queued,
            refresh: Vec::new(),
            after: None,
            steps,
            cleanup: Vec::new(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = d.into();
        self
    }
    pub fn done_title(mut self, t: impl Into<String>) -> Self {
        self.done_title = Some(t.into());
        self
    }
    pub fn transfer(mut self) -> Self {
        self.transfer = true;
        self
    }
    pub fn refresh(mut self, loc: Location) -> Self {
        if !self.refresh.contains(&loc) {
            self.refresh.push(loc);
        }
        self
    }
    pub fn cleanup(mut self, steps: Vec<Step>) -> Self {
        self.cleanup = steps;
        self
    }
    pub fn then(mut self, a: After) -> Self {
        self.after = Some(a);
        self
    }
}

/// At most this many jobs run at once: OpenSSH servers allow 10 sessions per connection.
const PARALLEL: usize = 4;

pub struct Jobs {
    pub list: Vec<Job>,
    next: u64,
    tx: Sender<(u64, Result<(), String>)>,
    rx: Receiver<(u64, Result<(), String>)>,
    waker: Option<Waker>,
}

impl Default for Jobs {
    fn default() -> Self {
        let (tx, rx) = channel();
        Jobs { list: Vec::new(), next: 1, tx, rx, waker: None }
    }
}

impl Jobs {
    pub fn set_waker(&mut self, w: Waker) {
        self.waker = Some(w);
    }

    pub fn push(&mut self, mut job: Job) -> u64 {
        job.id = self.next;
        self.next += 1;
        let id = job.id;
        self.list.push(job);
        self.start_queued();
        id
    }

    pub fn cancel(&mut self, id: u64) {
        if let Some(j) = self.list.iter_mut().find(|j| j.id == id) {
            match j.state {
                JobState::Queued => j.state = JobState::Cancelled,
                JobState::Running => j.cancel.store(true, Ordering::SeqCst),
                _ => {}
            }
        }
    }

    pub fn clear_finished(&mut self) {
        self.list.retain(|j| !j.state.finished());
    }

    pub fn running(&self) -> usize {
        self.list.iter().filter(|j| j.state == JobState::Running).count()
    }

    pub fn active(&self) -> usize {
        self.list.iter().filter(|j| matches!(j.state, JobState::Running | JobState::Queued)).count()
    }

    /// Collects finished jobs and starts queued ones; returns the jobs that just ended.
    pub fn poll(&mut self) -> Vec<u64> {
        let mut ended = Vec::new();
        while let Ok((id, result)) = self.rx.try_recv() {
            if let Some(j) = self.list.iter_mut().find(|j| j.id == id) {
                j.state = match result {
                    Ok(()) => JobState::Done,
                    Err(_) if j.cancel.load(Ordering::SeqCst) => JobState::Cancelled,
                    Err(e) => JobState::Failed(e),
                };
                ended.push(id);
            }
        }
        if !ended.is_empty() {
            self.start_queued();
        }
        ended
    }

    fn start_queued(&mut self) {
        let mut running = self.running();
        for j in self.list.iter_mut() {
            if running >= PARALLEL {
                break;
            }
            if j.state != JobState::Queued {
                continue;
            }
            j.state = JobState::Running;
            running += 1;
            let steps = std::mem::take(&mut j.steps);
            let cleanup = std::mem::take(&mut j.cleanup);
            let (id, tx, waker, cancel) = (j.id, self.tx.clone(), self.waker.clone(), j.cancel.clone());
            std::thread::Builder::new()
                .name(format!("job-{id}"))
                .spawn(move || {
                    let result = run_steps(steps, &cancel);
                    let _ = run_steps(cleanup, &Arc::new(AtomicBool::new(false)));
                    let _ = tx.send((id, result));
                    if let Some(w) = waker {
                        w();
                    }
                })
                .ok();
        }
    }
}

pub fn run_steps(steps: Vec<Step>, cancel: &AtomicBool) -> Result<(), String> {
    for step in steps {
        if cancel.load(Ordering::SeqCst) {
            return Err("Cancelled".into());
        }
        match step {
            Step::Func(f) => f()?,
            Step::Cmd { mut cmd, stdin } => run_cmd(&mut cmd, stdin, cancel)?,
        }
    }
    Ok(())
}

fn run_cmd(cmd: &mut Command, stdin: Option<String>, cancel: &AtomicBool) -> Result<(), String> {
    // Its own process group, so cancelling stops scp and the ssh it started.
    cmd.process_group(0);
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't run {program}: {e}"))?;
    if let Some(text) = stdin
        && let Some(mut pipe) = child.stdin.take()
    {
        let _ = pipe.write_all(text.as_bytes());
    }
    drop(child.stdin.take());
    let stderr = child.stderr.take().map(|mut e| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = e.read_to_end(&mut buf);
            buf
        })
    });
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if cancel.load(Ordering::SeqCst) {
            let _ = Command::new("kill").args(["-TERM", "--", &format!("-{}", child.id())]).status();
            let _ = child.wait();
            return Err("Cancelled".into());
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    if status.success() {
        return Ok(());
    }
    let err = stderr.and_then(|h| h.join().ok()).unwrap_or_default();
    let err = String::from_utf8_lossy(&err);
    Err(match program.as_str() {
        "ssh" | "scp" => ssh::classify(status.code(), &err).message(),
        _ => {
            let tail = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_owned();
            if tail.is_empty() { format!("{program} failed (exit {})", status.code().unwrap_or(-1)) } else { tail }
        }
    })
}
