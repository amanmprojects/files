//! SSH and scp through the system OpenSSH client, so `~/.ssh/config`, keys, the agent and
//! known hosts work as they do in a terminal. Every command shares one multiplexed connection
//! per host (`ControlMaster`), so a listing or a copy after the first costs no handshake.
//!
//! Remote work runs as a POSIX `sh` script fed on stdin to `sh -s`, so the remote login
//! shell (bash, zsh, fish) never parses it.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::model::{Entry, Kind};

/// How to reach one host. `dest` is a host name, address or `~/.ssh/config` alias.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Target {
    pub dest: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    /// Given to ssh through `SSH_ASKPASS` (this binary in askpass mode), never on a command line.
    pub password: Option<String>,
    /// Add the host's key to known_hosts on first contact (the user said to trust it).
    pub accept_new: bool,
    /// A private key file (`-i`), for keys not in the agent or config, such as AWS `.pem` files.
    pub identity: Option<String>,
    /// An ssh config file in place of `~/.ssh/config` (`-F`).
    pub config: Option<String>,
}

/// Environment variable that switches this binary into askpass mode.
pub const ASKPASS_ENV: &str = "FILES_ASKPASS_SECRET";

impl Target {
    /// `host`, `user@host`, `user@host:2222`, `[::1]:22`, `host:/path`, `user@host:2222/path`,
    /// `ssh://user@host:2222/path` or `sftp://…`. Returns the target and the path, if one was given.
    pub fn parse(input: &str) -> Result<(Target, Option<String>), String> {
        let s = input.trim();
        let s = s.strip_prefix("ssh://").or_else(|| s.strip_prefix("sftp://")).or_else(|| s.strip_prefix("scp://")).unwrap_or(s);
        if s.is_empty() {
            return Err("Enter a server, such as user@example.com".into());
        }
        let (user, rest) = match s.rsplit_once('@') {
            Some((u, r)) if !u.is_empty() => (Some(u.to_owned()), r),
            Some(_) => return Err("The user name before “@” is empty".into()),
            None => (None, s),
        };
        // Host, then an optional :port, then an optional path (starting with / or ~, after : or /).
        let (host, after) = if let Some(r) = rest.strip_prefix('[') {
            let end = r.find(']').ok_or("Missing “]” after the IPv6 address")?;
            (r[..end].to_owned(), &r[end + 1..])
        } else {
            let end = rest.find([':', '/']).unwrap_or(rest.len());
            (rest[..end].to_owned(), &rest[end..])
        };
        if host.is_empty() {
            return Err("The server name is empty".into());
        }
        if host.chars().any(|c| c.is_whitespace()) || host.starts_with('-') {
            return Err("That isn't a valid server name".into());
        }
        let mut port = None;
        let mut path = None;
        let mut after = after;
        if let Some(r) = after.strip_prefix(':') {
            let digits = r.find(|c: char| !c.is_ascii_digit()).unwrap_or(r.len());
            if digits > 0 {
                port = Some(r[..digits].parse::<u16>().map_err(|_| "The port must be 1–65535")?);
                after = &r[digits..];
            } else {
                after = r;
            }
        }
        if !after.is_empty() {
            let p = if after.starts_with("/~") { &after[1..] } else { after };
            path = Some(p.to_owned());
        }
        if port == Some(0) {
            return Err("The port must be 1–65535".into());
        }
        Ok((Target { dest: host, user, port, ..Target::default() }, path))
    }

    /// `-F` for this target: its own, else `$FILES_SSH_CONFIG`, else ssh's default.
    pub fn config_file(&self) -> Option<String> {
        self.config.clone().or_else(|| std::env::var("FILES_SSH_CONFIG").ok().filter(|f| !f.is_empty()))
    }

    /// The host's id and label: `user@host:port`, leaving out what wasn't given.
    pub fn id(&self) -> String {
        let host = if self.dest.contains(':') { format!("[{}]", self.dest) } else { self.dest.clone() };
        let mut s = match &self.user {
            Some(u) => format!("{u}@{host}"),
            None => host,
        };
        if let Some(p) = self.port {
            s.push_str(&format!(":{p}"));
        }
        s
    }

    fn options(&self) -> Vec<String> {
        let mut o: Vec<String> = Vec::new();
        if let Some(f) = self.config_file() {
            o.extend(["-F".into(), f]);
        }
        if let Some(i) = &self.identity {
            o.extend(["-i".into(), i.clone()]);
        }
        o.extend([
            "-o".into(),
            "ControlMaster=auto".into(),
            "-o".into(),
            format!("ControlPath={}", control_path()),
            "-o".into(),
            "ControlPersist=300".into(),
            "-o".into(),
            "ConnectTimeout=12".into(),
            "-o".into(),
            "ServerAliveInterval=15".into(),
        ]);
        if self.password.is_some() {
            o.extend(["-o".into(), "BatchMode=no".into(), "-o".into(), "NumberOfPasswordPrompts=1".into()]);
        } else {
            o.extend(["-o".into(), "BatchMode=yes".into()]);
        }
        if self.accept_new {
            o.extend(["-o".into(), "StrictHostKeyChecking=accept-new".into()]);
        }
        if let Some(u) = &self.user {
            o.extend(["-o".into(), format!("User={u}")]);
        }
        o
    }

