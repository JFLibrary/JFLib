// Bounded cooperative tasks for long native/browser operations. New shared implementation.

use crate::{ActionError, ActionResult, ErrorCode, Value};
use alloc::{string::String, vec::Vec};

/// State of a cooperative host operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Queued but not started.
    Pending,
    /// Executing between interruption points.
    Running,
    /// Cancellation was requested; a host checkpoint still has to acknowledge it.
    Cancelling,
    /// Finished successfully.
    Complete,
    /// Finished with an action error.
    Failed,
    /// Cancellation was acknowledged at an interruption point.
    Cancelled,
}

impl Status {
    /// Stable status spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
    /// Whether no more computation should be performed.
    pub fn terminal(self) -> bool {
        matches!(self, Self::Complete | Self::Failed | Self::Cancelled)
    }
}

/// One task, isolated inside its app/connection owner.
#[derive(Clone, Debug)]
pub struct Task {
    /// Monotonically allocated identifier; never reused by a table.
    pub id: u64,
    /// Semantic action that created the task.
    pub action: String,
    /// Current lifecycle state.
    pub status: Status,
    /// Progress in thousandths, when available.
    pub progress: Option<u16>,
    /// Short host-provided progress description.
    pub message: String,
    /// Whether cancellation is supported at host interruption points.
    pub cancellable: bool,
    /// Completed result, available only in the Complete state.
    pub result: Option<ActionResult>,
    /// Terminal failure or cancellation diagnosis.
    pub error: Option<ActionError>,
}

impl Task {
    /// Compact poll response. Binary resources remain links inside action results.
    pub fn to_json(&self) -> Value {
        Value::object([
            ("id", Value::u64(self.id)),
            ("action", Value::string(&self.action)),
            ("status", Value::string(self.status.as_str())),
            (
                "progress",
                self.progress
                    .map(|p| Value::u64(p as u64))
                    .unwrap_or(Value::Null),
            ),
            ("message", Value::string(&self.message)),
            ("cancellable", Value::Bool(self.cancellable)),
            (
                "result",
                self.result
                    .as_ref()
                    .map(ActionResult::to_json)
                    .unwrap_or(Value::Null),
            ),
            (
                "error",
                self.error
                    .as_ref()
                    .map(ActionError::to_json)
                    .unwrap_or(Value::Null),
            ),
        ])
    }
}

/// Bounded in-memory task owner; a host drives work by calling checkpoint/update/finish.
pub struct TaskTable {
    tasks: Vec<Task>,
    next_id: u64,
    capacity: usize,
}

impl Default for TaskTable {
    fn default() -> Self {
        Self::new(128)
    }
}

