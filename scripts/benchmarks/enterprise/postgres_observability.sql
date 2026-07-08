-- Enterprise benchmark Postgres observability queries.
--
-- pg_stat_statements requires PostgreSQL to start with:
-- shared_preload_libraries=pg_stat_statements
--
-- For docker compose environments in this repo, restart postgres after pulling
-- the compose change, then run:
-- docker compose exec -T postgres psql -U gitai -d gitai_enterprise \
--   -f /path/to/postgres_observability.sql

CREATE EXTENSION IF NOT EXISTS pg_stat_statements;

SELECT
    name,
    setting
FROM pg_settings
WHERE name IN (
    'shared_preload_libraries',
    'pg_stat_statements.track',
    'pg_stat_statements.max'
)
ORDER BY name;

SELECT
    extname,
    extversion
FROM pg_extension
WHERE extname = 'pg_stat_statements';

-- Slowest statements by mean execution time in the current database.
SELECT
    calls,
    ROUND(mean_exec_time::numeric, 2) AS mean_exec_ms,
    ROUND(max_exec_time::numeric, 2) AS max_exec_ms,
    ROUND(total_exec_time::numeric, 2) AS total_exec_ms,
    rows,
    LEFT(REGEXP_REPLACE(query, '\s+', ' ', 'g'), 240) AS query
FROM pg_stat_statements
WHERE dbid = (
    SELECT oid
    FROM pg_database
    WHERE datname = current_database()
)
ORDER BY mean_exec_time DESC
LIMIT 20;

-- Query families with the highest total DB time.
SELECT
    calls,
    ROUND(total_exec_time::numeric, 2) AS total_exec_ms,
    ROUND(mean_exec_time::numeric, 2) AS mean_exec_ms,
    ROUND(max_exec_time::numeric, 2) AS max_exec_ms,
    rows,
    LEFT(REGEXP_REPLACE(query, '\s+', ' ', 'g'), 240) AS query
FROM pg_stat_statements
WHERE dbid = (
    SELECT oid
    FROM pg_database
    WHERE datname = current_database()
)
ORDER BY total_exec_time DESC
LIMIT 20;

-- Current connection states for capacity checks during benchmark runs.
SELECT
    COALESCE(state, 'unknown') AS state,
    COUNT(*) AS connections
FROM pg_stat_activity
WHERE datname = current_database()
GROUP BY COALESCE(state, 'unknown')
ORDER BY state;

-- Active queries running longer than one second.
SELECT
    pid,
    state,
    NOW() - query_start AS age,
    wait_event_type,
    wait_event,
    LEFT(REGEXP_REPLACE(query, '\s+', ' ', 'g'), 240) AS query
FROM pg_stat_activity
WHERE datname = current_database()
  AND state <> 'idle'
  AND query_start IS NOT NULL
  AND NOW() - query_start > INTERVAL '1 second'
ORDER BY age DESC;
