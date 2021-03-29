#![deny(missing_docs, unsafe_code)]
//! # sqlxmq
//!
//! A task queue built on `sqlx` and `PostgreSQL`.
//!
//! This library allows a CRUD application to run background tasks without complicating its
//! deployment. The only runtime dependency is `PostgreSQL`, so this is ideal for applications
//! already using a `PostgreSQL` database.
//!
//! Although using a SQL database as a task queue means compromising on latency of
//! delivered tasks, there are several show-stopping issues present in ordinary task
//! queues which are avoided altogether.
//!
//! With any other task queue, in-flight tasks are state that is not covered by normal
//! database backups. Even if tasks _are_ backed up, there is no way to restore both
//! a database and a task queue to a consistent point-in-time without manually
//! resolving conflicts.
//!
//! By storing tasks in the database, existing backup procedures will store a perfectly
//! consistent state of both in-flight tasks and persistent data. Additionally, tasks can
//! be spawned and completed as part of other transactions, making it easy to write correct
//! application code.
//!
//! Leveraging the power of `PostgreSQL`, this task queue offers several features not
//! present in other task queues.
//!
//! # Features
//!
//! - **Send/receive multiple tasks at once.**
//!
//!   This reduces the number of queries to the database.
//!
//! - **Send tasks to be executed at a future date and time.**
//!
//!   Avoids the need for a separate scheduling system.
//!
//! - **Reliable delivery of tasks.**
//!
//! - **Automatic retries with exponential backoff.**
//!
//!   Number of retries and initial backoff parameters are configurable.
//!
//! - **Transactional sending of tasks.**
//!
//!   Avoids sending spurious tasks if a transaction is rolled back.
//!
//! - **Transactional completion of tasks.**
//!
//!   If all side-effects of a task are updates to the database, this provides
//!   true exactly-once execution of tasks.
//!
//! - **Transactional check-pointing of tasks.**
//!
//!   Long-running tasks can check-point their state to avoid having to restart
//!   from the beginning if there is a failure: the next retry can continue
//!   from the last check-point.
//!
//! - **Opt-in strictly ordered task delivery.**
//!
//!   Tasks within the same channel will be processed strictly in-order
//!   if this option is enabled for the task.
//!
//! - **Fair task delivery.**
//!
//!   A channel with a lot of tasks ready to run will not starve a channel with fewer
//!   tasks.
//!
//! - **Opt-in two-phase commit.**
//!
//!   This is particularly useful on an ordered channel where a position can be "reserved"
//!   in the task order, but not committed until later.
//!
//! - **JSON and/or binary payloads.**
//!
//!   Tasks can use whichever is most convenient.
//!
//! - **Automatic keep-alive of tasks.**
//!
//!   Long-running tasks will automatically be "kept alive" to prevent them being
//!   retried whilst they're still ongoing.
//!
//! - **Concurrency limits.**
//!
//!   Specify the minimum and maximum number of concurrent tasks each runner should
//!   handle.
//!
//! - **Built-in task registry via an attribute macro.**
//!
//!   Tasks can be easily registered with a runner, and default configuration specified
//!   on a per-task basis.
//!
//! - **Implicit channels.**
//!
//!   Channels are implicitly created and destroyed when tasks are sent and processed,
//!   so no setup is required.
//!
//! - **Channel groups.**
//!
//!   Easily subscribe to multiple channels at once, thanks to the separation of
//!   channel name and channel arguments.
//!
//! - **NOTIFY-based polling.**
//!
//!   This saves resources when few tasks are being processed.
//!
//! # Getting started
//!
//! ## Defining tasks
//!
//! The first step is to define a function to be run on the task queue.
//!
//! ```rust
//! use sqlxmq::{task, CurrentTask};
//!
//! // Arguments to the `#[task]` attribute allow setting default task options.
//! #[task(channel_name = "foo")]
//! async fn example_task(
//!     mut current_task: CurrentTask,
//! ) -> sqlx::Result<()> {
//!     // Decode a JSON payload
//!     let who: Option<String> = current_task.json()?;
//!
//!     // Do some work
//!     println!("Hello, {}!", who.as_deref().unwrap_or("world"));
//!
//!     // Mark the task as complete
//!     current_task.complete().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Listening for tasks
//!
//! Next we need to create a task runner: this is what listens for new tasks
//! and executes them.
//!
//! ```rust
//! use sqlxmq::TaskRegistry;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn Error>> {
//!     // You'll need to provide a Postgres connection pool.
//!     let pool = connect_to_db().await?;
//!
//!     // Construct a task registry from our single task.
//!     let mut registry = TaskRegistry::new(&[example_task]);
//!     // Here is where you can configure the registry
//!     // registry.set_error_handler(...)
//!
//!     let runner = registry
//!         // Create a task runner using the connection pool.
//!         .runner(&pool)
//!         // Here is where you can configure the task runner
//!         // Aim to keep 10-20 tasks running at a time.
//!         .set_concurrency(10, 20)
//!         // Start the task runner in the background.
//!         .run()
//!         .await?;
//!
//!     // The task runner will continue listening and running
//!     // tasks until `runner` is dropped.
//! }
//! ```
//!
//! ## Spawning a task
//!
//! The final step is to actually run a task.
//!
//! ```rust
//! example_task.new()
//!     // This is where we override task configuration
//!     .set_channel_name("bar")
//!     .set_json("John")
//!     .spawn(&pool)
//!     .await?;
//! ```

