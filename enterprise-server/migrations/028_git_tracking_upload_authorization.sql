-- Require an explicit administrator grant before a developer can upload
-- Git tracking data. Existing and newly-created organization memberships
-- start unauthorized.

ALTER TABLE org_members
  ADD COLUMN IF NOT EXISTS git_tracking_upload_enabled BOOLEAN NOT NULL DEFAULT false,
  ADD COLUMN IF NOT EXISTS git_tracking_upload_authorized_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS git_tracking_upload_authorized_by UUID REFERENCES users(id) ON DELETE SET NULL;

-- Summary payload labels are client-provided metadata. Persist the authenticated
-- identity as the authoritative ownership boundary for all new uploads.
ALTER TABLE summary_uploads
  ADD COLUMN IF NOT EXISTS user_id UUID REFERENCES users(id) ON DELETE SET NULL,
  ADD COLUMN IF NOT EXISTS org_id UUID REFERENCES organizations(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_summary_uploads_user_id ON summary_uploads(user_id);
CREATE INDEX IF NOT EXISTS idx_summary_uploads_org_id ON summary_uploads(org_id);
