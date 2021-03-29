use std::collections::HashMap;
use std::error::Error;
use std::fmt::Display;
use std::future::Future;
use std::sync::Arc;

use sqlx::{Pool, Postgres};
use uuid::Uuid;

use crate::hidden::{BuildFn, RunFn};
use crate::utils::Opaque;
use crate::{TaskBuilder, TaskRunnerOptions};

/// Stores a mapping from task name to task. Can be used to construct
/// a task runner.
pub struct TaskRegistry {
    error_handler: Arc<dyn Fn(&str, Box<dyn Error + Send + 'static>) + Send + Sync>,
    task_map: HashMap<&'static str, &'static NamedTask>,
}

/// Error returned when a task is received whose name is not in the registry.
#[derive(Debug)]
pub struct UnknownTaskError;

impl Error for UnknownTaskError {}
impl Display for UnknownTaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Unknown task")
    }
}

impl TaskRegistry {
    /// Construct a new task registry from the provided task list.
    pub fn new(tasks: &[&'static NamedTask]) -> Self {
        let mut task_map = HashMap::new();
        for &task in tasks {
            if task_map.insert(task.name(), task).is_some() {
                panic!("Duplicate task registered: {}", task.name());
            }
        }
        Self {
            error_handler: Arc::new(Self::default_error_handler),
            task_map,
        }
    }

    /// Set a function to be called whenever a task returns an error.
    pub fn set_error_handler(
        &mut self,
        error_handler: impl Fn(&str, Box<dyn Error + Send + 'static>) + Send + Sync + 'static,
    ) -> &mut Self {
        self.error_handler = Arc::new(error_handler);
        self
    }

    /// Look-up a task by name.
    pub fn resolve_task(&self, name: &str) -> Option<&'static NamedTask> {
        self.task_map.get(name).copied()
    }

    /// The default error handler implementation, which simply logs the error.
    pub fn default_error_handler(name: &str, error: Box<dyn Error + Send + 'static>) {
        log::error!("Task {} failed: {}", name, error);
    }

    #[doc(hidden)]
    pub fn spawn_internal<E: Into<Box<dyn Error + Send + Sync + 'static>>>(
        &self,
        name: &'static str,
        f: impl Future<Output = Result<(), E>> + Send + 'static,
    ) {
        let error_handler = self.error_handler.clone();
        tokio::spawn(async move {
            if let Err(e) = f.await {
                error_handler(name, e.into());
            }
        });
    }

    /// Construct a task runner from this registry and the provided connection
    /// pool.
    pub fn runner(self, pool: &Pool<Postgres>) -> TaskRunnerOptions {
        TaskRunnerOptions::new(pool, move |current_task| {
            if let Some(task) = self.resolve_task(current_task.name()) {
                (task.run_fn.0 .0)(&self, current_task);
            } else {
                (self.error_handler)(current_task.name(), Box::new(UnknownTaskError))
            }
        })
    }
}

/// Type for a named task. Functions annotated with `#[task]` are
/// transformed into static variables whose type is `&'static NamedTask`.
#[derive(Debug)]
pub struct NamedTask {
    name: &'static str,
    build_fn: Opaque<BuildFn>,
    run_fn: Opaque<RunFn>,
}

impl NamedTask {
    #[doc(hidden)]
    pub const fn new_internal(name: &'static str, build_fn: BuildFn, run_fn: RunFn) -> Self {
        Self {
            name,
            build_fn: Opaque(build_fn),
            run_fn: Opaque(run_fn),
        }
    }
    /// Initialize a task builder with the name and defaults of this task.
    pub fn new(&self) -> TaskBuilder<'static> {
        let mut builder = TaskBuilder::new(self.name);
        (self.build_fn.0 .0)(&mut builder);
        builder
    }
    /// Initialize a task builder with the name and defaults of this task,
    /// using the provided task ID.
    pub fn new_with_id(&self, id: Uuid) -> TaskBuilder<'static> {
        let mut builder = TaskBuilder::new_with_id(id, self.name);
        (self.build_fn.0 .0)(&mut builder);
        builder
    }

    /// Returns the name of this task.
    pub const fn name(&self) -> &'static str {
        self.name
    }
}
