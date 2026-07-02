-- Data isolation: Add org_id and user_id to projects table
-- This allows filtering report data (projects, commit_stats, tool_model_stats) by organization and user.

-- Add org_id UUID column referencing organizations
ALTER TABLE projects ADD COLUMN IF NOT EXISTS org_id UUID REFERENCES organizations(id) ON DELETE SET NULL;

-- Add user_id UUID column referencing users  
ALTER TABLE projects ADD COLUMN IF NOT EXISTS user_id UUID REFERENCES users(id) ON DELETE SET NULL;

-- Create indexes for efficient filtering
CREATE INDEX IF NOT EXISTS idx_projects_org_id ON projects(org_id);
CREATE INDEX IF NOT EXISTS idx_projects_user_id ON projects(user_id);

-- Backfill: try to match existing projects.organization text to organizations.slug
UPDATE projects p
SET org_id = o.id
FROM organizations o
WHERE p.organization IS NOT NULL
  AND p.org_id IS NULL
  AND (p.organization = o.name OR p.organization = o.slug);
