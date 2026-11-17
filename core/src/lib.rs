//! Core of the account switcher: no UI, no Tauri.
//!
//! Riot Client keeps the "stay signed in" session in a handful of files and
//! writes them when it quits. We keep one encrypted copy of those files per
//! account, put the right copy back while the client is closed, and start
//! the client again.

pub mod api;
pub mod dpapi;
pub mod identity;
pub mod procs;
pub mod shortcut;
pub mod snapshot;
pub mod store;
pub mod switcher;
pub mod typer;
pub mod vault;

pub use api::{ClientApi, Identity, LeagueInfo, LocalApi, LoginState};
pub use procs::{Procs, SysProcs};
pub use store::{Account, Config, Env, Game};
pub use switcher::{Error, LoginOutcome, Switcher, Timing, View};
pub use typer::{LoginTyper, UiTarget, WindowTyper};
pub use vault::KdfParams;
