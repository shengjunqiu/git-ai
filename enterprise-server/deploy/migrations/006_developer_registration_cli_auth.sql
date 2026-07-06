-- Developer registration and CLI authorization groundwork.
-- Adds account-login fields, organization-domain registration rules,
-- browser web sessions, and one-time CLI authorization codes.

ALTER TABLE users
  ADD COLUMN IF NOT EXISTS password_hash TEXT,
  ADD COLUMN IF NOT EXISTS email_verified_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS default_org_id UUID REFERENCES organizations(id) ON DELETE SET NULL,
  ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'active';

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1
    FROM pg_constraint
    WHERE conname = 'users_status_check'
      AND conrelid = 'users'::regclass
  ) THEN
    ALTER TABLE users
      ADD CONSTRAINT users_status_check
      CHECK (status IN ('active', 'disabled'));
  END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_users_default_org_id
  ON users(default_org_id);

CREATE INDEX IF NOT EXISTS idx_users_status
  ON users(status);

CREATE TABLE IF NOT EXISTS organization_domains (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  org_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  domain TEXT NOT NULL,
  verified BOOLEAN NOT NULL DEFAULT false,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(org_id, domain),
  CHECK (domain = lower(domain))
);

CREATE INDEX IF NOT EXISTS idx_organization_domains_domain
  ON organization_domains(domain);

CREATE INDEX IF NOT EXISTS idx_organization_domains_org_id
  ON organization_domains(org_id);

CREATE TABLE IF NOT EXISTS web_sessions (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  session_token_hash TEXT UNIQUE NOT NULL,
  expires_at TIMESTAMPTZ NOT NULL,
  revoked_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_seen_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_web_sessions_user_id
  ON web_sessions(user_id);

CREATE INDEX IF NOT EXISTS idx_web_sessions_token_hash
  ON web_sessions(session_token_hash);

CREATE INDEX IF NOT EXISTS idx_web_sessions_expires_at
  ON web_sessions(expires_at);

CREATE TABLE IF NOT EXISTS authorization_codes (
  code_hash TEXT PRIMARY KEY,
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  client_id TEXT NOT NULL,
  redirect_uri TEXT NOT NULL,
  code_challenge TEXT NOT NULL,
  code_challenge_method TEXT NOT NULL,
  expires_at TIMESTAMPTZ NOT NULL,
  consumed_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_authorization_codes_user_id
  ON authorization_codes(user_id);

CREATE INDEX IF NOT EXISTS idx_authorization_codes_expires_at
  ON authorization_codes(expires_at);

UPDATE users u
SET default_org_id = om.org_id
FROM org_members om
WHERE om.user_id = u.id
  AND (u.personal_org_id IS NULL OR om.org_id <> u.personal_org_id)
  AND u.default_org_id IS NULL;
