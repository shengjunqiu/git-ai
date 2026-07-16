pub mod agents;
pub(crate) mod command_line;
pub mod ensure_git_symlinks;
pub mod git_client_installer;
pub mod git_clients;
pub mod hook_installer;
pub mod jetbrains;
pub mod skills_installer;
pub mod spinner;
pub mod utils;

#[cfg(feature = "test-support")]
pub use command_line::test_support as command_line_test_support;
pub use ensure_git_symlinks::ensure_git_symlinks;
