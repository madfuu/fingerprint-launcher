//! Tauri commands (design §5): one thin async wrapper per bridge call. Arguments arrive in
//! camelCase; failures reject with `{ code, message }` (`AppError`).
//!
//! Every command is `async`, so none runs on the main thread (a sync command would block the
//! UI); database work goes through `AppState::db` on the blocking pool.

pub mod app;
pub mod kernels;
pub mod launch;
pub mod library;
pub mod profiles;
