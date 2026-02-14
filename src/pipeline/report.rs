use tracing::info;

use crate::error::Result;
use crate::pipeline::execute::ExecuteResult;
use crate::state::tracker::{SharedState, TaskStatus};
use crate::task::ForgeTask;

pub fn report(forge_task: &ForgeTask, result: &ExecuteResult, state: &SharedState) -> Result<()> {
  let branch = forge_task.branch_name();

  match result {
    ExecuteResult::Success { .. } => {}

    ExecuteResult::TestFailure { commits, .. } => {
      info!("test failure: {forge_task} with {commits} commits");
      info!("task {forge_task}: tests failed, branch {branch} left as-is");
      state.lock().unwrap().set_status(
        &forge_task.id,
        &forge_task.title,
        TaskStatus::TestFailure,
      )?;
    }

    ExecuteResult::Unclear(reason) => {
      info!("unclear result: {forge_task}: {reason}");
      state.lock().unwrap().set_error(&forge_task.id, reason)?;
    }

    ExecuteResult::Error(error) => {
      info!("error: {forge_task}: {error}");
      state.lock().unwrap().set_error(&forge_task.id, error)?;
    }
  }

  Ok(())
}
