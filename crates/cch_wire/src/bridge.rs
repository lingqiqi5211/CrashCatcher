use std::collections::BTreeMap;

use cch_model::RecordId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeHello {
    pub android_sdk: u32,
    pub bridge_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgePackageInfo {
    pub package_name: String,
    pub user_id: i32,
    pub version_name: Option<String>,
    pub version_code: Option<i64>,
    pub target_sdk: Option<u32>,
    pub min_sdk: Option<u32>,
    pub primary_abi: Option<String>,
    pub label: Option<String>,
    pub is_system_app: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeAction {
    OpenDetails,
    ReopenApp,
    AppInfo,
    MuteUntilUnlock,
    MuteUntilRestart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationAction {
    pub title: String,
    pub action: BridgeAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSpec {
    pub record_id: RecordId,
    pub package_name: String,
    pub user_id: i32,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub actions: Vec<NotificationAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentSpec {
    pub action: String,
    pub package_name: Option<String>,
    pub component: Option<String>,
    #[serde(default)]
    pub extras: BTreeMap<String, String>,
}

/// Commands sent from the root daemon to the privileged Java bridge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeCommand {
    PostNotification {
        request_id: u64,
        notification: NotificationSpec,
    },
    CancelNotification {
        request_id: u64,
        record_id: RecordId,
    },
    QueryPackageInfo {
        request_id: u64,
        package_name: String,
        user_id: i32,
    },
    StartActivity {
        request_id: u64,
        intent: IntentSpec,
        user_id: i32,
    },
}

/// Events and replies sent from the Java bridge to the daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeEvent {
    Hello {
        hello: BridgeHello,
    },
    PackageInfoResult {
        request_id: u64,
        package: Option<BridgePackageInfo>,
        error: Option<String>,
    },
    ActivityResult {
        request_id: u64,
        launched: bool,
        error: Option<String>,
    },
    NotificationResult {
        request_id: u64,
        posted: bool,
        error: Option<String>,
    },
    ForegroundChanged {
        package_name: Option<String>,
        user_id: Option<i32>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use cch_model::RecordIdGenerator;

    #[test]
    fn bridge_messages_have_stable_direction_tags() {
        let command = BridgeCommand::CancelNotification {
            request_id: 7,
            record_id: RecordIdGenerator::new().next(1),
        };
        let json = serde_json::to_string(&command).expect("serializes");
        assert!(json.contains(r#""type":"cancel_notification""#));
        let parsed: BridgeCommand = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(parsed, command);

        let hello = BridgeEvent::Hello {
            hello: BridgeHello {
                android_sdk: 35,
                bridge_version: "0.1.0".into(),
            },
        };
        assert!(
            serde_json::to_string(&hello)
                .expect("serializes")
                .contains(r#""type":"hello""#)
        );
    }
}
