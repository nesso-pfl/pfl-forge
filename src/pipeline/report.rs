use tracing::info;

use crate::error::Result;
use crate::pipeline::execute::ExecuteResult;
use crate::state::tracker::SharedState;
use crate::task::ForgeTask;

pub fn report(forge_task: &ForgeTask, result: &ExecuteResult, state: &SharedState) -> Result<()> {
  match result {
    ExecuteResult::Success { .. } => {}

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
