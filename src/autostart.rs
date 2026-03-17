use crate::error::{AppError, Result};

pub fn set_autostart(_enabled: bool) -> Result<()> {
    Err(AppError::Process(
        "autostart is not implemented in Phase 1".to_string(),
    ))
}

pub fn is_autostart_enabled() -> bool {
    false
}
