//! AOSP Android 12 tombstone schema, vendored as prost data types.
//!
//! The platform uses proto3 and only appends fields outside its reserved range,
//! so these definitions also decode newer tombstones while ignoring additions.

use std::collections::HashMap;

use prost::{Enumeration, Message};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Enumeration)]
#[repr(i32)]
pub enum Architecture {
    Arm32 = 0,
    Arm64 = 1,
    X86 = 2,
    X8664 = 3,
}

#[derive(Clone, PartialEq, Message)]
pub struct Tombstone {
    #[prost(enumeration = "Architecture", tag = "1")]
    pub arch: i32,
    #[prost(string, tag = "2")]
    pub build_fingerprint: String,
    #[prost(string, tag = "3")]
    pub revision: String,
    #[prost(string, tag = "4")]
    pub timestamp: String,
    #[prost(uint32, tag = "5")]
    pub pid: u32,
    #[prost(uint32, tag = "6")]
    pub tid: u32,
    #[prost(uint32, tag = "7")]
    pub uid: u32,
    #[prost(string, tag = "8")]
    pub selinux_label: String,
    /// The crashing process's `argv`, not just its name.
    ///
    /// Repeated in the platform schema, and it has to be repeated here too. Declared as a
    /// single string, prost applies proto3's last-one-wins rule to the repeated field and
    /// yields the *final* argument — so a process invoked as
    /// `./probe ./android.hardware.bluetooth.audio@2.0-impl.so` was filed under the shared
    /// object it was passed, which is not a process at all. The executable is `argv[0]`.
    #[prost(string, repeated, tag = "9")]
    pub command_line: Vec<String>,
    #[prost(message, optional, tag = "10")]
    pub signal_info: Option<Signal>,
    #[prost(string, tag = "14")]
    pub abort_message: String,
    #[prost(message, optional, tag = "15")]
    pub cause: Option<Cause>,
    #[prost(map = "uint32, message", tag = "16")]
    pub threads: HashMap<u32, Thread>,
    #[prost(message, repeated, tag = "17")]
    pub memory_mappings: Vec<MemoryMapping>,
    #[prost(message, repeated, tag = "18")]
    pub log_buffers: Vec<LogBuffer>,
    #[prost(message, repeated, tag = "19")]
    pub open_fds: Vec<FileDescriptor>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Signal {
    #[prost(int32, tag = "1")]
    pub number: i32,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(int32, tag = "3")]
    pub code: i32,
    #[prost(string, tag = "4")]
    pub code_name: String,
    #[prost(bool, tag = "5")]
    pub has_sender: bool,
    #[prost(int32, tag = "6")]
    pub sender_uid: i32,
    #[prost(int32, tag = "7")]
    pub sender_pid: i32,
    #[prost(bool, tag = "8")]
    pub has_fault_address: bool,
    #[prost(uint64, tag = "9")]
    pub fault_address: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct Cause {
    #[prost(string, tag = "1")]
    pub human_readable: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct Register {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(uint64, tag = "2")]
    pub u64: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct Thread {
    #[prost(int32, tag = "1")]
    pub id: i32,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(message, repeated, tag = "3")]
    pub registers: Vec<Register>,
    #[prost(message, repeated, tag = "4")]
    pub current_backtrace: Vec<BacktraceFrame>,
    #[prost(message, repeated, tag = "5")]
    pub memory_dump: Vec<MemoryDump>,
}

#[derive(Clone, PartialEq, Message)]
pub struct BacktraceFrame {
    #[prost(uint64, tag = "1")]
    pub rel_pc: u64,
    #[prost(uint64, tag = "2")]
    pub pc: u64,
    #[prost(uint64, tag = "3")]
    pub sp: u64,
    #[prost(string, tag = "4")]
    pub function_name: String,
    #[prost(uint64, tag = "5")]
    pub function_offset: u64,
    #[prost(string, tag = "6")]
    pub file_name: String,
    #[prost(uint64, tag = "7")]
    pub file_map_offset: u64,
    #[prost(string, tag = "8")]
    pub build_id: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct MemoryDump {
    #[prost(string, tag = "1")]
    pub register_name: String,
    #[prost(string, tag = "2")]
    pub mapping_name: String,
    #[prost(uint64, tag = "3")]
    pub begin_address: u64,
    #[prost(bytes = "vec", tag = "4")]
    pub memory: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct MemoryMapping {
    #[prost(uint64, tag = "1")]
    pub begin_address: u64,
    #[prost(uint64, tag = "2")]
    pub end_address: u64,
    #[prost(uint64, tag = "3")]
    pub offset: u64,
    #[prost(bool, tag = "4")]
    pub read: bool,
    #[prost(bool, tag = "5")]
    pub write: bool,
    #[prost(bool, tag = "6")]
    pub execute: bool,
    #[prost(string, tag = "7")]
    pub mapping_name: String,
    #[prost(string, tag = "8")]
    pub build_id: String,
    #[prost(uint64, tag = "9")]
    pub load_bias: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct FileDescriptor {
    #[prost(int32, tag = "1")]
    pub fd: i32,
    #[prost(string, tag = "2")]
    pub path: String,
    #[prost(string, tag = "3")]
    pub owner: String,
    #[prost(uint64, tag = "4")]
    pub tag: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct LogBuffer {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(message, repeated, tag = "2")]
    pub logs: Vec<LogMessage>,
}

#[derive(Clone, PartialEq, Message)]
pub struct LogMessage {
    #[prost(string, tag = "1")]
    pub timestamp: String,
    #[prost(uint32, tag = "2")]
    pub pid: u32,
    #[prost(uint32, tag = "3")]
    pub tid: u32,
    #[prost(uint32, tag = "4")]
    pub priority: u32,
    #[prost(string, tag = "5")]
    pub tag: String,
    #[prost(string, tag = "6")]
    pub message: String,
}
