use thiserror::Error;

#[derive(Error, Debug)]
pub enum KtoError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] ureq::Error),

    #[error("Database error: {0}")]
    DatabaseError(#[from] rusqlite::Error),

    #[error("Migration error: {0}")]
    MigrationError(#[from] refinery::Error),

    #[error("JSON serialization error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("TOML parsing error: {0}")]
    TomlError(#[from] toml::de::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("URL parse error: {0}")]
    UrlParseError(#[from] url::ParseError),

    #[error("Extraction failed: {0}")]
    ExtractionError(String),

    #[error("Playwright error: {0}")]
    PlaywrightError(String),

    #[error("Claude CLI not installed: {0}")]
    ClaudeNotInstalled(String),

    #[error("Claude CLI failed: {0}")]
    ClaudeFailed(String),

    #[error("Watch not found: {0}")]
    WatchNotFound(String),

    #[error("Watch name already exists: {0}")]
    DuplicateWatchName(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Notification failed: {0}")]
    NotificationError(String),

    #[error("Feed parsing error: {0}")]
    FeedParseError(String),

    #[error("Retry with deep research mode")]
    RetryWithDeepResearch,
}

impl KtoError {
    /// Get an actionable hint for how to resolve this error
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            KtoError::HttpError(_) => Some(
                "Check your internet connection, or try:\n  kto test \"<watch>\" --verbose"
            ),
            KtoError::WatchNotFound(_) => Some(
                "Run `kto list` to see available watches"
            ),
            KtoError::DuplicateWatchName(_) => Some(
                "Choose a different name, or edit the existing watch:\n  kto edit \"<name>\" --url <new-url>"
            ),
            KtoError::ClaudeNotInstalled(_) => Some(
                "Install Claude CLI: curl -fsSL https://claude.ai/install.sh | bash"
            ),
            KtoError::PlaywrightError(_) => Some(
                "Run `kto enable-js` to set up JavaScript rendering"
            ),
            KtoError::NotificationError(_) => Some(
                "Check your notification settings with `kto notify show`\nOr reconfigure with `kto notify set`"
            ),
            KtoError::ExtractionError(_) => Some(
                "Try using a CSS selector: kto edit \"<watch>\" --selector \".content\"\nOr enable JavaScript: kto edit \"<watch>\" --js true"
            ),
            KtoError::DatabaseError(_) => Some(
                "Try running `kto doctor` to check database status"
            ),
            KtoError::FeedParseError(_) => Some(
                "Check that the URL points to a valid RSS or Atom feed"
            ),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, KtoError>;
