use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Deserialize)]
pub struct Request {
    pub id: String,
    #[serde(flatten)]
    pub op: Operation,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Operation {
    // Folder Picker
    PickDownloadDirectory,

    // Register a download root from an externally-picked path (e.g. Tauri dialog)
    RegisterDownloadRoot {
        path: String,
    },

    // Delete Download Root
    DeleteDownloadRoot {
        key: String,
    },

    // Handshake
    Handshake {
        #[serde(rename = "extensionId")]
        extension_id: String,
        #[allow(dead_code)] // Legacy field, ignored after Phase 2
        #[serde(default, rename = "installId")]
        install_id: Option<String>,
        #[serde(default, rename = "profileId")]
        profile_id: Option<String>,
        #[serde(default, rename = "clientType")]
        client_type: Option<String>,
        #[serde(default, rename = "clientVersion")]
        client_version: Option<String>,
    },

    // Take over an in-use profile
    TakeOver {
        #[serde(rename = "extensionId")]
        extension_id: String,
        #[allow(dead_code)] // Legacy field, ignored after Phase 2
        #[serde(default, rename = "installId")]
        install_id: Option<String>,
        #[serde(default, rename = "profileId")]
        profile_id: Option<String>,
        #[serde(default, rename = "clientType")]
        client_type: Option<String>,
        #[serde(default, rename = "clientVersion")]
        client_version: Option<String>,
    },

    // Open file with default application
    OpenFile {
        #[serde(rename = "rootKey")]
        root_key: String,
        path: String,
    },

    // Reveal file in system file manager
    RevealInFolder {
        #[serde(rename = "rootKey")]
        root_key: String,
        path: String,
    },

    // Update operations
    CheckForUpdates,
    InstallUpdate,

    // KV storage operations
    KvGet {
        key: String,
    },
    KvGetMulti {
        keys: Vec<String>,
    },
    KvSet {
        key: String,
        value: String,
    },
    KvDelete {
        key: String,
    },
    KvKeys {
        prefix: Option<String>,
    },
    KvClear {
        prefix: Option<String>,
    },

    // Read a .torrent file from disk
    ReadTorrentFile {
        path: String,
    },

    // Launch the Tauri desktop app
    LaunchDesktop,

    // Profile management (no handshake required)
    ListProfiles,
    RenameProfile {
        #[serde(rename = "profileId")]
        profile_id: String,
        #[serde(rename = "displayName")]
        display_name: String,
    },
    DeleteProfile {
        #[serde(rename = "profileId")]
        profile_id: String,
    },
}

#[derive(Debug, Serialize)]
pub struct Response {
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(flatten)]
    pub payload: ResponsePayload,
}

use jstorrent_common::DownloadRoot;

