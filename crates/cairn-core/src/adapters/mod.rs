pub mod claude;
pub mod codex;
pub mod codex_file;

use std::path::Path;

use crate::error::Result;
use crate::model::{Agent, ImportOutcome, SessionRecord};

pub trait AgentAdapter {
    fn agent(&self) -> Agent;
    fn discover(&self) -> Result<Vec<SessionRecord>>;
    fn import(&self, record: &SessionRecord, target_cwd: &Path) -> Result<ImportOutcome>;
    fn resume_command(&self, native_id: &str) -> Vec<String>;

    /// Some(distro) for adapters that manage sessions inside WSL.
    fn wsl_distro(&self) -> Option<&str> {
        None
    }
}
