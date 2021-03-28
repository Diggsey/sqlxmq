use std::borrow::Cow;
use std::fmt::Debug;
use std::time::Duration;

use serde::Serialize;
use sqlx::Postgres;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct TaskBuilder<'a> {
    id: Uuid,
    delay: Duration,
    channel_name: &'a str,
    channel_args: &'a str,
    retries: usize,
    retry_backoff: Duration,
    commit_interval: Option<Duration>,
    ordered: bool,
    name: &'a str,
    payload_json: Option<Cow<'a, str>>,
    payload_bytes: Option<&'a [u8]>,
}

impl<'a> TaskBuilder<'a> {
    pub fn new(name: &'a str) -> Self {
        Self::new_with_id(Uuid::new_v4(), name)
    }
    pub fn new_with_id(id: Uuid, name: &'a str) -> Self {
        Self {
            id,
            delay: Duration::from_secs(0),
            channel_name: "",
            channel_args: "",
            retries: 4,
            retry_backoff: Duration::from_secs(1),
            commit_interval: None,
            ordered: false,
            name,
            payload_json: None,
            payload_bytes: None,
        }
    }
    pub fn set_channel_name(&mut self, channel_name: &'a str) -> &mut Self {
        self.channel_name = channel_name;
        self
    }
    pub fn set_channel_args(&mut self, channel_args: &'a str) -> &mut Self {
        self.channel_args = channel_args;
        self
    }
    pub fn set_retries(&mut self, retries: usize) -> &mut Self {
        self.retries = retries;
        self
    }
    pub fn set_retry_backoff(&mut self, retry_backoff: Duration) -> &mut Self {
        self.retry_backoff = retry_backoff;
        self
    }
    pub fn set_commit_interval(&mut self, commit_interval: Option<Duration>) -> &mut Self {
        self.commit_interval = commit_interval;
        self
    }
    pub fn set_ordered(&mut self, ordered: bool) -> &mut Self {
        self.ordered = ordered;
        self
    }
    pub fn set_delay(&mut self, delay: Duration) -> &mut Self {
        self.delay = delay;
        self
    }
    pub fn set_raw_json(&mut self, raw_json: &'a str) -> &mut Self {
        self.payload_json = Some(Cow::Borrowed(raw_json));
        self
    }
    pub fn set_raw_bytes(&mut self, raw_bytes: &'a [u8]) -> &mut Self {
        self.payload_bytes = Some(raw_bytes);
        self
    }
    pub fn set_json<T: Serialize>(&mut self, value: &T) -> Result<&mut Self, serde_json::Error> {
        let value = serde_json::to_string(value)?;
        self.payload_json = Some(Cow::Owned(value));
        Ok(self)
    }
    pub async fn spawn<'b, E: sqlx::Executor<'b, Database = Postgres>>(
        &self,
        executor: E,
    ) -> Result<Uuid, sqlx::Error> {
        sqlx::query(
            "SELECT mq_insert(ARRAY[($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)::mq_new_t])",
        )
        .bind(self.id)
        .bind(self.delay)
        .bind(self.retries as i32)
        .bind(self.retry_backoff)
        .bind(self.channel_name)
        .bind(self.channel_args)
        .bind(self.commit_interval)
        .bind(self.ordered)
        .bind(self.name)
        .bind(self.payload_json.as_deref())
        .bind(self.payload_bytes)
        .execute(executor)
        .await?;
        Ok(self.id)
    }
}