    fn apply_env(&self, cmd: &mut Command) {
        cmd.env_remove(ASKPASS_ENV);
        if let Some(pw) = &self.password
            && let Ok(exe) = std::env::current_exe()
        {
            cmd.env("SSH_ASKPASS", exe).env("SSH_ASKPASS_REQUIRE", "force").env(ASKPASS_ENV, pw);
            // ssh only uses askpass with a display; any value works with REQUIRE=force on new
            // OpenSSH, and older ones want DISPLAY set.
            if std::env::var_os("DISPLAY").is_none() {
                cmd.env("DISPLAY", ":0");
            }
        }
    }

    /// `ssh … dest sh -s`: write a script to its stdin.
    pub fn sh(&self) -> Command {
        let mut cmd = Command::new("ssh");
        cmd.args(self.options());
        if let Some(p) = self.port {
            cmd.arg("-p").arg(p.to_string());
        }
        cmd.arg("-T").arg("--").arg(&self.dest).arg("sh -s");
        self.apply_env(&mut cmd);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd
    }

    /// `scp -r -p …` with this host's options; add sources and the destination.
    pub fn scp(&self) -> Command {
        let mut cmd = Command::new("scp");
        cmd.args(self.options());
        if let Some(p) = self.port {
            cmd.arg("-P").arg(p.to_string());
        }
        cmd.arg("-r").arg("-p").arg("-q");
        self.apply_env(&mut cmd);
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
        cmd
    }

    /// `dest:path` as scp takes it. Paths are sent over SFTP, not through a remote shell.
    pub fn remote_arg(&self, path: &str) -> String {
        let host = if self.dest.contains(':') { format!("[{}]", self.dest) } else { self.dest.clone() };
        format!("{host}:{path}")
    }

    /// Closes the shared connection (on disconnect).
    pub fn close_master(&self) {
        let mut cmd = Command::new("ssh");
        if let Some(f) = self.config_file() {
            cmd.args(["-F", &f]);
        }
        cmd.args(["-o", &format!("ControlPath={}", control_path())]);
        if let Some(u) = &self.user {
            cmd.args(["-o", &format!("User={u}")]);
        }
        if let Some(p) = self.port {
            cmd.arg("-p").arg(p.to_string());
        }
        cmd.args(["-O", "exit", "--", &self.dest]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        let _ = cmd.status();
    }
}

pub fn control_path() -> String {
    let dir = std::env::var("XDG_RUNTIME_DIR").ok().filter(|d| Path::new(d).is_dir()).unwrap_or_else(|| "/tmp".into());
    // %C is a hash of the local host, remote host, port and user: short, and unique per target.
    format!("{dir}/files-ssh-%C")
}

/// POSIX single-quoting: safe for any byte string in `sh`.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Why a remote command failed, from ssh's stderr and exit status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshError {
    /// The host isn't in known_hosts; the user can choose to trust it.
    UnknownHostKey,
    /// The host's key differs from known_hosts: possibly an attack; never offered to accept.
    ChangedHostKey(String),
    /// Keys and agent were refused, or the password was wrong.
    Auth(String),
    Other(String),
}

impl SshError {
    pub fn message(&self) -> String {
        match self {
            SshError::UnknownHostKey => "The server's host key isn't known yet.".into(),
            SshError::ChangedHostKey(m) => format!("The server's host key has changed. Refusing to connect.\n{m}"),
            SshError::Auth(m) => m.clone(),
            SshError::Other(m) => m.clone(),
        }
    }
}

pub fn classify(status: Option<i32>, stderr: &str) -> SshError {
    let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let tail = lines.iter().rev().take(3).rev().copied().collect::<Vec<_>>().join("\n");
    if stderr.contains("REMOTE HOST IDENTIFICATION HAS CHANGED") || stderr.contains("Host key for") && stderr.contains("has changed") {
        return SshError::ChangedHostKey(tail);
    }
    if stderr.contains("Host key verification failed") || stderr.contains("No ED25519 host key is known") || stderr.contains("host key is known for") {
        return SshError::UnknownHostKey;
    }
    if stderr.contains("Permission denied") || stderr.contains("Too many authentication failures") {
        return SshError::Auth(if lines.iter().any(|l| l.contains("password")) {
            "The password was refused.".into()
        } else {
            "The server refused the login. Enter a password, or add your key to the server.".into()
        });
    }
    if status == Some(255) && tail.is_empty() {
        return SshError::Other("Couldn't connect to the server.".into());
    }
    SshError::Other(if tail.is_empty() { format!("Failed (exit {})", status.unwrap_or(-1)) } else { tail })
}

