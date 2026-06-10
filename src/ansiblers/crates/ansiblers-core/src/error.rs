use thiserror::Error;

#[derive(Debug, Error)]
pub enum AnsiblersError {
    #[error("Task failed on host '{host}': {message}")]
    TaskFailed { host: String, message: String },

    #[error("Host unreachable: {host}")]
    HostUnreachable { host: String },

    #[error("Module not found: {module}")]
    ModuleNotFound { module: String },

    #[error("Playbook parse error: {0}")]
    ParseError(String),

    #[error("Template error: {0}")]
    TemplateError(String),

    #[error("Inventory error: {0}")]
    InventoryError(String),

    #[error("Variable error: {0}")]
    VarError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, AnsiblersError>;