impl TaskTable {
    /// Create a table; capacity is clamped to 1..=1024 tasks.
    pub fn new(capacity: usize) -> Self {
        Self {
            tasks: Vec::new(),
            next_id: 1,
            capacity: capacity.clamp(1, 1024),
        }
    }
    /// Allocate a pending task. Completed tasks are retained until explicitly forgotten.
    pub fn create(&mut self, action: &str, cancellable: bool) -> Result<u64, ActionError> {
        if self.tasks.len() >= self.capacity || self.next_id == u64::MAX {
            return Err(ActionError::new(
                ErrorCode::LimitExceeded,
                "Task table is full",
            ));
        }
        if action.is_empty() || action.len() > 128 {
            return Err(ActionError::new(
                ErrorCode::InvalidInput,
                "Invalid task action name",
            ));
        }
        let id = self.next_id;
        self.next_id += 1;
        self.tasks.push(Task {
            id,
            action: action.into(),
            status: Status::Pending,
            progress: None,
            message: String::new(),
            cancellable,
            result: None,
            error: None,
        });
        Ok(id)
    }
    /// Inspect one task, without allowing external callers to mutate its state directly.
    pub fn get(&self, id: u64) -> Result<&Task, ActionError> {
        self.tasks.iter().find(|t| t.id == id).ok_or_else(not_found)
    }
    fn get_mut(&mut self, id: u64) -> Result<&mut Task, ActionError> {
        self.tasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(not_found)
    }
    /// Request cancellation; completion is reported only when the host acknowledges it.
    pub fn request_cancel(&mut self, id: u64) -> Result<&Task, ActionError> {
        let task = self.get_mut(id)?;
        if task.status.terminal() {
            return Ok(task);
        }
        if !task.cancellable {
            return Err(ActionError::new(
                ErrorCode::Unsupported,
                "This task does not support cancellation",
            ));
        }
        task.status = Status::Cancelling;
        Ok(task)
    }
    /// Enter a safe interruption point. False means cancelled/finished; do no more work.
    pub fn checkpoint(&mut self, id: u64) -> Result<bool, ActionError> {
        let task = self.get_mut(id)?;
        if task.status == Status::Cancelling {
            task.status = Status::Cancelled;
            task.error = Some(ActionError::new(
                ErrorCode::Cancelled,
                "Task cancelled at an interruption point",
            ));
            return Ok(false);
        }
        if task.status.terminal() {
            return Ok(false);
        }
        task.status = Status::Running;
        Ok(true)
    }
    /// Update progress; host work must still call checkpoint before each interruptible chunk.
    pub fn update(
        &mut self,
        id: u64,
        progress: Option<u16>,
        message: &str,
    ) -> Result<(), ActionError> {
        if progress.is_some_and(|p| p > 1000) || message.len() > 4096 {
            return Err(ActionError::new(
                ErrorCode::InvalidInput,
                "Invalid task progress",
            ));
        }
        let task = self.get_mut(id)?;
        if task.status.terminal() {
            return Err(ActionError::new(
                ErrorCode::Unavailable,
                "Task already finished",
            ));
        }
        task.progress = progress;
        task.message = message.into();
        Ok(())
    }
    /// Finish a task. A pending cancellation must first be resolved at a checkpoint.
    pub fn finish(&mut self, id: u64, result: ActionResult) -> Result<(), ActionError> {
        let task = self.get_mut(id)?;
        if task.status.terminal() || task.status == Status::Cancelling {
            return Err(ActionError::new(
                ErrorCode::Unavailable,
                "Task is finished or awaiting cancellation",
            ));
        }
        if result.task.is_some() {
            return Err(ActionError::new(
                ErrorCode::InvalidInput,
                "Task completion cannot defer itself",
            ));
        }
        task.status = Status::Complete;
        task.progress = Some(1000);
        task.result = Some(result);
        Ok(())
    }
    /// Fail a pending/running task with structured diagnostic details.
    pub fn fail(&mut self, id: u64, error: ActionError) -> Result<(), ActionError> {
        let task = self.get_mut(id)?;
        if task.status.terminal() {
            return Err(ActionError::new(
                ErrorCode::Unavailable,
                "Task already finished",
            ));
        }
        task.status = Status::Failed;
        task.error = Some(error);
        Ok(())
    }
    /// Remove only a terminal task, retaining monotonic IDs to prevent accidental reuse.
    pub fn forget(&mut self, id: u64) -> Result<(), ActionError> {
        let Some(index) = self.tasks.iter().position(|t| t.id == id) else {
            return Err(not_found());
        };
        if !self.tasks[index].status.terminal() {
            return Err(ActionError::new(
                ErrorCode::Unavailable,
                "Running tasks cannot be forgotten",
            ));
        }
        self.tasks.remove(index);
        Ok(())
    }
    /// One ID-ordered page of task metadata.
    pub fn list(&self, after: u64, limit: usize) -> Result<Value, ActionError> {
        if limit == 0 || limit > 100 {
            return Err(ActionError::new(
                ErrorCode::InvalidInput,
                "Task page limit is 1 to 100",
            ));
        }
        let tasks: Vec<&Task> = self
            .tasks
            .iter()
            .filter(|t| t.id > after)
            .take(limit + 1)
            .collect();
        let cursor = if tasks.len() > limit {
            Value::u64(tasks[limit - 1].id)
        } else {
            Value::Null
        };
        Ok(Value::object([
            (
                "tasks",
                Value::Array(tasks.into_iter().take(limit).map(Task::to_json).collect()),
            ),
            ("nextCursor", cursor),
        ]))
    }
}

fn not_found() -> ActionError {
    ActionError::new(ErrorCode::NotFound, "Task not found in this instance")
}
