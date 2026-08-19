//! Starts and watches the privileged `app_process` bridge.

#![forbid(unsafe_code)]

use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use thiserror::Error;
use tracing::{error, info, warn};

#[derive(Debug, Clone)]
pub struct BridgeProcessSpec {
    pub app_process: PathBuf,
    pub dex_path: PathBuf,
    pub main_class: String,
    pub socket_name: String,
    pub manager_package: String,
}

impl BridgeProcessSpec {
    #[must_use]
    pub fn android_defaults(
        dex_path: impl Into<PathBuf>,
        socket_name: impl Into<String>,
        manager_package: impl Into<String>,
    ) -> Self {
        Self {
            app_process: PathBuf::from("/system/bin/app_process"),
            dex_path: dex_path.into(),
            main_class: "io.github.lingqiqi5211.crashcatcher.bridge.CrashCatcherBridge".into(),
            socket_name: socket_name.into(),
            manager_package: manager_package.into(),
        }
    }

    #[must_use]
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.app_process);
        command
            .env("CLASSPATH", &self.dex_path)
            .arg("/system/bin")
            .arg(&self.main_class)
            .arg("--socket")
            .arg(&self.socket_name)
            .arg("--manager-package")
            .arg(&self.manager_package)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RestartPolicy {
    pub initial_delay: Duration,
    pub maximum_delay: Duration,
    /// A process alive for this long is considered stable and resets backoff.
    pub stable_after: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(1),
            maximum_delay: Duration::from_secs(60),
            stable_after: Duration::from_secs(30),
        }
    }
}

impl RestartPolicy {
    #[must_use]
    pub fn next_delay(self, previous: Duration, ran_for: Duration) -> Duration {
        if ran_for >= self.stable_after {
            self.initial_delay
        } else {
            previous.saturating_mul(2).min(self.maximum_delay)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeState {
    Starting,
    Running {
        pid: u32,
        restart_count: u64,
    },
    Backoff {
        delay: Duration,
        restart_count: u64,
        detail: String,
    },
    Stopped,
}

pub struct BridgeSupervisor {
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<BridgeState>>,
    thread: Option<JoinHandle<()>>,
}

impl BridgeSupervisor {
    pub fn start(spec: BridgeProcessSpec, policy: RestartPolicy) -> Result<Self, SupervisorError> {
        if spec.main_class.is_empty() || spec.socket_name.is_empty() {
            return Err(SupervisorError::InvalidSpec);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(BridgeState::Starting));
        let thread_stop = Arc::clone(&stop);
        let thread_state = Arc::clone(&state);
        let handle = thread::Builder::new()
            .name("ct-bridge-supervisor".into())
            .spawn(move || supervise(spec, policy, &thread_stop, &thread_state))
            .map_err(SupervisorError::SpawnThread)?;
        Ok(Self {
            stop,
            state,
            thread: Some(handle),
        })
    }

    #[must_use]
    pub fn state(&self) -> BridgeState {
        self.state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_else(|_| BridgeState::Backoff {
                delay: Duration::ZERO,
                restart_count: 0,
                detail: "supervisor state lock poisoned".into(),
            })
    }

    pub fn stop(mut self) -> Result<(), SupervisorError> {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            handle.thread().unpark();
            handle.join().map_err(|_| SupervisorError::Join)?;
        }
        Ok(())
    }
}

impl Drop for BridgeSupervisor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}

fn supervise(
    spec: BridgeProcessSpec,
    policy: RestartPolicy,
    stop: &AtomicBool,
    state: &Mutex<BridgeState>,
) {
    let mut delay = policy.initial_delay;
    let mut restarts = 0_u64;
    while !stop.load(Ordering::Acquire) {
        set_state(state, BridgeState::Starting);
        let started_at = Instant::now();
        match spec.command().spawn() {
            Ok(mut child) => {
                info!(pid = child.id(), "privileged bridge started");
                set_state(
                    state,
                    BridgeState::Running {
                        pid: child.id(),
                        restart_count: restarts,
                    },
                );
                wait_for_child(&mut child, stop);
                let ran_for = started_at.elapsed();
                if stop.load(Ordering::Acquire) {
                    terminate(&mut child);
                    break;
                }
                warn!(?ran_for, "privileged bridge exited; scheduling restart");
                delay = policy.next_delay(delay, ran_for);
            }
            Err(error) => {
                error!(%error, "failed to start privileged bridge");
                delay = policy.next_delay(delay, Duration::ZERO);
            }
        }
        restarts = restarts.saturating_add(1);
        set_state(
            state,
            BridgeState::Backoff {
                delay,
                restart_count: restarts,
                detail: "bridge exited or could not start".into(),
            },
        );
        thread::park_timeout(delay);
    }
    set_state(state, BridgeState::Stopped);
}

fn wait_for_child(child: &mut Child, stop: &AtomicBool) {
    while !stop.load(Ordering::Acquire) {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => thread::park_timeout(Duration::from_millis(250)),
            Err(error) => {
                warn!(%error, "failed to query bridge process state");
                return;
            }
        }
    }
}

fn terminate(child: &mut Child) {
    if let Err(error) = child.kill() {
        warn!(%error, "failed to terminate bridge during shutdown");
    }
    let _ = child.wait();
}

fn set_state(state: &Mutex<BridgeState>, value: BridgeState) {
    if let Ok(mut state) = state.lock() {
        *state = value;
    }
}

#[derive(Debug, Error)]
pub enum SupervisorError {
    #[error("bridge process specification is incomplete")]
    InvalidSpec,
    #[error("failed to spawn supervisor thread: {0}")]
    SpawnThread(#[source] std::io::Error),
    #[error("bridge supervisor thread panicked")]
    Join,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_contract_matches_app_process() {
        let spec = BridgeProcessSpec::android_defaults(
            "/module/dex/cch_bridge.dex",
            "socket",
            "io.example.manager",
        );
        let command = spec.command();
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy())
            .collect();
        assert_eq!(args[0], "/system/bin");
        assert!(args.iter().any(|arg| arg == "--socket"));
        assert_eq!(
            command
                .get_envs()
                .find(|(key, _)| *key == "CLASSPATH")
                .and_then(|(_, value)| value),
            Some(std::ffi::OsStr::new("/module/dex/cch_bridge.dex"))
        );
    }

    #[test]
    fn stable_process_resets_backoff() {
        let policy = RestartPolicy::default();
        assert_eq!(
            policy.next_delay(Duration::from_secs(16), Duration::from_secs(31)),
            policy.initial_delay
        );
        assert_eq!(
            policy.next_delay(Duration::from_secs(40), Duration::from_secs(1)),
            policy.maximum_delay
        );
    }
}
