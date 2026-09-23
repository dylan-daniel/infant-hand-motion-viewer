use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{
    Arc, Mutex, RwLock,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

use serde::{Deserialize, Serialize};

use crate::assets;
use crate::util::WorkerQueue;

const DAEMON_FILE_NAME: &str = "viewer_daemon.py";
const REMOTE_DAEMON_DIR: &str = ".infant-hand-motion-viewer";
const REMOTE_DAEMON_PATH_REL: &str = ".infant-hand-motion-viewer/viewer_daemon.py";
const REMOTE_DAEMON_PATH: &str = "~/.infant-hand-motion-viewer/viewer_daemon.py";

const STATE_DISCONNECTED: u8 = 0;
const STATE_CONNECTING: u8 = 1;
const STATE_CONNECTED: u8 = 2;
const STATE_ERROR: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

fn u8_to_state(val: u8) -> ConnectionState {
    match val {
        STATE_CONNECTING => ConnectionState::Connecting,
        STATE_CONNECTED => ConnectionState::Connected,
        STATE_ERROR => ConnectionState::Error,
        _ => ConnectionState::Disconnected,
    }
}

fn state_to_u8(s: ConnectionState) -> u8 {
    match s {
        ConnectionState::Disconnected => STATE_DISCONNECTED,
        ConnectionState::Connecting => STATE_CONNECTING,
        ConnectionState::Connected => STATE_CONNECTED,
        ConnectionState::Error => STATE_ERROR,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteConfig {
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_python_bin")]
    pub python_bin: String,
    #[serde(default)]
    pub root_folder: String,
}

fn default_port() -> u16 {
    22
}

fn default_python_bin() -> String {
    "python3".to_string()
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: default_port(),
            python_bin: default_python_bin(),
            root_folder: String::new(),
        }
    }
}

/// A node in the remote/local file explorer tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExplorerNode {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub is_file: bool,
    #[serde(default)]
    pub children: Vec<ExplorerNode>,
    #[serde(default)]
    pub default_open: bool,
}

/// A trial's source video as downloaded from the remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteVideo {
    pub filename: String,
    pub video_hash: String,
    pub bytes: Vec<u8>,
}

struct ChildSession {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    next_id: u64,
}

/// Client managing remote daemon process and communication over SSH or local Python.
#[derive(Clone)]
pub struct RemoteClient {
    state: Arc<AtomicU8>,
    just_connected: Arc<AtomicBool>,
    config: Arc<RwLock<RemoteConfig>>,
    last_error: Arc<RwLock<String>>,
    session: Arc<Mutex<Option<ChildSession>>>,
    async_worker: Arc<WorkerQueue>,
}

