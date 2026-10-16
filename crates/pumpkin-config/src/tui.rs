use serde::{Deserialize, Serialize};

/// Configuration for the interactive terminal dashboard.
#[derive(Deserialize, Serialize)]
#[serde(default)]
pub struct TuiConfig {
    /// Enables the dashboard when standard input and output are terminals.
    pub enabled: bool,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}
