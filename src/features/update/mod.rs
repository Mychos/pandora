//! Whether a newer Pandora exists, and the download that replaces this build with
//! it: the one request the app makes with no account behind it.

pub mod github;
pub mod install;
pub mod logic;
pub mod model;
pub mod ui;

pub use model::Update;
