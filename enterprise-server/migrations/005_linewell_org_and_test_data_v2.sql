-- Migration 005 v2: Set up linewell.com as the primary organization
-- Fixes UUID conflicts from v1 and properly migrates existing data

-- =====================================================
-- Step 1: Ensure linewell.com organization exists
-- =====================================================
INSERT INTO organizations (name, slug) VALUES ('Linewell', 'linewell.com')
ON CONFLICT (slug) DO NOTHING;

-- =====================================================
-- Step 2: Migrate existing admin user to linewell.com
-- =====================================================

-- Update admin's personal_org_id to linewell.com
UPDATE users SET personal_org_id = (SELECT id FROM organizations WHERE slug = 'linewell.com')
WHERE email = 'admin@git-ai.local';

-- Ensure admin is owner of linewell.com org
INSERT INTO org_members (user_id, org_id, role)
SELECT u.id, o.id, 'owner'
FROM users u, organizations o
WHERE u.email = 'admin@git-ai.local'
  AND o.slug = 'linewell.com'
ON CONFLICT (user_id, org_id) DO UPDATE SET role = 'owner';

-- Remove admin from non-linewell.com orgs
DELETE FROM org_members
WHERE user_id = (SELECT id FROM users WHERE email = 'admin@git-ai.local')
  AND org_id != (SELECT id FROM organizations WHERE slug = 'linewell.com');

-- =====================================================
-- Step 3: Create developer1@linewell.com (using non-conflicting UUID)
-- =====================================================
INSERT INTO users (id, email, name, personal_org_id)
VALUES (
    'd0000000-0000-0000-0000-000000000001',
    'developer1@linewell.com',
    'Developer One',
    (SELECT id FROM organizations WHERE slug = 'linewell.com')
) ON CONFLICT (email) DO NOTHING;

-- Create personal org for developer1
INSERT INTO organizations (id, name, slug)
VALUES (
    'd0000000-0000-0000-0000-000000000010',
    'Developer One''s Org',
    'personal-dev1'
) ON CONFLICT (slug) DO NOTHING;

-- Fix personal_org_id
UPDATE users SET personal_org_id = 'd0000000-0000-0000-0000-000000000010'
WHERE email = 'developer1@linewell.com';

-- Add developer1 to personal org
INSERT INTO org_members (user_id, org_id, role)
VALUES ('d0000000-0000-0000-0000-000000000001', 'd0000000-0000-0000-0000-000000000010', 'owner')
ON CONFLICT DO NOTHING;

-- Add developer1 to linewell.com as member
INSERT INTO org_members (user_id, org_id, role)
SELECT 'd0000000-0000-0000-0000-000000000001', id, 'member'
FROM organizations WHERE slug = 'linewell.com'
ON CONFLICT DO NOTHING;

-- =====================================================
-- Step 4: Fix developer2@linewell.com (already exists, just ensure membership)
-- =====================================================
UPDATE users SET personal_org_id = 'b0000000-0000-0000-0000-000000000002'
WHERE email = 'developer2@linewell.com' AND personal_org_id IS NULL;

-- Ensure developer2 in linewell.com
INSERT INTO org_members (user_id, org_id, role)
SELECT u.id, o.id, 'member'
FROM users u, organizations o
WHERE u.email = 'developer2@linewell.com' AND o.slug = 'linewell.com'
ON CONFLICT DO NOTHING;

-- =====================================================
-- Step 5: Ensure teamlead@linewell.com is admin in linewell.com
-- =====================================================
UPDATE org_members SET role = 'admin'
WHERE user_id = (SELECT id FROM users WHERE email = 'teamlead@linewell.com')
  AND org_id = (SELECT id FROM organizations WHERE slug = 'linewell.com');

-- =====================================================
-- Step 6: Ensure xsm@linewell.com is in linewell.com org as member
-- =====================================================
INSERT INTO org_members (user_id, org_id, role)
SELECT u.id, o.id, 'member'
FROM users u, organizations o
WHERE u.email = 'xsm@linewell.com' AND o.slug = 'linewell.com'
ON CONFLICT DO NOTHING;

