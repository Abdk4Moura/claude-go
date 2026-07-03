use std::path::Path;
use std::time::Duration;

use crate::error::ClaudeGoError;
use crate::paths::Paths;

/// Range for the local proxy port search (inclusive). Mirrors the
/// bash v0.1.1 contract (line 44 of the reference).
pub const PORT_MIN: u16 = 4141;
pub const PORT_MAX: u16 = 4242;

/// Per-attempt HTTP timeout for the health check. 2s.
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);
/// Backoff between health-check attempts. 500ms.
pub const HEALTH_BACKOFF: Duration = Duration::from_millis(500);
/// Number of health-check attempts before giving up. 10.
pub const HEALTH_ATTEMPTS: u32 = 10;
/// Poll interval while waiting for the proxy to exit after SIGTERM.
/// 500ms.
pub const STOP_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Number of polls after SIGTERM before SIGKILL. 4 × 500ms = 2s.
pub const STOP_POLL_COUNT: u32 = 4;

/// Proxy state machine. The only invariant the rest of the code
/// depends on: `Healthy { port, pid }` means a process is actually
/// running and answering `/health` with 200, and we know which port
/// and PID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyState {
    Stopped,
    Healthy { port: u16, pid: u32 },
    /// The proxy we tried to start is not running. The string is a
    /// short human-readable reason for the UI.
    Failed(String),
}

/// Cheap handle to the on-disk proxy state files + spawn health.
/// All real I/O happens here; tests can call `start` / `stop` with
/// the same surface and assert on `ProxyState`.
pub struct Proxy<'a> {
    paths: &'a Paths,
}

impl<'a> Proxy<'a> {
    pub fn new(paths: &'a Paths) -> Self {
        Self { paths }
    }

    /// Read the current proxy state from on-disk markers. Cheap
    /// (no network), safe to call often.
    pub fn current_state(&self) -> ProxyState {
        let Some(pid) = self.read_pid() else {
            return ProxyState::Stopped;
        };
        if !is_proxy_process_alive(pid) {
            return ProxyState::Stopped;
        }
        let Some(port) = self.read_port() else {
            return ProxyState::Stopped;
        };
        ProxyState::Healthy { port, pid }
    }

    /// Start the proxy and wait for `/health` to answer 200. Returns
    /// the resulting `Healthy` state. Fails loud with
    /// `ProxyStartFailed` (and the log path) on any failure.
    pub fn start(&self, preferred_port: Option<u16>) -> Result<ProxyState, ClaudeGoError> {
        self.ensure_state_dir()?;

        // If a previous proxy is still alive, reuse it.
        if let ProxyState::Healthy { port, pid } = self.current_state() {
            return Ok(ProxyState::Healthy { port, pid });
        }

        // Pick a port: explicit > file > probe.
        let port = match preferred_port {
            Some(p) if (PORT_MIN..=PORT_MAX).contains(&p) => p,
            Some(bad) => return Err(ClaudeGoError::InvalidPort(bad as u64)),
            None => self.read_port().unwrap_or_else(|| self.probe_free_port()),
        };

        self.rotate_log_if_too_big();

        if which_opencode_api().is_none() {
            return Err(ClaudeGoError::ProxyBinaryMissing);
        }

        let pid = spawn_detached(&self.paths.proxy_log_file, port)?;
        self.write_pid(pid)?;

        // Health check: 10 attempts × 500ms backoff × 2s curl timeout.
        // Early exit if the proxy process dies.
        for _ in 0..HEALTH_ATTEMPTS {
            std::thread::sleep(HEALTH_BACKOFF);
            if !is_pid_alive(pid) {
                break;
            }
            if health_ok(port) {
                self.write_port(port)?;
                return Ok(ProxyState::Healthy { port, pid });
            }
        }

        // Failed to come up. Clean up.
        let _ = kill_process(pid);
        std::thread::sleep(Duration::from_millis(300));
        let _ = kill_process_kill(pid);
        let _ = std::fs::remove_file(&self.paths.proxy_pid_file);
        let _ = std::fs::remove_file(&self.paths.proxy_port_file);
        Err(ClaudeGoError::ProxyStartFailed {
            port,
            log: self.paths.proxy_log_file.display().to_string(),
        })
    }

