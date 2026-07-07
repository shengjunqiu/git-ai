ALTER TABLE release_assets
ADD COLUMN IF NOT EXISTS version TEXT;

UPDATE release_assets ra
SET version = COALESCE(
    NULLIF(TRIM(ra.version), ''),
    (
        SELECT rc.version
        FROM release_channels rc
        WHERE rc.channel = ra.channel
    ),
    ra.channel
)
WHERE ra.version IS NULL OR TRIM(ra.version) = '';

ALTER TABLE release_assets
ALTER COLUMN version SET NOT NULL;

UPDATE release_assets
SET storage_path = CONCAT('releases/', version, '/', filename)
WHERE storage_path IS NULL OR TRIM(storage_path) = '';

DELETE FROM release_assets duplicate
USING release_assets kept
WHERE duplicate.version = kept.version
  AND duplicate.filename = kept.filename
  AND duplicate.id < kept.id;

ALTER TABLE release_assets
DROP CONSTRAINT IF EXISTS release_assets_channel_fkey;

ALTER TABLE release_assets
DROP CONSTRAINT IF EXISTS release_assets_channel_filename_key;

ALTER TABLE release_assets
ADD CONSTRAINT release_assets_version_filename_key UNIQUE (version, filename);

CREATE INDEX IF NOT EXISTS idx_release_assets_version
    ON release_assets(version);