impl RemoteClient {
    pub fn new() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(STATE_DISCONNECTED)),
            just_connected: Arc::new(AtomicBool::new(false)),
            config: Arc::new(RwLock::new(RemoteConfig::default())),
            last_error: Arc::new(RwLock::new(String::new())),
            session: Arc::new(Mutex::new(None)),
            async_worker: Arc::new(WorkerQueue::new()),
        }
    }

    pub fn state(&self) -> ConnectionState {
        u8_to_state(self.state.load(Ordering::SeqCst))
    }

    pub fn is_connected(&self) -> bool {
        self.state() == ConnectionState::Connected
    }

    pub fn is_connecting(&self) -> bool {
        self.state() == ConnectionState::Connecting
    }

    pub fn last_error(&self) -> String {
        self.last_error.read().map(|l| l.clone()).unwrap_or_default()
    }

    pub fn config(&self) -> RemoteConfig {
        self.config.read().map(|c| c.clone()).unwrap_or_default()
    }

    pub fn consume_just_connected(&self) -> bool {
        self.just_connected.swap(false, Ordering::SeqCst)
    }

    /// Initiate asynchronous connection on a background worker thread.
    pub fn connect_async(&self, config: RemoteConfig) {
        if self.is_connecting() {
            return;
        }
        self.set_state(ConnectionState::Connecting);
        if let Ok(mut c) = self.config.write() {
            *c = config.clone();
        }
        if let Ok(mut err) = self.last_error.write() {
            err.clear();
        }

        let client = self.clone();
        self.async_worker.submit(move || {
            let _ = client.connect_sync(&config);
        });
    }

    /// Connect synchronously to daemon (spawns child process and performs ping handshake).
    pub fn connect_sync(&self, config: &RemoteConfig) -> Result<(), String> {
        self.set_state(ConnectionState::Connecting);
        if let Ok(mut c) = self.config.write() {
            *c = config.clone();
        }
        if let Ok(mut err) = self.last_error.write() {
            err.clear();
        }

        match self.connect_internal(config) {
            Ok(()) => {
                self.set_state(ConnectionState::Connected);
                self.just_connected.store(true, Ordering::SeqCst);
                Ok(())
            }
            Err(e) => {
                if let Ok(mut err) = self.last_error.write() {
                    *err = e.clone();
                }
                self.set_state(ConnectionState::Error);
                Err(e)
            }
        }
    }

    /// Per-user directory that holds the installed daemon: `~/.infant-hand-motion-viewer`.
    fn daemon_dir() -> Option<PathBuf> {
        let home = if cfg!(windows) {
            std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
        } else {
            std::env::var_os("HOME")
        }?;
        Some(PathBuf::from(home).join(REMOTE_DAEMON_DIR))
    }

    /// Write the embedded daemon script into the user config directory and return its path.
    fn install_local_daemon() -> Result<PathBuf, String> {
        let dir = Self::daemon_dir().ok_or_else(|| "Could not locate the home directory".to_string())?;
        fs::create_dir_all(&dir).map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
        let script = dir.join(DAEMON_FILE_NAME);
        if fs::read_to_string(&script).ok().as_deref() != Some(assets::VIEWER_DAEMON) {
            fs::write(&script, assets::VIEWER_DAEMON)
                .map_err(|e| format!("Failed to write {}: {e}", script.display()))?;
        }
        Ok(script)
    }

    /// Base `ssh` invocation for `config` with the remote command still to be appended.
    fn ssh_command(config: &RemoteConfig) -> Command {
        let mut c = Command::new("ssh");
        c.arg("-T")
            .arg("-q")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("ConnectTimeout=10");
        if config.port != 22 && config.port > 0 {
            c.arg("-p").arg(config.port.to_string());
        }
        c.arg("--").arg(&config.host);
        c
    }

    /// Copy the embedded daemon script to `~/.infant-hand-motion-viewer/` on the remote host.
    fn upload_remote_daemon(config: &RemoteConfig) -> Result<(), String> {
        let mut c = Self::ssh_command(config);
        c.arg(format!(
            "mkdir -p ~/{REMOTE_DAEMON_DIR} && cat > ~/{REMOTE_DAEMON_PATH_REL}"
        ));
        c.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x08000000);
        }
        let mut child = c.spawn().map_err(|e| format!("Failed to spawn ssh: {e}"))?;
        child
            .stdin
            .take()
            .ok_or_else(|| "Failed to obtain ssh stdin".to_string())?
            .write_all(assets::VIEWER_DAEMON.as_bytes())
            .map_err(|e| format!("Failed to upload daemon script: {e}"))?;
        let out = child.wait_with_output().map_err(|e| format!("ssh failed: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "Failed to install daemon on remote host: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }

    fn connect_internal(&self, config: &RemoteConfig) -> Result<(), String> {
        let is_local = config.host.is_empty() || config.host == "localhost" || config.host == "127.0.0.1";
        if !is_local && !is_valid_ssh_host(&config.host) {
            return Err(format!("Invalid host: {:?}", config.host));
        }

        let python = if config.python_bin.is_empty() {
            "python3"
        } else {
            &config.python_bin
        };

        let mut cmd = if is_local {
            let script = Self::install_local_daemon()?;
            let mut c = Command::new(python);
            c.arg(script);
            c
        } else {
            Self::upload_remote_daemon(config)?;
            let mut c = Self::ssh_command(config);
            c.arg(format!("{python} {REMOTE_DAEMON_PATH}"));
            c
        };

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());

        let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn process: {e}"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Failed to obtain process stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Failed to obtain process stdout".to_string())?;
        let mut reader = BufReader::new(stdout);

        let ping_req = serde_json::json!({
            "id": 1,
            "cmd": "ping"
        });
        let line = format!("{}\n", ping_req);
        if let Err(e) = stdin.write_all(line.as_bytes()) {
            let _ = child.kill();
            return Err(format!("Connection failed: unable to send handshake to daemon: {e}"));
        }
        if let Err(e) = stdin.flush() {
            let _ = child.kill();
            return Err(format!("Connection failed: unable to flush handshake to daemon: {e}"));
        }

        let mut resp_line = String::new();
        if let Err(e) = reader.read_line(&mut resp_line) {
            let _ = child.kill();
            return Err(format!(
                "Connection failed: timed out or error waiting for daemon response: {e}"
            ));
        }
        if resp_line.trim().is_empty() {
            let _ = child.kill();
            return Err("Connection failed: empty response on handshake".to_string());
        }

        let ping_resp: serde_json::Value = serde_json::from_str(resp_line.trim()).map_err(|e| {
            let _ = child.kill();
            format!("Malformed JSON during handshake: {e}")
        })?;

        if ping_resp.get("status").and_then(|s| s.as_str()) != Some("ok") {
            let msg = ping_resp
                .get("message")
                .and_then(|s| s.as_str())
                .unwrap_or("Daemon returned error status on handshake");
            let _ = child.kill();
            return Err(msg.to_string());
        }

        if self.state() != ConnectionState::Connecting {
            let _ = child.kill();
            return Err("Connection canceled".to_string());
        }

        let mut session_lock = self.session.lock().unwrap();
        if let Some(mut old) = session_lock.take() {
            let _ = old.child.kill();
            let _ = old.child.wait();
        }
        *session_lock = Some(ChildSession {
            child,
            stdin,
            reader,
            next_id: 2,
        });

        Ok(())
    }

    /// Disconnect from remote host and terminate daemon process cleanly.
    pub fn disconnect(&self) {
        self.set_state(ConnectionState::Disconnected);
        let mut session_lock = self.session.lock().unwrap();
        if let Some(mut session) = session_lock.take() {
            let quit_req = serde_json::json!({
                "id": session.next_id,
                "cmd": "quit"
            });
            session.next_id += 1;
            let line = format!("{}\n", quit_req);
            let _ = session.stdin.write_all(line.as_bytes());
            let _ = session.stdin.flush();

            let _ = session.child.kill();
            let _ = session.child.wait();
        }
    }

    /// Called on the UI thread to monitor child process health and update state.
    pub fn poll(&self) {
        if self.state() != ConnectionState::Connected {
            return;
        }
        let mut session_lock = match self.session.try_lock() {
            Ok(lock) => lock,
            Err(_) => return,
        };
        if let Some(session) = session_lock.as_mut() {
            match session.child.try_wait() {
                Ok(Some(_exit_status)) => {
                    session_lock.take();
                    if let Ok(mut err) = self.last_error.write() {
                        *err = "Remote server connection closed unexpectedly.".to_string();
                    }
                    self.set_state(ConnectionState::Disconnected);
                }
                Ok(None) => {}
                Err(e) => {
                    session_lock.take();
                    if let Ok(mut err) = self.last_error.write() {
                        *err = format!("Error checking remote process status: {e}");
                    }
                    self.set_state(ConnectionState::Disconnected);
                }
            }
        }
    }

    /// Ping the daemon to ensure connection is live.
    pub fn ping(&self) -> Result<(), String> {
        let mut lock = self.session.lock().unwrap();
        let session = lock.as_mut().ok_or_else(|| "Not connected".to_string())?;
        let req_id = session.next_id;
        session.next_id += 1;

        let req = serde_json::json!({
            "id": req_id,
            "cmd": "ping"
        });
        let resp = Self::send_command(session, &req)?;
        if resp.get("status").and_then(|s| s.as_str()) == Some("ok") {
            Ok(())
        } else {
            Err("Ping failed".to_string())
        }
    }

    /// Scan directory on remote server and build a pruned tree for the explorer pane.
    pub fn scan_tree(&self, remote_root: &str) -> Result<ExplorerNode, String> {
        let mut lock = self.session.lock().unwrap();
        let session = lock.as_mut().ok_or_else(|| "Not connected".to_string())?;
        let req_id = session.next_id;
        session.next_id += 1;

        let req = serde_json::json!({
            "id": req_id,
            "cmd": "scan_tree",
            "root": remote_root,
        });

        let resp = Self::send_command(session, &req)?;
        if resp.get("status").and_then(|s| s.as_str()) != Some("ok") {
            let msg = resp
                .get("message")
                .and_then(|s| s.as_str())
                .unwrap_or("Scan failed on remote server");
            return Err(msg.to_string());
        }

        let tree_val = resp
            .get("tree")
            .ok_or_else(|| "Invalid tree response: missing 'tree'".to_string())?;
        let mut node: ExplorerNode =
            serde_json::from_value(tree_val.clone()).map_err(|e| format!("Failed to parse tree node: {e}"))?;
        node.default_open = true;
        Ok(node)
    }

    /// Fetch a remote file (such as a `.hexport` file) into memory.
    pub fn fetch_file_bytes(&self, remote_path: &str) -> Result<Vec<u8>, String> {
        let (hdr, data) = {
            let mut lock = self.session.lock().unwrap();
            let session = lock.as_mut().ok_or_else(|| "Not connected".to_string())?;
            let req_id = session.next_id;
            session.next_id += 1;

            let req = serde_json::json!({
                "id": req_id,
                "cmd": "get_file",
                "path": remote_path,
            });

            Self::send_command_binary(session, &req)?
        };

        if hdr.get("status").and_then(|s| s.as_str()) != Some("ok") {
            let msg = hdr
                .get("message")
                .and_then(|s| s.as_str())
                .unwrap_or("Failed to fetch remote file");
            return Err(msg.to_string());
        }
        Ok(data)
    }

    /// Fetch the source video of an export in one request. `Ok(None)` means the remote cannot find it.
    pub fn fetch_video(&self, remote_export_path: &str) -> Result<Option<RemoteVideo>, String> {
        let (hdr, bytes) = {
            let mut lock = self.session.lock().unwrap();
            let session = lock.as_mut().ok_or_else(|| "Not connected".to_string())?;
            let req_id = session.next_id;
            session.next_id += 1;

            let req = serde_json::json!({
                "id": req_id,
                "cmd": "get_video",
                "path": remote_export_path,
            });
            Self::send_command_binary(session, &req)?
        };

        let text = |key: &str| hdr.get(key).and_then(|s| s.as_str()).unwrap_or_default().to_string();
        match hdr.get("status").and_then(|s| s.as_str()) {
            Some("ok") => Ok(Some(RemoteVideo {
                filename: text("filename"),
                video_hash: text("video_hash"),
                bytes,
            })),
            Some("not_found") => Ok(None),
            _ => Err(hdr
                .get("message")
                .and_then(|s| s.as_str())
                .unwrap_or("Failed to fetch remote video")
                .to_string()),
        }
    }

    fn set_state(&self, s: ConnectionState) {
        self.state.store(state_to_u8(s), Ordering::SeqCst);
    }

    fn send_command(session: &mut ChildSession, req: &serde_json::Value) -> Result<serde_json::Value, String> {
        let line = format!("{}\n", req);
        session
            .stdin
            .write_all(line.as_bytes())
            .map_err(|e| format!("Failed to send request: {e}"))?;
        session
            .stdin
            .flush()
            .map_err(|e| format!("Failed to flush request: {e}"))?;

        let mut resp_line = String::new();
        session
            .reader
            .read_line(&mut resp_line)
            .map_err(|e| format!("Failed to read response line: {e}"))?;
        if resp_line.trim().is_empty() {
            return Err("Connection closed or empty response".to_string());
        }
        serde_json::from_str(resp_line.trim()).map_err(|e| format!("Malformed JSON response: {e}"))
    }

    fn send_command_binary(
        session: &mut ChildSession,
        req: &serde_json::Value,
    ) -> Result<(serde_json::Value, Vec<u8>), String> {
        let line = format!("{}\n", req);
        session
            .stdin
            .write_all(line.as_bytes())
            .map_err(|e| format!("Failed to send request: {e}"))?;
        session
            .stdin
            .flush()
            .map_err(|e| format!("Failed to flush request: {e}"))?;

        let mut hdr_line = String::new();
        session
            .reader
            .read_line(&mut hdr_line)
            .map_err(|e| format!("Failed to read header line: {e}"))?;
        if hdr_line.trim().is_empty() {
            return Err("Connection closed or empty header response".to_string());
        }
        let hdr: serde_json::Value =
            serde_json::from_str(hdr_line.trim()).map_err(|e| format!("Malformed header JSON: {e}"))?;

        if hdr.get("status").and_then(|s| s.as_str()) != Some("ok") {
            return Ok((hdr, Vec::new()));
        }

        let byte_size = hdr.get("size").and_then(|s| s.as_u64()).unwrap_or(0) as usize;
        let mut data = vec![0u8; byte_size];
        if byte_size > 0 {
            session
                .reader
                .read_exact(&mut data)
                .map_err(|e| format!("Failed reading binary payload: {e}"))?;
        }

        Ok((hdr, data))
    }
}

impl Default for RemoteClient {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RemoteClient {
    fn drop(&mut self) {
        // If this is the last reference holding the session, disconnect
        if Arc::strong_count(&self.session) == 1 {
            self.disconnect();
        }
    }
}

/// Whether `host` is safe to hand to `ssh` as its destination: no whitespace or control characters,
/// and not something `ssh` could mistake for an option.
pub fn is_valid_ssh_host(host: &str) -> bool {
    !host.is_empty() && !host.starts_with('-') && !host.chars().any(|c| c.is_whitespace() || c.is_control())
}