    /// Stop the proxy we started. Idempotent. Does not touch the
    /// `marker_file` -- callers should manage that.
    pub fn stop(&self) -> Result<(), ClaudeGoError> {
        let Some(pid) = self.read_pid() else {
            let _ = std::fs::remove_file(&self.paths.proxy_port_file);
            return Ok(());
        };
        if is_proxy_process_alive(pid) {
            // SIGTERM
            let _ = kill_process(pid);
            for _ in 0..STOP_POLL_COUNT {
                if !is_proxy_process_alive(pid) {
                    break;
                }
                std::thread::sleep(STOP_POLL_INTERVAL);
            }
            if is_proxy_process_alive(pid) {
                // SIGKILL
                let _ = kill_process_kill(pid);
            }
        }
        let _ = std::fs::remove_file(&self.paths.proxy_pid_file);
        let _ = std::fs::remove_file(&self.paths.proxy_port_file);
        Ok(())
    }

    fn ensure_state_dir(&self) -> Result<(), ClaudeGoError> {
        std::fs::create_dir_all(&self.paths.state_dir)?;
        Ok(())
    }

    fn read_pid(&self) -> Option<u32> {
        read_trim(&self.paths.proxy_pid_file).and_then(|s| s.parse().ok())
    }

    fn read_port(&self) -> Option<u16> {
        read_trim(&self.paths.proxy_port_file).and_then(|s| s.parse().ok())
    }

    fn write_pid(&self, pid: u32) -> Result<(), ClaudeGoError> {
        std::fs::write(&self.paths.proxy_pid_file, pid.to_string())?;
        Ok(())
    }

    fn write_port(&self, port: u16) -> Result<(), ClaudeGoError> {
        std::fs::write(&self.paths.proxy_port_file, port.to_string())?;
        Ok(())
    }

    fn probe_free_port(&self) -> u16 {
        for p in PORT_MIN..=PORT_MAX {
            if is_port_free(p) {
                return p;
            }
        }
        // Fall back to PORT_MIN even if all are in use; the spawn
        // will fail and the caller will see a clear error.
        PORT_MIN
    }

    fn rotate_log_if_too_big(&self) {
        const FIVE_MIB: u64 = 5 * 1024 * 1024;
        let Ok(meta) = std::fs::metadata(&self.paths.proxy_log_file) else {
            return;
        };
        if meta.len() <= FIVE_MIB {
            return;
        }
        let rotated = self.paths.proxy_log_file.with_extension("log.old");
        let _ = std::fs::rename(&self.paths.proxy_log_file, rotated);
    }
}

fn read_trim(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

fn is_port_free(port: u16) -> bool {
    use std::net::TcpListener;
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn which_opencode_api() -> Option<std::path::PathBuf> {
    // Use `command -v` via stdlib (no shell) -- walk PATH ourselves.
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in &["opencode-api"] {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(target_family = "unix")]
fn spawn_detached(log: &Path, port: u16) -> Result<u32, ClaudeGoError> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)?;
    let log_file_err = log_file.try_clone()?;

    let mut cmd = Command::new("opencode-api");
    cmd.arg("start")
        .arg("--port")
        .arg(port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_file_err));
    // Put the child in its own process group via setsid(2) so its
    // pid == its pgid, and `stop` can signal the whole group at once
    // (parent + node workers).
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    Ok(child.id())
}

#[cfg(not(target_family = "unix"))]
fn spawn_detached(_log: &Path, _port: u16) -> Result<u32, ClaudeGoError> {
    Err(ClaudeGoError::ProxyBinaryMissing)
}

fn is_pid_alive(pid: u32) -> bool {
    #[cfg(target_family = "unix")]
    {
        // kill(pid, 0) is the portable liveness probe.
        unsafe {
            if libc::kill(pid as i32, 0) == 0 {
                return true;
            }
        }
        false
    }
    #[cfg(not(target_family = "unix"))]
    {
        let _ = pid;
        false
    }
}

#[cfg(target_family = "unix")]
fn kill_process(pid: u32) -> Result<(), ClaudeGoError> {
    // Negative pid means "send to the process group whose ID is
    // |pid|". We spawn the proxy with setsid, so the child's pid ==
    // its pgid, and killing pgid = pid kills the whole group (parent
    // + node workers).
    unsafe {
        libc::kill(-(pid as i32), libc::SIGTERM);
    }
    Ok(())
}

