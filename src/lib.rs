mod runner;
mod spawn;
mod utils;

pub use runner::*;
pub use spawn::*;
pub use utils::OwnedTask;

#[cfg(test)]
mod tests {
    use super::*;

    use std::env;
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

    async fn pause() {
        pause_ms(50).await;
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

        let backoff = 100;

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
}
