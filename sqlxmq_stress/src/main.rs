use std::env;
use std::error::Error;
use std::process::abort;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use futures::channel::mpsc;
use futures::StreamExt;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Postgres};
use sqlxmq::{job, CurrentJob, JobRegistry};
use tokio::task;

lazy_static! {
    static ref INSTANT_EPOCH: Instant = Instant::now();
    static ref CHANNEL: RwLock<mpsc::UnboundedSender<JobResult>> = RwLock::new(mpsc::unbounded().0);
}

struct JobResult {
    duration: Duration,
}

#[derive(Serialize, Deserialize)]
struct JobData {
    start_time: Duration,
}

// Arguments to the `#[job]` attribute allow setting default job options.
#[job(channel_name = "foo")]
async fn example_job(
    mut current_job: CurrentJob,
) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    // Decode a JSON payload
    let data: JobData = current_job.json()?.unwrap();

    // Mark the job as complete
    current_job.complete().await?;
    let end_time = INSTANT_EPOCH.elapsed();

    CHANNEL.read().unwrap().unbounded_send(JobResult {
        duration: end_time - data.start_time,
    })?;

    Ok(())
}

async fn start_job(
    pool: Pool<Postgres>,
    seed: usize,
) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    let channel_name = if seed % 3 == 0 { "foo" } else { "bar" };
    let channel_args = format!("{}", seed / 32);
    example_job
        .builder()
        // This is where we can override job configuration
        .set_channel_name(channel_name)
        .set_channel_args(&channel_args)
        .set_json(&JobData {
            start_time: INSTANT_EPOCH.elapsed(),
        })?
        .spawn(&pool)
        .await?;
    Ok(())
}

async fn schedule_tasks(num_jobs: usize, interval: Duration, pool: Pool<Postgres>) {
    let mut stream = tokio::time::interval(interval);
    for i in 0..num_jobs {
        let pool = pool.clone();
        task::spawn(async move {
            if let Err(e) = start_job(pool, i).await {
                eprintln!("Failed to start job: {:?}", e);
                abort();
            }
        });
        stream.tick().await;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = dotenv::dotenv();

    let pool = Pool::connect(&env::var("DATABASE_URL")?).await?;

    // Make sure the queues are empty
    sqlxmq::clear_all(&pool).await?;

    let registry = JobRegistry::new(&[example_job]);

    let _runner = registry
        .runner(&pool)
        .set_concurrency(50, 100)
        .run()
        .await?;
    let num_jobs = 10000;
    let interval = Duration::from_nanos(700_000);

    let (tx, rx) = mpsc::unbounded();
    *CHANNEL.write()? = tx;

    let start_time = Instant::now();
    task::spawn(schedule_tasks(num_jobs, interval, pool.clone()));

    let mut results: Vec<_> = rx.take(num_jobs).collect().await;
    let total_duration = start_time.elapsed();

    assert_eq!(results.len(), num_jobs);

    results.sort_by_key(|r| r.duration);
    let (min, max, median, pct) = (
        results[0].duration,
        results[num_jobs - 1].duration,
        results[num_jobs / 2].duration,
        results[(num_jobs * 19) / 20].duration,
    );
    let throughput = num_jobs as f64 / total_duration.as_secs_f64();

    println!("min: {}s", min.as_secs_f64());
    println!("max: {}s", max.as_secs_f64());
    println!("median: {}s", median.as_secs_f64());
    println!("95th percentile: {}s", pct.as_secs_f64());
    println!("throughput: {}/s", throughput);

    // The job runner will continue listening and running
    // jobs until `runner` is dropped.
    Ok(())
}
