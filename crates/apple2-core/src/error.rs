use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid card slot {0}")]
    InvalidSlot(u8),

    #[error("invalid save-state version {0}")]
    InvalidVersion(u32),

    #[error("ROM load error: {0}")]
    RomLoad(String),

    #[error("disk image error: {0}")]
    DiskImage(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("save-state serialization error: {0}")]
    Serde(String),
}

pub type Result<T> = std::result::Result<T, Error>;
