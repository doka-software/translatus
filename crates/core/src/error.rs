use thiserror::Error;

/// Top-level error type for the translation core.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("xml error: {0}")]
    Xml(#[from] quick_xml::Error),

    #[error("xml attr error: {0}")]
    XmlAttr(#[from] quick_xml::events::attributes::AttrError),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),

    #[error("malformed epub: {0}")]
    MalformedEpub(String),

    /// The LLM response failed structural validation (alignment / placeholders / language).
    #[error("validation failed: {0}")]
    Validation(String),

    #[error("provider error: {0}")]
    Provider(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;
