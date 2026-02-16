//! Error types for the database layer.

/// Errors originating from the persistence layer.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// SurrealDB returned an error.
    #[error("database error: {0}")]
    Database(String),

    /// Schema migration failed.
    #[error("schema migration failed: {0}")]
    Migration(String),

    /// Serialization / deserialization failure.
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),

    /// The database has not been initialized.
    #[error("database not initialized — call DaqDb::init() first")]
    NotInitialized,

    /// An upsert operation failed.
    #[error("upsert failed on {table}[{key}]: {reason}")]
    UpsertFailed {
        table: String,
        key: String,
        reason: String,
    },

    /// A raw query failed.
    #[error("query failed: {reason} (query: {query})")]
    QueryFailed { query: String, reason: String },

    /// A transaction was aborted / rolled back.
    #[error("transaction aborted: {0}")]
    TransactionAborted(String),

    /// Reconciliation between TOML and DB state failed.
    #[error("reconcile failed: {0}")]
    ReconcileFailed(String),
}

#[cfg(any(feature = "kv-mem", feature = "kv-rocksdb"))]
impl From<surrealdb::Error> for DbError {
    fn from(e: surrealdb::Error) -> Self {
        Self::Database(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, DbError>;
