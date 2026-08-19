use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};

use cch_model::RecordId;
use cch_wire::{
    BridgeCommand, BridgeEvent, BridgePackageInfo, IntentSpec, NotificationSpec, WireError,
};

const BRIDGE_REPLY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub struct BridgeBroker {
    connected: AtomicBool,
    next_request_id: AtomicU64,
    command_sender: Mutex<Option<Sender<BridgeCommand>>>,
    pending: Mutex<HashMap<u64, Sender<BridgeEvent>>>,
    package_cache: Mutex<HashMap<(String, i32), BridgePackageInfo>>,
}

impl Default for BridgeBroker {
    fn default() -> Self {
        Self {
            connected: AtomicBool::new(false),
            next_request_id: AtomicU64::new(1),
            command_sender: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            package_cache: Mutex::new(HashMap::new()),
        }
    }
}

impl BridgeBroker {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    pub fn attach(&self) -> Result<Receiver<BridgeCommand>, WireError> {
        let (sender, receiver) = mpsc::channel();
        let mut active = self
            .command_sender
            .lock()
            .map_err(|_| WireError::internal("bridge sender lock poisoned"))?;
        *active = Some(sender);
        self.connected.store(false, Ordering::Release);
        Ok(receiver)
    }

    pub fn detach(&self) {
        self.connected.store(false, Ordering::Release);
        if let Ok(mut sender) = self.command_sender.lock() {
            *sender = None;
        }
        if let Ok(mut pending) = self.pending.lock() {
            pending.clear();
        }
    }

    pub fn handle_event(&self, event: BridgeEvent) {
        match &event {
            BridgeEvent::Hello { .. } => {
                self.connected.store(true, Ordering::Release);
            }
            BridgeEvent::PackageInfoResult {
                request_id,
                package,
                ..
            } => {
                if let Some(package) = package
                    && let Ok(mut cache) = self.package_cache.lock()
                {
                    cache.insert(
                        (package.package_name.clone(), package.user_id),
                        package.clone(),
                    );
                }
                self.finish_request(*request_id, event);
            }
            BridgeEvent::ActivityResult { request_id, .. }
            | BridgeEvent::NotificationResult { request_id, .. } => {
                self.finish_request(*request_id, event);
            }
            BridgeEvent::ForegroundChanged { .. } => {}
        }
    }

    #[must_use]
    pub fn cached_package(&self, package_name: &str, user_id: i32) -> Option<BridgePackageInfo> {
        self.package_cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(&(package_name.to_owned(), user_id)).cloned())
    }

    pub fn query_package(
        &self,
        package_name: &str,
        user_id: i32,
    ) -> Result<Option<BridgePackageInfo>, WireError> {
        if let Some(cached) = self.cached_package(package_name, user_id) {
            return Ok(Some(cached));
        }
        let request_id = self.next_id();
        let event = self.request(
            request_id,
            BridgeCommand::QueryPackageInfo {
                request_id,
                package_name: package_name.to_owned(),
                user_id,
            },
        )?;
        match event {
            BridgeEvent::PackageInfoResult { package, error, .. } => match error {
                Some(error) => Err(WireError::unavailable(error)),
                None => Ok(package),
            },
            _ => Err(WireError::internal("bridge returned the wrong reply type")),
        }
    }

    pub fn start_activity(&self, intent: IntentSpec, user_id: i32) -> Result<bool, WireError> {
        let request_id = self.next_id();
        let event = self.request(
            request_id,
            BridgeCommand::StartActivity {
                request_id,
                intent,
                user_id,
            },
        )?;
        match event {
            BridgeEvent::ActivityResult {
                launched, error, ..
            } => match error {
                Some(error) => Err(WireError::unavailable(error)),
                None => Ok(launched),
            },
            _ => Err(WireError::internal("bridge returned the wrong reply type")),
        }
    }

    pub fn post_notification(&self, notification: NotificationSpec) -> Result<bool, WireError> {
        let request_id = self.next_id();
        let event = self.request(
            request_id,
            BridgeCommand::PostNotification {
                request_id,
                notification,
            },
        )?;
        match event {
            BridgeEvent::NotificationResult { posted, error, .. } => match error {
                Some(error) => Err(WireError::unavailable(error)),
                None => Ok(posted),
            },
            _ => Err(WireError::internal("bridge returned the wrong reply type")),
        }
    }

    /// Takes down a notification the bridge posted earlier.
    ///
    /// Shares `NotificationResult` with [`Self::post_notification`] — the bridge answers
    /// both with the same reply — so a cancel that reaches a disconnected bridge fails the
    /// same way a post does rather than looking like it worked.
    pub fn cancel_notification(&self, record_id: RecordId) -> Result<bool, WireError> {
        let request_id = self.next_id();
        let event = self.request(
            request_id,
            BridgeCommand::CancelNotification {
                request_id,
                record_id,
            },
        )?;
        match event {
            BridgeEvent::NotificationResult { posted, error, .. } => match error {
                Some(error) => Err(WireError::unavailable(error)),
                None => Ok(posted),
            },
            _ => Err(WireError::internal("bridge returned the wrong reply type")),
        }
    }

    fn next_id(&self) -> u64 {
        self.next_request_id.fetch_add(1, Ordering::Relaxed)
    }

    fn request(&self, request_id: u64, command: BridgeCommand) -> Result<BridgeEvent, WireError> {
        if !self.is_connected() {
            return Err(WireError::unavailable("system bridge is not connected"));
        }
        let (sender, receiver) = mpsc::channel();
        self.pending
            .lock()
            .map_err(|_| WireError::internal("bridge pending lock poisoned"))?
            .insert(request_id, sender);

        let send_result = self
            .command_sender
            .lock()
            .map_err(|_| WireError::internal("bridge sender lock poisoned"))?
            .as_ref()
            .ok_or_else(|| WireError::unavailable("system bridge is not connected"))?
            .send(command);
        if send_result.is_err() {
            self.remove_pending(request_id);
            return Err(WireError::unavailable(
                "system bridge command channel closed",
            ));
        }

        match receiver.recv_timeout(BRIDGE_REPLY_TIMEOUT) {
            Ok(event) => Ok(event),
            Err(_) => {
                self.remove_pending(request_id);
                Err(WireError::unavailable("system bridge reply timed out"))
            }
        }
    }

    fn finish_request(&self, request_id: u64, event: BridgeEvent) {
        let sender = self
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(&request_id));
        if let Some(sender) = sender {
            let _ = sender.send(event);
        }
    }

    fn remove_pending(&self, request_id: u64) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&request_id);
        }
    }
}
