use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use serde_json::{Value, json};

/// One request waiting for its reply: the header line and, for file transfers, the bytes of its chunk frames.
#[derive(Default)]
struct Pending {
    state: Mutex<PendingState>,
    done: Condvar,
}

#[derive(Default)]
struct PendingState {
    header: Option<Value>,
    expected: usize,
    data: Vec<u8>,
    outcome: Option<Result<(), String>>,
}

impl Pending {
    fn finish(&self, outcome: Result<(), String>) {
        let mut state = self.state.lock().unwrap();
        if state.outcome.is_none() {
            state.outcome = Some(outcome);
            self.done.notify_all();
        }
    }
}

/// Many requests in flight over one daemon pipe. A reader thread sorts the incoming frames by request id, so a
/// small reply can arrive between the chunks of a large download instead of waiting behind all of it.
pub struct Mux {
    writer: Mutex<Box<dyn Write + Send>>,
    pending: Mutex<HashMap<u64, Arc<Pending>>>,
    next_id: AtomicU64,
    alive: AtomicBool,
}

impl Mux {
    /// Start reading `reader` on a thread of its own; `writer` carries the requests.
    pub fn start(writer: impl Write + Send + 'static, reader: impl BufRead + Send + 'static) -> Arc<Self> {
        let mux = Arc::new(Self {
            writer: Mutex::new(Box::new(writer)),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            alive: AtomicBool::new(true),
        });
        let reading = Arc::clone(&mux);
        std::thread::Builder::new()
            .name("remote_reader".to_string())
            .spawn(move || {
                let why = reading.read_frames(reader);
                reading.fail_all(&why);
            })
            .expect("failed to start the remote reader thread");
        mux
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// Send `request` (an object without an id) and wait for the reply: its header and any file bytes.
    pub fn call(&self, mut request: Value) -> Result<(Value, Vec<u8>), String> {
        if !self.is_alive() {
            return Err("Connection closed".to_string());
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        request["id"] = json!(id);
        let pending = Arc::new(Pending::default());
        self.pending.lock().unwrap().insert(id, Arc::clone(&pending));
        // The connection may have died after the check above and before the request was registered
        if !self.is_alive() {
            self.pending.lock().unwrap().remove(&id);
            return Err("Connection closed".to_string());
        }
        if let Err(err) = self.write_line(&request) {
            self.pending.lock().unwrap().remove(&id);
            return Err(err);
        }
        let mut state = pending.state.lock().unwrap();
        while state.outcome.is_none() {
            state = pending.done.wait(state).unwrap();
        }
        self.pending.lock().unwrap().remove(&id);
        match state.outcome.take().unwrap() {
            Ok(()) => Ok((
                state.header.take().unwrap_or(Value::Null),
                std::mem::take(&mut state.data),
            )),
            Err(err) => Err(err),
        }
    }

    /// Send `request` without waiting for an answer.
    pub fn notify(&self, mut request: Value) {
        request["id"] = json!(self.next_id.fetch_add(1, Ordering::SeqCst));
        let _ = self.write_line(&request);
    }

    /// Give up on the connection: every waiting request fails.
    pub fn shutdown(&self) {
        self.fail_all("Connection closed");
    }

    fn write_line(&self, value: &Value) -> Result<(), String> {
        let mut writer = self.writer.lock().unwrap();
        writer
            .write_all(format!("{value}\n").as_bytes())
            .and_then(|()| writer.flush())
            .map_err(|e| format!("Failed to send request: {e}"))
    }

    fn fail_all(&self, why: &str) {
        self.alive.store(false, Ordering::SeqCst);
        let all: Vec<_> = self.pending.lock().unwrap().values().cloned().collect();
        for pending in all {
            pending.finish(Err(why.to_string()));
        }
    }

    fn pending_for(&self, id: u64) -> Option<Arc<Pending>> {
        self.pending.lock().unwrap().get(&id).cloned()
    }

    /// Route frames to their requests until the pipe closes; returns why it stopped.
    fn read_frames(&self, mut reader: impl BufRead) -> String {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => return "Remote server connection closed.".to_string(),
                Ok(_) => {}
                Err(e) => return format!("Failed to read from the remote server: {e}"),
            }
            let Ok(frame) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            let Some(id) = frame.get("id").and_then(Value::as_u64) else {
                continue;
            };
            if let Some(len) = frame.get("chunk").and_then(Value::as_u64) {
                let mut chunk = vec![0u8; len as usize];
                if let Err(e) = reader.read_exact(&mut chunk) {
                    return format!("Failed reading binary payload: {e}");
                }
                if let Some(pending) = self.pending_for(id) {
                    let mut state = pending.state.lock().unwrap();
                    state.data.extend_from_slice(&chunk);
                    if state.data.len() >= state.expected {
                        drop(state);
                        pending.finish(Ok(()));
                    }
                }
                // The daemon sends the next chunk only once this one is acknowledged, even if nobody is waiting
                let _ = self.write_line(&json!({ "cmd": "ack", "id": id }));
                continue;
            }
            let Some(pending) = self.pending_for(id) else {
                continue;
            };
            let status = frame.get("status").and_then(Value::as_str).unwrap_or_default();
            let has_header = pending.state.lock().unwrap().header.is_some();
            if has_header && status == "error" {
                let message = frame
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Transfer failed");
                pending.finish(Err(message.to_string()));
                continue;
            }
            let expected = if status == "ok" {
                frame.get("size").and_then(Value::as_u64).unwrap_or(0) as usize
            } else {
                0
            };
            {
                let mut state = pending.state.lock().unwrap();
                state.header = Some(frame);
                state.expected = expected;
                state.data.reserve(expected);
            }
            if expected == 0 {
                pending.finish(Ok(()));
            }
        }
    }
}
