use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Tauri error: {0}")]
    Tauri(#[from] tauri::Error),

    #[error("Book not found: {0}")]
    BookNotFound(String),

    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("{0}")]
    General(String),
}

impl From<zip::result::ZipError> for AppError {
    fn from(e: zip::result::ZipError) -> Self {
        AppError::General(e.to_string())
    }
}

impl From<base64::DecodeError> for AppError {
    fn from(e: base64::DecodeError) -> Self {
        AppError::General(e.to_string())
    }
}

impl From<AppError> for String {
    fn from(e: AppError) -> String {
        e.to_string()
    }
}

/// 允许 String 错误自动转换为 AppError::General
/// 这样现有的 `.map_err(|e| e.to_string())?` 模式可以无缝迁移到 AppResult
impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError::General(s)
    }
}

/// 允许 &str 错误自动转换为 AppError::General
/// 这样 `Err("...".into())` 模式可以无缝迁移到 AppResult
impl From<&str> for AppError {
    fn from(s: &str) -> Self {
        AppError::General(s.to_string())
    }
}

impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.to_string().as_ref())
    }
}

pub type AppResult<T> = Result<T, AppError>;
