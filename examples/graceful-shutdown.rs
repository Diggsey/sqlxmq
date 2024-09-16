use sqlxmq::{job, CurrentJob, JobRegistry};
use std::time::Duration;
use tokio::time;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let db = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").unwrap()).await?;

    sleep.builder().set_json(&5u64)?.spawn(&db).await?;

    let mut handle = JobRegistry::new(&[sleep]).runner(&db).run().await?;

    // Let's emulate a stop signal in a couple of seconts after running the job
    time::sleep(Duration::from_secs(2)).await;
    println!("A stop signal received");

    // Stop listening for new jobs
    handle.stop().await;

    // Wait for the running jobs to stop for maximum 10 seconds
    handle.wait_jobs_finish(Duration::from_secs(10)).await;

    Ok(())
}

#[job]
pub async fn sleep(mut job: CurrentJob) -> sqlx::Result<()> {
    let second = Duration::from_secs(1);
    let mut to_sleep: u64 = job.json().unwrap().unwrap();
    while to_sleep > 0 {
        println!("job#{} {to_sleep} more seconds to sleep ...", job.id());
        time::sleep(second).await;
        to_sleep -= 1;
    }
    job.complete().await
}
