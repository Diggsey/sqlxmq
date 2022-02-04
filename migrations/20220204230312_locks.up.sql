-- Add up migration script here
ALTER TABLE mq_msgs
ADD COLUMN lock_number BIGINT;

CREATE OR REPLACE FUNCTION mq_insert(new_messages mq_new_t[])
RETURNS VOID AS $$
BEGIN
    PERFORM pg_notify(CONCAT('mq_', channel_name), '')
    FROM unnest(new_messages) AS new_msgs
    GROUP BY channel_name;

    IF FOUND THEN
        PERFORM pg_notify('mq', '');
    END IF;

    INSERT INTO mq_payloads (
        id,
        name,
        payload_json,
        payload_bytes
    ) SELECT
        id,
        name,
        payload_json::JSONB,
        payload_bytes
    FROM UNNEST(new_messages);

    INSERT INTO mq_msgs (
        id,
        lock_number,
        attempt_at,
        attempts,
        retry_backoff,
        channel_name,
        channel_args,
        commit_interval,
        after_message_id
    )
    SELECT
        id,
        ('x' || translate(id::text, '-', ''))::bit(64)::bigint,
        NOW() + delay + COALESCE(commit_interval, INTERVAL '0'),
        retries + 1,
        retry_backoff,
        channel_name,
        channel_args,
        commit_interval,
        CASE WHEN ordered
            THEN
                LAG(id, 1, mq_latest_message(channel_name, channel_args))
                OVER (PARTITION BY channel_name, channel_args, ordered ORDER BY id)
            ELSE
                NULL
            END
    FROM UNNEST(new_messages);
END;
$$ LANGUAGE plpgsql;

UPDATE mq_msgs SET lock_number = ('x' || translate(id::text, '-', ''))::bit(64)::bigint WHERE lock_number IS NULL ;


-- Main entry-point for job runner: pulls a batch of messages from the queue.
CREATE FUNCTION mq_poll_locking(channel_names TEXT[], batch_size INT DEFAULT 1)
RETURNS TABLE(
    id UUID,
    is_committed BOOLEAN,
    name TEXT,
    payload_json TEXT,
    payload_bytes BYTEA,
    retry_backoff INTERVAL,
    wait_time INTERVAL
) AS $$
BEGIN
    RETURN QUERY UPDATE mq_msgs
    SET
        attempt_at = CASE WHEN mq_msgs.attempts = 1 THEN NULL ELSE NOW() + mq_msgs.retry_backoff END,
        attempts = mq_msgs.attempts - 1,
        retry_backoff = mq_msgs.retry_backoff * 2
    FROM (
        SELECT
            msgs.id,
            msgs.lock_number
        FROM mq_active_channels(channel_names, batch_size) AS active_channels
        INNER JOIN LATERAL (
            SELECT * FROM mq_msgs
            WHERE mq_msgs.id != uuid_nil()
            AND mq_msgs.attempt_at <= NOW()
            AND mq_msgs.channel_name = active_channels.name
            AND mq_msgs.channel_args = active_channels.args
            AND NOT mq_uuid_exists(mq_msgs.after_message_id)
            ORDER BY mq_msgs.attempt_at ASC
            LIMIT batch_size
        ) AS msgs ON TRUE
        WHERE pg_try_advisory_xact_lock(lock_number)
        LIMIT batch_size
    ) AS messages_to_update
    LEFT JOIN mq_payloads ON mq_payloads.id = messages_to_update.id
    WHERE mq_msgs.id = messages_to_update.id 
    RETURNING
        mq_msgs.id,
        mq_msgs.commit_interval IS NULL,
        mq_payloads.name,
        mq_payloads.payload_json::TEXT,
        mq_payloads.payload_bytes,
        mq_msgs.retry_backoff / 2,
        interval '0' AS wait_time;

    IF NOT FOUND THEN
        RETURN QUERY SELECT
            NULL::UUID,
            NULL::BOOLEAN,
            NULL::TEXT,
            NULL::TEXT,
            NULL::BYTEA,
            NULL::INTERVAL,
            MIN(mq_msgs.attempt_at) - NOW()
        FROM mq_msgs
        WHERE mq_msgs.id != uuid_nil()
        AND NOT mq_uuid_exists(mq_msgs.after_message_id)
        AND (channel_names IS NULL OR mq_msgs.channel_name = ANY(channel_names));
    END IF;
END;
$$ LANGUAGE plpgsql;

ALTER TABLE mq_msgs
    ALTER COLUMN lock_number SET NOT NULL
