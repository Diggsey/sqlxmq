# sqlxmq

A task queue built on `sqlx` and `PostgreSQL`.

This library allows a CRUD application to run background tasks without complicating its
deployment. The only runtime dependency is `PostgreSQL`, so this is ideal for applications
already using a `PostgreSQL` database.

Although using a SQL database as a task queue means compromising on latency of
delivered tasks, there are several show-stopping issues present in ordinary task
queues which are avoided altogether.

With any other task queue, in-flight tasks are state that is not covered by normal
database backups. Even if tasks _are_ backed up, there is no way to restore both
a database and a task queue to a consistent point-in-time without manually
resolving conflicts.

By storing tasks in the database, existing backup procedures will store a perfectly
consistent state of both in-flight tasks and persistent data. Additionally, tasks can
be spawned and completed as part of other transactions, making it easy to write correct
application code.

Leveraging the power of `PostgreSQL`, this task queue offers several features not
present in other task queues.

# Features

- **Send/receive multiple tasks at once.**

  This reduces the number of queries to the database.

- **Send tasks to be executed at a future date and time.**

  Avoids the need for a separate scheduling system.

- **Reliable delivery of tasks.**

- **Automatic retries with exponential backoff.**

  Number of retries and initial backoff parameters are configurable.

- **Transactional sending of tasks.**

  Avoids sending spurious tasks if a transaction is rolled back.

- **Transactional completion of tasks.**

  If all side-effects of a task are updates to the database, this provides
  true exactly-once execution of tasks.

- **Transactional check-pointing of tasks.**

  Long-running tasks can check-point their state to avoid having to restart
  from the beginning if there is a failure: the next retry can continue
  from the last check-point.

- **Opt-in strictly ordered task delivery.**

  Tasks within the same channel will be processed strictly in-order
  if this option is enabled for the task.

- **Fair task delivery.**

  A channel with a lot of tasks ready to run will not starve a channel with fewer
  tasks.

- **Opt-in two-phase commit.**

  This is particularly useful on an ordered channel where a position can be "reserved"
  in the task order, but not committed until later.

- **JSON and/or binary payloads.**

  Tasks can use whichever is most convenient.

- **Automatic keep-alive of tasks.**

  Long-running tasks will automatically be "kept alive" to prevent them being
  retried whilst they're still ongoing.

- **Concurrency limits.**

  Specify the minimum and maximum number of concurrent tasks each runner should
  handle.

- **Built-in task registry via an attribute macro.**

  Tasks can be easily registered with a runner, and default configuration specified
  on a per-task basis.

- **Implicit channels.**

  Channels are implicitly created and destroyed when tasks are sent and processed,
  so no setup is required.

- **Channel groups.**

  Easily subscribe to multiple channels at once, thanks to the separation of
  channel name and channel arguments.

- **NOTIFY-based polling.**

  This saves resources when few tasks are being processed.

# Getting started

## Defining tasks

The first step is to define a function to be run on the task queue.

```rust
use sqlxmq::{task, CurrentTask};

// Arguments to the `#[task]` attribute allow setting default task options.
#[task(channel_name = "foo")]
async fn example_task(
    mut current_task: CurrentTask,
) -> sqlx::Result<()> {
    // Decode a JSON payload
    let who: Option<String> = current_task.json()?;

    // Do some work
    println!("Hello, {}!", who.as_deref().unwrap_or("world"));

    // Mark the task as complete
    current_task.complete().await?;

    Ok(())
}
```

## Listening for tasks

Next we need to create a task runner: this is what listens for new tasks
and executes them.

```rust
use sqlxmq::TaskRegistry;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // You'll need to provide a Postgres connection pool.
    let pool = connect_to_db().await?;

    // Construct a task registry from our single task.
    let mut registry = TaskRegistry::new(&[example_task]);
    // Here is where you can configure the registry
    // registry.set_error_handler(...)

    let runner = registry
        // Create a task runner using the connection pool.
        .runner(&pool)
        // Here is where you can configure the task runner
        // Aim to keep 10-20 tasks running at a time.
        .set_concurrency(10, 20)
        // Start the task runner in the background.
        .run()
        .await?;

    // The task runner will continue listening and running
    // tasks until `runner` is dropped.
}
```

## Spawning a task

The final step is to actually run a task.

```rust
example_task.new()
    // This is where we override task configuration
    .set_channel_name("bar")
    .set_json("John")
    .spawn(&pool)
    .await?;
```