#[cfg(target_family = "unix")]
fn kill_process_kill(pid: u32) -> Result<(), ClaudeGoError> {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    Ok(())
}

#[cfg(not(target_family = "unix"))]
fn kill_process(_pid: u32) -> Result<(), ClaudeGoError> {
    Err(ClaudeGoError::ProxyBinaryMissing)
}

#[cfg(not(target_family = "unix"))]
fn kill_process_kill(_pid: u32) -> Result<(), ClaudeGoError> {
    Err(ClaudeGoError::ProxyBinaryMissing)
}

/// True iff `/proc/<pid>/cmdline` (Linux) or `ps` (macOS) shows
/// `opencode-api`. Critical safety: we never kill a PID that isn't
/// our own proxy, even if the PID happens to be reassigned after
/// the old process died.
fn is_proxy_process_alive(pid: u32) -> bool {
    if !is_pid_alive(pid) {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        let Ok(bytes) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            return false;
        };
        // cmdline is NUL-separated.
        let cmdline = bytes
            .split(|b| *b == 0)
            .filter_map(|s| std::str::from_utf8(s).ok())
            .collect::<Vec<_>>()
            .join(" ");
        cmdline.contains("opencode-api")
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "comm="])
            .output();
        match out {
            Ok(o) => {
                let s = String::from_utf8_lossy(&o.stdout);
                s.contains("opencode-api")
            }
            Err(_) => false,
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        false
    }
}

fn health_ok(port: u16) -> bool {
    // Synchronous HTTP probe: we deliberately do not pull in reqwest
    // for this. A 3-line TCP read is enough.
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Instant;

    let addr = format!("127.0.0.1:{port}");
    let Ok(mut stream) = TcpStream::connect_timeout(
        &addr.parse().unwrap_or_else(|_| std::net::SocketAddr::from(([127, 0, 0, 1], port))),
        HEALTH_TIMEOUT,
    ) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(HEALTH_TIMEOUT));
    let _ = stream.set_write_timeout(Some(HEALTH_TIMEOUT));
    let _ = stream.set_nodelay(true);
    let request = format!(
        "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0u8; 256];
    let start = Instant::now();
    let mut total = 0;
    while start.elapsed() < HEALTH_TIMEOUT {
        match stream.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => {
                total += n;
                if buf[..total].windows(12).any(|w| w == b"HTTP/1.1 200") {
                    return true;
                }
                if total == buf.len() {
                    break;
                }
            }
            Err(_) => return false,
        }
    }
    String::from_utf8_lossy(&buf[..total]).contains("200")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;

    fn fresh_dir(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("claude-go-proxy-test-{name}"));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn fresh_proxy_is_stopped() {
        let dir = fresh_dir("stopped");
        let paths = Paths::resolve_under(&dir);
        let proxy = Proxy::new(&paths);
        assert_eq!(proxy.current_state(), ProxyState::Stopped);
    }

    #[test]
    fn stop_is_idempotent() {
        let dir = fresh_dir("stop-idem");
        let paths = Paths::resolve_under(&dir);
        let proxy = Proxy::new(&paths);
        assert!(proxy.stop().is_ok());
        assert!(proxy.stop().is_ok());
    }

    #[test]
    fn port_probe_skips_in_use() {
        // Bind 4141, then verify probe returns something else.
        use std::net::TcpListener;
        let _held = TcpListener::bind(("127.0.0.1", 4141));
        let dir = fresh_dir("port-probe");
        let paths = Paths::resolve_under(&dir);
        let proxy = Proxy::new(&paths);
        let p = proxy.probe_free_port();
        assert_ne!(p, 4141);
    }

    #[test]
    fn invalid_port_rejected() {
        let dir = fresh_dir("bad-port");
        let paths = Paths::resolve_under(&dir);
        let proxy = Proxy::new(&paths);
        // PORT_MIN - 1 and PORT_MAX + 1 should be rejected.
        assert!(matches!(
            proxy.start(Some(4000)),
            Err(ClaudeGoError::InvalidPort(_))
        ));
        assert!(matches!(
            proxy.start(Some(5000)),
            Err(ClaudeGoError::InvalidPort(_))
        ));
    }
}
