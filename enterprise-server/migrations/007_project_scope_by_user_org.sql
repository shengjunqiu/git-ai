-- Scope report projects by authenticated user and organization.
-- The previous global remote_url_hash uniqueness caused one developer's report
-- upload for a repository to overwrite another developer/org's project scope.

DO $$
BEGIN
  IF EXISTS (
    SELECT 1
    FROM pg_constraint
    WHERE conname = 'projects_remote_url_hash_key'
      AND conrelid = 'projects'::regclass
  ) THEN
    ALTER TABLE projects
      DROP CONSTRAINT projects_remote_url_hash_key;
  END IF;
END $$;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1
    FROM pg_constraint
    WHERE conname = 'projects_remote_url_hash_org_user_key'
      AND conrelid = 'projects'::regclass
  ) THEN
    ALTER TABLE projects
      ADD CONSTRAINT projects_remote_url_hash_org_user_key
      UNIQUE (remote_url_hash, org_id, user_id);
  END IF;
END $$;