-- =====================================================
-- Step 7: Create test metrics for developer1
-- =====================================================
INSERT INTO metrics_events (event_type, timestamp, user_id, distinct_id, org_id, repo_url, author_email, tool, model, commit_sha, human_additions, ai_additions, git_diff_added_lines, git_diff_deleted_lines)
VALUES (
    1, EXTRACT(EPOCH FROM NOW())::bigint,
    'd0000000-0000-0000-0000-000000000001',
    'dev1-linewell',
    (SELECT id FROM organizations WHERE slug = 'linewell.com'),
    'https://github.com/linewell/project-alpha.git',
    'developer1@linewell.com',
    'cursor', 'claude-3.5-sonnet',
    'abc123dev1',
    50, 100, 160, 10
);

-- AgentUsage event for developer1
INSERT INTO metrics_events (event_type, timestamp, user_id, distinct_id, org_id, repo_url, author_email, tool, model, ai_additions)
VALUES (
    2, EXTRACT(EPOCH FROM NOW())::bigint - 1800,
    'd0000000-0000-0000-0000-000000000001',
    'dev1-linewell',
    (SELECT id FROM organizations WHERE slug = 'linewell.com'),
    'https://github.com/linewell/project-alpha.git',
    'developer1@linewell.com',
    'cursor', 'claude-3.5-sonnet',
    50
);

-- =====================================================
-- Step 8: Create test project for developer1
-- =====================================================
INSERT INTO projects (remote_url_hash, branch, head_commit, organization, department, org_id, user_id)
VALUES (
    'hash-project-alpha',
    'main',
    'abc123dev1',
    'linewell.com',
    'engineering',
    (SELECT id FROM organizations WHERE slug = 'linewell.com'),
    'd0000000-0000-0000-0000-000000000001'
) ON CONFLICT (remote_url_hash) DO NOTHING;

-- =====================================================
-- Step 9: Ensure departments exist in linewell.com
-- =====================================================
INSERT INTO departments (id, org_id, name, slug)
VALUES (
    'c0000000-0000-0000-0000-000000000001',
    (SELECT id FROM organizations WHERE slug = 'linewell.com'),
    'Engineering',
    'engineering'
) ON CONFLICT (slug) DO NOTHING;

INSERT INTO departments (id, org_id, name, slug)
VALUES (
    'c0000000-0000-0000-0000-000000000002',
    (SELECT id FROM organizations WHERE slug = 'linewell.com'),
    'Management',
    'management'
) ON CONFLICT (slug) DO NOTHING;

-- Assign developers to engineering department
UPDATE org_members SET department_id = 'c0000000-0000-0000-0000-000000000001'
WHERE user_id IN (
    SELECT id FROM users WHERE email IN ('developer1@linewell.com', 'developer2@linewell.com', 'xsm@linewell.com')
)
  AND org_id = (SELECT id FROM organizations WHERE slug = 'linewell.com');

-- Assign teamlead to management department
UPDATE org_members SET department_id = 'c0000000-0000-0000-0000-000000000002'
WHERE user_id = (SELECT id FROM users WHERE email = 'teamlead@linewell.com')
  AND org_id = (SELECT id FROM organizations WHERE slug = 'linewell.com');

-- =====================================================
-- Step 10: Clean up non-linewell.com organizations
-- =====================================================

-- Delete test user and related data
DELETE FROM org_members WHERE user_id = 'b0000000-0000-0000-0000-000000000002';
DELETE FROM users WHERE id = 'b0000000-0000-0000-0000-000000000002';

-- Delete test org
DELETE FROM org_members WHERE org_id = 'b0000000-0000-0000-0000-000000000001';
DELETE FROM departments WHERE org_id = 'b0000000-0000-0000-0000-000000000001';
DELETE FROM organizations WHERE id = 'b0000000-0000-0000-0000-000000000001';

-- Delete default-org (no longer needed)
DELETE FROM org_members WHERE org_id = 'a0000000-0000-0000-0000-000000000001';
DELETE FROM departments WHERE org_id = 'a0000000-0000-0000-0000-000000000001';
DELETE FROM organizations WHERE id = 'a0000000-0000-0000-0000-000000000001';

-- Record migration
INSERT INTO _migrations (id, name) VALUES (5, '005_linewell_org_and_test_data')
ON CONFLICT DO NOTHING;
