#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();
    let db = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").unwrap()).await?;

    sleep.builder().set_json(&10)?.spawn(&db).await?;

    let handle = sqlxmq::JobRegistry::new(&[sleep]).runner(&db).run().await?;

    println!("Press Ctrl+C to send a stop signal");
    wait_signal()?;
    println!("Got a stop signal, waiting for jobs to finish ...");

    // this would just instantly kill the job
    // handle.stop().await;

    // this waits for the job to finish, but hangs forever afterwards
    handle.into_inner().await?;

    Ok(())
}

fn wait_signal() -> std::io::Result<()> {
    use signal_hook::consts::signal::*;
    use signal_hook::iterator::Signals;

    let mut signals = Signals::new(&[SIGTERM, SIGINT, SIGQUIT])?;
    let handle = signals.handle();
    for signal in signals.forever() {
        match signal {
            SIGTERM | SIGINT | SIGQUIT => break,
            _ => (),
        }
    }
    handle.close();
    Ok(())
}

#[sqlxmq::job]
pub async fn sleep(mut job: sqlxmq::CurrentJob) -> sqlx::Result<()> {
    let second = std::time::Duration::from_secs(1);
    let mut to_sleep: u64 = job.json().unwrap().unwrap();
    while to_sleep > 0 {
        println!("job#{} {to_sleep} more seconds to sleep ...", job.id());
        tokio::time::sleep(second).await;
        to_sleep -= 1;
    }
    job.complete().await
}