#[doc(hidden)]
pub mod hidden;
mod registry;
mod runner;
mod spawn;
mod utils;

pub use registry::*;
pub use runner::*;
pub use spawn::*;
pub use sqlxmq_macros::task;
pub use utils::OwnedTask;

#[cfg(test)]
mod tests {
    use super::*;
    use crate as sqlxmq;

    use std::env;
    use std::error::Error;
    use std::future::Future;
    use std::ops::Deref;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Once};
    use std::time::Duration;

    use futures::channel::mpsc;
    use futures::StreamExt;
    use sqlx::{Pool, Postgres};
    use tokio::sync::{Mutex, MutexGuard};
    use tokio::task;

    struct TestGuard<T>(MutexGuard<'static, ()>, T);

    impl<T> Deref for TestGuard<T> {
        type Target = T;

        fn deref(&self) -> &T {
            &self.1
        }
    }

    async fn test_pool() -> TestGuard<Pool<Postgres>> {
        static INIT_LOGGER: Once = Once::new();
        static TEST_MUTEX: Mutex<()> = Mutex::const_new(());

        let guard = TEST_MUTEX.lock().await;

        let _ = dotenv::dotenv();

        INIT_LOGGER.call_once(|| pretty_env_logger::init());

        let pool = Pool::connect(&env::var("DATABASE_URL").unwrap())
            .await
            .unwrap();

        sqlx::query("TRUNCATE TABLE mq_payloads")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM mq_msgs WHERE id != uuid_nil()")
            .execute(&pool)
            .await
            .unwrap();

        TestGuard(guard, pool)
    }

    async fn test_task_runner<F: Future + Send + 'static>(
        pool: &Pool<Postgres>,
        f: impl (Fn(CurrentTask) -> F) + Send + Sync + 'static,
    ) -> (OwnedTask, Arc<AtomicUsize>)
    where
        F::Output: Send + 'static,
    {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter2 = counter.clone();
        let runner = TaskRunnerOptions::new(pool, move |task| {
            counter2.fetch_add(1, Ordering::SeqCst);
            task::spawn(f(task));
        })
        .run()
        .await
        .unwrap();
        (runner, counter)
    }

    fn task_proto<'a, 'b>(builder: &'a mut TaskBuilder<'b>) -> &'a mut TaskBuilder<'b> {
        builder.set_channel_name("bar")
    }

    #[task(channel_name = "foo", ordered, retries = 3, backoff_secs = 2.0)]
    async fn example_task1(
        mut current_task: CurrentTask,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        current_task.complete().await?;
        Ok(())
    }

    #[task(proto(task_proto))]
    async fn example_task2(
        mut current_task: CurrentTask,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        current_task.complete().await?;
        Ok(())
    }

    async fn named_task_runner(pool: &Pool<Postgres>) -> OwnedTask {
        TaskRegistry::new(&[example_task1, example_task2])
            .runner(pool)
            .run()
            .await
            .unwrap()
    }

    async fn pause() {
        pause_ms(100).await;
    }

    async fn pause_ms(ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    #[tokio::test]
    async fn it_can_spawn_task() {
        let pool = &*test_pool().await;
        let (_runner, counter) =
            test_task_runner(&pool, |mut task| async move { task.complete().await }).await;

        assert_eq!(counter.load(Ordering::SeqCst), 0);
        TaskBuilder::new("foo").spawn(pool).await.unwrap();
        pause().await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn it_runs_tasks_in_order() {
        let pool = &*test_pool().await;
        let (tx, mut rx) = mpsc::unbounded();

        let (_runner, counter) = test_task_runner(&pool, move |task| {
            let tx = tx.clone();
            async move {
                tx.unbounded_send(task).unwrap();
            }
        })
        .await;

        assert_eq!(counter.load(Ordering::SeqCst), 0);
        TaskBuilder::new("foo")
            .set_ordered(true)
            .spawn(pool)
            .await
            .unwrap();
        TaskBuilder::new("bar")
            .set_ordered(true)
            .spawn(pool)
            .await
            .unwrap();

        pause().await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        let mut task = rx.next().await.unwrap();
        task.complete().await.unwrap();

        pause().await;
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn it_runs_tasks_in_parallel() {
        let pool = &*test_pool().await;
        let (tx, mut rx) = mpsc::unbounded();

        let (_runner, counter) = test_task_runner(&pool, move |task| {
            let tx = tx.clone();
            async move {
                tx.unbounded_send(task).unwrap();
            }
        })
        .await;

        assert_eq!(counter.load(Ordering::SeqCst), 0);
        TaskBuilder::new("foo").spawn(pool).await.unwrap();
        TaskBuilder::new("bar").spawn(pool).await.unwrap();

        pause().await;
        assert_eq!(counter.load(Ordering::SeqCst), 2);

        for _ in 0..2 {
            let mut task = rx.next().await.unwrap();
            task.complete().await.unwrap();
        }
    }

    #[tokio::test]
    async fn it_retries_failed_tasks() {
        let pool = &*test_pool().await;
        let (_runner, counter) = test_task_runner(&pool, move |_| async {}).await;

        let backoff = 200;

        assert_eq!(counter.load(Ordering::SeqCst), 0);
        TaskBuilder::new("foo")
            .set_retry_backoff(Duration::from_millis(backoff))
            .set_retries(2)
            .spawn(pool)
            .await
            .unwrap();

        // First attempt
        pause().await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        // Second attempt
        pause_ms(backoff).await;
        pause().await;
        assert_eq!(counter.load(Ordering::SeqCst), 2);

        // Third attempt
        pause_ms(backoff * 2).await;
        pause().await;
        assert_eq!(counter.load(Ordering::SeqCst), 3);

        // No more attempts
        pause_ms(backoff * 5).await;
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn it_can_checkpoint_tasks() {
        let pool = &*test_pool().await;
        let (_runner, counter) = test_task_runner(&pool, move |mut current_task| async move {
            let state: bool = current_task.json().unwrap().unwrap();
            if state {
                current_task.complete().await.unwrap();
            } else {
                current_task
                    .checkpoint(Checkpoint::new().set_json(&true).unwrap())
                    .await
                    .unwrap();
            }
        })
        .await;

        let backoff = 200;

        assert_eq!(counter.load(Ordering::SeqCst), 0);
        TaskBuilder::new("foo")
            .set_retry_backoff(Duration::from_millis(backoff))
            .set_retries(5)
            .set_json(&false)
            .unwrap()
            .spawn(pool)
            .await
            .unwrap();

        // First attempt
        pause().await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        // Second attempt
        pause_ms(backoff).await;
        pause().await;
        assert_eq!(counter.load(Ordering::SeqCst), 2);

        // No more attempts
        pause_ms(backoff * 3).await;
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn it_can_use_registry() {
        let pool = &*test_pool().await;
        let _runner = named_task_runner(pool).await;

        example_task1.new().spawn(pool).await.unwrap();
        example_task2.new().spawn(pool).await.unwrap();
        pause().await;
    }
}
