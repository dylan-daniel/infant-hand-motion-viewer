use std::collections::VecDeque;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};

type Task = Box<dyn FnOnce() + Send + 'static>;

struct WorkerShared {
    state: Mutex<WorkerState>,
    cv: Condvar,
    stop: AtomicBool,
}

struct WorkerState {
    tasks: VecDeque<Task>,
}

/// A single background thread that runs submitted tasks in FIFO order.
pub struct WorkerQueue {
    shared: Arc<WorkerShared>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl WorkerQueue {
    /// Create a new worker queue and spawn its worker thread.
    pub fn new() -> Self {
        let shared = Arc::new(WorkerShared {
            state: Mutex::new(WorkerState { tasks: VecDeque::new() }),
            cv: Condvar::new(),
            stop: AtomicBool::new(false),
        });

        let shared_clone = Arc::clone(&shared);
        let handle = thread::Builder::new()
            .name("worker_queue".to_string())
            .spawn(move || {
                Self::run_loop(shared_clone);
            })
            .expect("Failed to spawn worker thread");

        Self {
            shared,
            handle: Mutex::new(Some(handle)),
        }
    }

    /// Submit a task to run sequentially on the worker thread.
    pub fn submit<F>(&self, task: F)
    where
        F: FnOnce() + Send + 'static,
    {
        if self.shared.stop.load(Ordering::SeqCst) {
            return;
        }
        {
            let mut state = self.shared.state.lock().unwrap();
            state.tasks.push_back(Box::new(task));
        }
        self.shared.cv.notify_one();
    }

    /// Discard all pending queued tasks without running them.
    pub fn clear(&self) {
        let mut state = self.shared.state.lock().unwrap();
        state.tasks.clear();
    }

    /// Stop accepting tasks, signal the worker to exit, and join the thread.
    pub fn shutdown(&self) {
        if self.shared.stop.swap(true, Ordering::SeqCst) {
            return;
        }
        {
            let mut state = self.shared.state.lock().unwrap();
            state.tasks.clear();
        }
        self.shared.cv.notify_all();

        if let Ok(mut lock) = self.handle.lock()
            && let Some(h) = lock.take()
        {
            let _ = h.join();
        }
    }

    fn run_loop(shared: Arc<WorkerShared>) {
        while !shared.stop.load(Ordering::SeqCst) {
            let task = {
                let mut state = shared.state.lock().unwrap();
                while state.tasks.is_empty() && !shared.stop.load(Ordering::SeqCst) {
                    state = shared.cv.wait(state).unwrap();
                }
                if shared.stop.load(Ordering::SeqCst) && state.tasks.is_empty() {
                    return;
                }
                state.tasks.pop_front()
            };

            if let Some(task) = task {
                task();
            }
        }
    }
}

impl Default for WorkerQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for WorkerQueue {
    fn drop(&mut self) {
        self.shutdown();
    }
}
