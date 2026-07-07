ALTER TABLE developer_client_status
ADD COLUMN IF NOT EXISTS device_key TEXT;

UPDATE developer_client_status
SET device_key = COALESCE(NULLIF(TRIM(distinct_id), ''), NULLIF(TRIM(hostname), ''), 'unknown')
WHERE device_key IS NULL OR device_key = '';

ALTER TABLE developer_client_status
ALTER COLUMN device_key SET NOT NULL;

ALTER TABLE developer_client_status
DROP CONSTRAINT IF EXISTS developer_client_status_pkey;

ALTER TABLE developer_client_status
ADD PRIMARY KEY (user_id, device_key);

CREATE INDEX IF NOT EXISTS idx_developer_client_status_user
    ON developer_client_status(user_id);

CREATE INDEX IF NOT EXISTS idx_developer_client_status_org_user
    ON developer_client_status(org_id, user_id);