/// Runs a script on the host; stdout on success.
pub fn run_script(t: &Target, script: &str) -> Result<Vec<u8>, SshError> {
    use std::io::Write;
    let mut child = t.sh().spawn().map_err(|e| SshError::Other(format!("Couldn't run ssh: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(script.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| SshError::Other(e.to_string()))?;
    if out.status.success() { Ok(out.stdout) } else { Err(classify(out.status.code(), &String::from_utf8_lossy(&out.stderr))) }
}

/// Connects (starting the shared connection) and returns the remote home folder.
pub fn connect(t: &Target) -> Result<String, SshError> {
    let out = run_script(t, "printf '%s' \"$HOME\"\n")?;
    let home = String::from_utf8_lossy(&out).trim().to_owned();
    Ok(if home.starts_with('/') { home } else { "/".into() })
}

pub fn list_script(path: &str) -> String {
    format!(
        r#"cd -- {p} || exit 3
if find . -maxdepth 0 -printf '' >/dev/null 2>&1; then
  LC_ALL=C find . -mindepth 1 -maxdepth 1 -printf '%y%Y\t%s\t%T@\t%m\t%f\0'
else
  for f in .* *; do
    case "$f" in .|..) continue;; esac
    [ -e "$f" ] || [ -L "$f" ] || continue
    if [ -d "$f" ]; then t=dd; elif [ -f "$f" ]; then t=ff; else t=ss; fi
    printf '%s\t%s\0' "$t" "$f"
  done
fi
"#,
        p = quote(path)
    )
}

/// Lists a remote folder (GNU find; a plain `sh` loop on hosts without it).
pub fn list(t: &Target, path: &str) -> Result<Vec<Entry>, SshError> {
    let out = run_script(t, &list_script(path)).map_err(|e| match e {
        SshError::Other(m) if m.contains("can't cd") || m.contains("No such file") || m.contains("cd:") => {
            SshError::Other(format!("Can't open {path}: {}", m.rsplit(':').next().unwrap_or("").trim()))
        }
        e => e,
    })?;
    Ok(parse_listing(&out))
}

pub fn parse_listing(out: &[u8]) -> Vec<Entry> {
    let mut entries = Vec::new();
    for rec in out.split(|b| *b == 0) {
        if rec.is_empty() {
            continue;
        }
        let rec = String::from_utf8_lossy(rec);
        let fields: Vec<&str> = rec.splitn(5, '\t').collect();
        let (ty, name, size, modified, mode) = match fields.as_slice() {
            [ty, size, time, mode, name] => (*ty, *name, size.parse().ok(), time.parse().ok(), u32::from_str_radix(mode, 8).ok()),
            [ty, name] => (*ty, *name, None, None, None),
            _ => continue,
        };
        let mut chars = ty.chars();
        let (own, target) = (chars.next().unwrap_or('f'), chars.next().unwrap_or('f'));
        let link = own == 'l';
        let kind = match if link { target } else { own } {
            'd' => Kind::Dir,
            'f' => Kind::File,
            _ => Kind::Other,
        };
        entries.push(Entry { name: name.to_owned(), kind, link, size: if kind == Kind::Dir { None } else { size }, modified, mode });
    }
    entries
}

/// Host aliases from `~/.ssh/config` (and files it includes), skipping wildcard patterns.
pub fn config_hosts(file: Option<std::path::PathBuf>) -> Vec<String> {
    let Some(home) = std::env::var_os("HOME") else { return Vec::new() };
    let ssh_dir = Path::new(&home).join(".ssh");
    let mut out = Vec::new();
    read_config(&file.unwrap_or_else(|| ssh_dir.join("config")), &ssh_dir, &mut out, 0);
    out
}

fn read_config(file: &Path, ssh_dir: &Path, out: &mut Vec<String>, depth: u8) {
    let Ok(text) = std::fs::read_to_string(file) else { return };
    let (hosts, includes) = parse_config(&text);
    for h in hosts {
        if !out.contains(&h) {
            out.push(h);
        }
    }
    if depth >= 4 {
        return;
    }
    for inc in includes {
        for path in expand_include(&inc, ssh_dir) {
            read_config(&path, ssh_dir, out, depth + 1);
        }
    }
}

/// The `Host` names in a config, in order, without wildcard patterns, and its `Include` arguments.
pub fn parse_config(text: &str) -> (Vec<String>, Vec<String>) {
    let (mut hosts, mut includes) = (Vec::<String>::new(), Vec::new());
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let mut words = line.split(|c: char| c.is_whitespace() || c == '=').filter(|w| !w.is_empty());
        let Some(key) = words.next() else { continue };
        if key.eq_ignore_ascii_case("host") {
            // One entry per block: its aliases name the same server. Prefer a name to an address.
            let names: Vec<&str> = words.map(|w| w.trim_matches('"')).filter(|n| !n.contains(['*', '?', '!'])).collect();
            let is_addr = |n: &&str| n.parse::<std::net::IpAddr>().is_ok();
            if let Some(name) = names.iter().find(|n| !is_addr(n)).or(names.first())
                && !hosts.iter().any(|h| h == name)
            {
                hosts.push((*name).to_owned());
            }
        } else if key.eq_ignore_ascii_case("include") {
            includes.extend(words.map(|w| w.trim_matches('"').to_owned()));
        }
    }
    (hosts, includes)
}

/// An `Include` argument: `~/` expands to home, relative paths are under `~/.ssh`, and a `*`
/// in the last component matches any file there.
fn expand_include(arg: &str, ssh_dir: &Path) -> Vec<std::path::PathBuf> {
    let path = if let Some(rest) = arg.strip_prefix("~/") {
        ssh_dir.parent().map_or_else(|| ssh_dir.join(rest), |home| home.join(rest))
    } else if arg.starts_with('/') {
        std::path::PathBuf::from(arg)
    } else {
        ssh_dir.join(arg)
    };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if !name.contains('*') {
        return vec![path];
    }
    let (prefix, suffix) = name.split_once('*').unwrap_or((&name, ""));
    let Some(dir) = path.parent() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<_> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().is_some_and(|n| {
                let n = n.to_string_lossy();
                n.len() >= prefix.len() + suffix.len() && n.starts_with(prefix) && n.ends_with(suffix)
            })
        })
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_targets() {
        let (t, p) = Target::parse("ssh://alice@example.com:2222/srv/www").unwrap();
        assert_eq!((t.user.as_deref(), t.dest.as_str(), t.port, p.as_deref()), (Some("alice"), "example.com", Some(2222), Some("/srv/www")));
        let (t, p) = Target::parse("bob@host").unwrap();
        assert_eq!((t.user.as_deref(), t.dest.as_str(), t.port, p), (Some("bob"), "host", None, None));
        let (t, p) = Target::parse("host:/etc").unwrap();
        assert_eq!((t.dest.as_str(), t.port, p.as_deref()), ("host", None, Some("/etc")));
        let (t, _) = Target::parse("[::1]:22").unwrap();
        assert_eq!((t.dest.as_str(), t.port), ("::1", Some(22)));
        assert_eq!(t.id(), "[::1]:22");
        assert!(Target::parse("").is_err());
        assert!(Target::parse("-oProxyCommand=x").is_err());
        assert!(Target::parse("h:99999").is_err());
        assert_eq!(Target::parse("user@h:2200").unwrap().0.id(), "user@h:2200");
    }

    #[test]
    fn quotes_for_sh() {
        assert_eq!(quote("it's here"), "'it'\\''s here'");
        assert_eq!(quote("$HOME `x`"), "'$HOME `x`'");
    }

    #[test]
    fn parses_find_output() {
        let out = b"dd\t4096\t1700000000.5\t755\tsrc\0ff\t12\t1700000001.0\t644\ta\tb.txt\0ld\t7\t1.0\t777\tlink\0lN\t5\t1.0\t777\tdead\0dd\tplain\0";
        let e = parse_listing(out);
        assert_eq!(e.len(), 5);
        assert_eq!((e[0].name.as_str(), e[0].kind, e[0].size), ("src", Kind::Dir, None));
        assert_eq!((e[1].name.as_str(), e[1].size, e[1].mode), ("a\tb.txt", Some(12), Some(0o644)));
        assert!(e[2].link && e[2].kind == Kind::Dir);
        assert!(e[3].link && e[3].kind == Kind::Other);
        assert_eq!((e[4].name.as_str(), e[4].kind, e[4].modified), ("plain", Kind::Dir, None));
    }

    #[test]
    fn reads_config_hosts() {
        let (hosts, inc) = parse_config("Host a b\n  HostName x\nHost *.corp !c\nhost=d\nHost 10.0.0.1 box\nInclude config.d/*\n# Host commented\n");
        assert_eq!(hosts, ["a", "d", "box"]);
        assert_eq!(inc, ["config.d/*"]);
    }

    #[test]
    fn classifies_errors() {
        assert_eq!(classify(Some(255), "Host key verification failed.\n"), SshError::UnknownHostKey);
        assert!(matches!(classify(Some(255), "u@h: Permission denied (publickey).\n"), SshError::Auth(_)));
        assert!(matches!(classify(Some(255), "@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@"), SshError::ChangedHostKey(_)));
    }
}