#[derive(Debug, Serialize, Deserialize)]
pub struct ProfileListEntry {
    #[serde(rename = "profileId")]
    pub profile_id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub created: u64,
    #[serde(rename = "lastUsed")]
    pub last_used: u64,
    #[serde(rename = "clientType")]
    pub client_type: Option<String>,
    #[serde(rename = "clientVersion")]
    pub client_version: Option<String>,
    pub live: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct DaemonCapabilities {
    pub roots_manageable: bool,
    pub lan_share_urls: bool,
    #[serde(default)]
    pub free_space: bool,
    #[serde(default)]
    pub write_atomic: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum ResponsePayload {
    Empty,
    DaemonInfo {
        #[serde(rename = "profileId")]
        profile_id: String,
        port: u16,
        token: String,
        version: String,
        #[serde(rename = "protocolVersion", skip_serializing_if = "Option::is_none")]
        protocol_version: Option<u32>,
        #[serde(rename = "behaviorVersion", skip_serializing_if = "Option::is_none")]
        behavior_version: Option<u32>,
        roots: Vec<DownloadRoot>,
        #[serde(rename = "addToken", skip_serializing_if = "Option::is_none")]
        add_token: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        capabilities: Option<DaemonCapabilities>,
        #[serde(rename = "desktopVersion", skip_serializing_if = "Option::is_none")]
        desktop_version: Option<String>,
    },
    Path {
        path: String,
    },
    RootAdded {
        root: DownloadRoot,
    },
    RootRemoved {
        key: String,
    },
    KvValue {
        value: Option<String>,
    },
    KvMultiValue {
        entries: HashMap<String, String>,
    },
    KvKeys {
        keys: Vec<String>,
    },
    ProfileInUse {
        #[serde(rename = "profileId")]
        profile_id: String,
        #[serde(rename = "clientType")]
        client_type: Option<String>,
        #[serde(rename = "clientVersion")]
        client_version: Option<String>,
        #[serde(rename = "browserName")]
        browser_name: Option<String>,
        pid: u32,
        started: u64,
    },
    UpdateCheck {
        available: bool,
        version: Option<String>,
        #[serde(rename = "currentVersion")]
        current_version: Option<String>,
        body: Option<String>,
    },
    ProfileList {
        profiles: Vec<ProfileListEntry>,
    },
    TorrentFileContents {
        name: String,
        #[serde(rename = "contentsBase64")]
        contents_base64: String,
    },
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operation::PickDownloadDirectory => write!(f, "PickDownloadDirectory"),
            Operation::RegisterDownloadRoot { path } => {
                write!(f, "RegisterDownloadRoot {path}")
            }
            Operation::DeleteDownloadRoot { key } => write!(f, "DeleteDownloadRoot {key}"),
            Operation::Handshake {
                extension_id,
                profile_id,
                ..
            } => write!(
                f,
                "Handshake ext={extension_id} profile={}",
                profile_id.as_deref().unwrap_or("(auto)")
            ),
            Operation::TakeOver {
                extension_id,
                profile_id,
                ..
            } => write!(
                f,
                "TakeOver ext={extension_id} profile={}",
                profile_id.as_deref().unwrap_or("(auto)")
            ),
            Operation::OpenFile { root_key, path } => {
                write!(f, "OpenFile {root_key}:{path}")
            }
            Operation::RevealInFolder { root_key, path } => {
                write!(f, "RevealInFolder {root_key}:{path}")
            }
            Operation::CheckForUpdates => write!(f, "CheckForUpdates"),
            Operation::InstallUpdate => write!(f, "InstallUpdate"),
            Operation::KvGet { key } => write!(f, "KvGet {key}"),
            Operation::KvGetMulti { keys } => write!(f, "KvGetMulti [{}]", keys.join(", ")),
            Operation::KvSet { key, value } => write!(f, "KvSet {key} ({} bytes)", value.len()),
            Operation::KvDelete { key } => write!(f, "KvDelete {key}"),
            Operation::KvKeys { prefix } => {
                write!(f, "KvKeys {}", prefix.as_deref().unwrap_or("(all)"))
            }
            Operation::KvClear { prefix } => {
                write!(f, "KvClear {}", prefix.as_deref().unwrap_or("(all)"))
            }
            Operation::ReadTorrentFile { path } => write!(f, "ReadTorrentFile {path}"),
            Operation::LaunchDesktop => write!(f, "LaunchDesktop"),
            Operation::ListProfiles => write!(f, "ListProfiles"),
            Operation::RenameProfile {
                profile_id,
                display_name,
            } => write!(f, "RenameProfile {profile_id} -> {display_name}"),
            Operation::DeleteProfile { profile_id } => {
                write!(f, "DeleteProfile {profile_id}")
            }
        }
    }
}

impl fmt::Display for ResponsePayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResponsePayload::Empty => write!(f, "Empty"),
            ResponsePayload::DaemonInfo {
                port,
                version,
                roots,
                profile_id,
                ..
            } => write!(
                f,
                "DaemonInfo port={port} v={version} profile={profile_id} roots={}",
                roots.len()
            ),
            ResponsePayload::Path { path } => write!(f, "Path {path}"),
            ResponsePayload::RootAdded { root } => write!(f, "RootAdded {}", root.key),
            ResponsePayload::RootRemoved { key } => write!(f, "RootRemoved {key}"),
            ResponsePayload::KvValue { value } => match value {
                Some(v) => write!(f, "{} bytes", v.len()),
                None => write!(f, "None"),
            },
            ResponsePayload::KvMultiValue { entries } => {
                write!(f, "{} entries", entries.len())
            }
            ResponsePayload::KvKeys { keys } => write!(f, "{} keys", keys.len()),
            ResponsePayload::ProfileInUse {
                profile_id, pid, ..
            } => {
                write!(f, "ProfileInUse profile={profile_id} pid={pid}")
            }
            ResponsePayload::UpdateCheck {
                available, version, ..
            } => {
                if *available {
                    write!(
                        f,
                        "UpdateCheck available={}",
                        version.as_deref().unwrap_or("?")
                    )
                } else {
                    write!(f, "UpdateCheck up-to-date")
                }
            }
            ResponsePayload::ProfileList { profiles } => {
                write!(f, "ProfileList {} profiles", profiles.len())
            }
            ResponsePayload::TorrentFileContents {
                name,
                contents_base64,
            } => {
                write!(
                    f,
                    "TorrentFileContents {} ({} bytes)",
                    name,
                    contents_base64.len()
                )
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", content = "payload")]
pub enum Event {
    Log {
        message: String,
    },
    MagnetAdded {
        link: String,
    },
    TorrentAdded {
        name: String,
        infohash: String,
        #[serde(rename = "contentsBase64")]
        contents_base64: String,
    },
    UpdateAvailable {
        version: String,
        #[serde(rename = "currentVersion")]
        current_version: String,
    },
}
