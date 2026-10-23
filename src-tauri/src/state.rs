//! What `briefpane-hook` records per pane: one JSON file under
//! `$XDG_STATE_HOME/briefpane/panes`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct AgentState {
    state: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    pub transcript: String,
    #[serde(default)]
    pub last: String,
    at: u64,
}

fn dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::home().join(".local/state"))
        .join("briefpane/panes")
}

pub fn state_of(pane_id: &str) -> Option<AgentState> {
    let raw =
        std::fs::read(dir().join(format!("{}.json", pane_id.trim_start_matches('%')))).ok()?;
    serde_json::from_slice(&raw).ok()
}
