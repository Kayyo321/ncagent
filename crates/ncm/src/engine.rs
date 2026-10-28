/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The real NCM engine. It does not exist yet.
//!
//! [`open_export`] is the single place where a real backend plugs in. When milestones A1
//! (export reader) and N1 (engine) are done, this function validates the export directory and
//! returns an engine that implements [`backend_t`](crate::backend::backend_t). Until then it
//! returns a named error. It reads nothing, validates nothing and loads nothing.

use std::path::Path;
use std::sync::Arc;

use crate::backend::{BackendError, backend_t};

/// Open an export directory as a backend.
pub fn open_export(dir: &Path) -> Result<Arc<dyn backend_t>, BackendError> {
    Err(BackendError::NotImplemented {
        feature: format!(
            "the NCM engine and the export reader (milestones N1 and A1); {} was not read or validated",
            dir.display()
        ),
    })
}
