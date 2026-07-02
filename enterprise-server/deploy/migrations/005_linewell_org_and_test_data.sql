-- Migration 005: Set up linewell.com as the primary organization
-- 1. Create linewell.com organization if not exists
-- 2. Migrate admin user to linewell.com
-- 3. Remove non-linewell.com organizations and their data
-- 4. Create test users and data for data isolation testing

-- =====================================================
-- Step 1: Create linewell.com organization
-- =====================================================
INSERT INTO organizations (name, slug) VALUES ('Linewell', 'linewell.com')
ON CONFLICT (slug) DO NOTHING;

-- =====================================================
-- Step 2: Migrate existing admin user to linewell.com
-- =====================================================

-- Update admin user's personal_org_id to linewell.com org
UPDATE users SET personal_org_id = (SELECT id FROM organizations WHERE slug = 'linewell.com')
WHERE email = 'admin@git-ai.local' AND personal_org_id IS NULL;

-- If old admin user exists, also update their personal org
UPDATE users SET personal_org_id = (SELECT id FROM organizations WHERE slug = 'linewell.com')
WHERE email = 'admin@linewell.com';

-- Add admin to linewell.com org as owner
INSERT INTO org_members (user_id, org_id, role)
SELECT u.id, o.id, 'owner'
FROM users u, organizations o
WHERE u.email IN ('admin@git-ai.local', 'admin@linewell.com')
  AND o.slug = 'linewell.com'
ON CONFLICT (user_id, org_id) DO UPDATE SET role = 'owner';

-- =====================================================
-- Step 3: Delete data from non-linewell.com organizations
-- =====================================================

-- Delete metrics_events belonging to other orgs
DELETE FROM metrics_events
WHERE org_id IS NOT NULL
  AND org_id != (SELECT id FROM organizations WHERE slug = 'linewell.com');

-- Delete projects belonging to other orgs
DELETE FROM projects
WHERE org_id IS NOT NULL
  AND org_id != (SELECT id FROM organizations WHERE slug = 'linewell.com');

-- Delete CAS objects belonging to other orgs
DELETE FROM cas_objects
WHERE org_id IS NOT NULL
  AND org_id != (SELECT id FROM organizations WHERE slug = 'linewell.com');

-- Delete CAS ownership belonging to other orgs
DELETE FROM cas_ownership
WHERE org_id IS NOT NULL
  AND org_id != (SELECT id FROM organizations WHERE slug = 'linewell.com');

-- Delete audit log for other orgs
DELETE FROM audit_log
WHERE org_id IS NOT NULL
  AND org_id != (SELECT id FROM organizations WHERE slug = 'linewell.com');

-- Delete pull requests belonging to other orgs
DELETE FROM pull_requests
WHERE org_id IS NOT NULL
  AND org_id != (SELECT id FROM organizations WHERE slug = 'linewell.com');

-- Delete agent readiness scores for other orgs
DELETE FROM agent_readiness_scores
WHERE org_id IS NOT NULL
  AND org_id != (SELECT id FROM organizations WHERE slug = 'linewell.com');

-- Delete persistence snapshots for other orgs
DELETE FROM ai_code_persistence_snapshots
WHERE org_id IS NOT NULL
  AND org_id != (SELECT id FROM organizations WHERE slug = 'linewell.com');

-- Remove org_members from other orgs (except personal orgs)
DELETE FROM org_members
WHERE org_id NOT IN (
    SELECT id FROM organizations WHERE slug = 'linewell.com'
)
AND org_id NOT IN (
    SELECT personal_org_id FROM users WHERE personal_org_id IS NOT NULL
);

-- Delete departments from other orgs
DELETE FROM departments
WHERE org_id NOT IN (
    SELECT id FROM organizations WHERE slug = 'linewell.com'
)
AND org_id NOT IN (
    SELECT personal_org_id FROM users WHERE personal_org_id IS NOT NULL
);

-- Delete non-linewell.com non-personal organizations
DELETE FROM organizations
WHERE slug != 'linewell.com'
  AND id NOT IN (SELECT personal_org_id FROM users WHERE personal_org_id IS NOT NULL);

-- =====================================================
-- Step 4: Create test users and test data
-- =====================================================

-- Test user 1: developer1@linewell.com (member)
INSERT INTO users (id, email, name, personal_org_id)
VALUES (
    'a0000000-0000-0000-0000-000000000001',
    'developer1@linewell.com',
    'Developer One',
    (SELECT id FROM organizations WHERE slug = 'linewell.com')
) ON CONFLICT (email) DO NOTHING;

-- Create personal org for developer1 if not already done
INSERT INTO organizations (id, name, slug)
VALUES (
    'b0000000-0000-0000-0000-000000000001',
    'Developer One''s Org',
    'personal-dev1'
) ON CONFLICT (id) DO NOTHING;

-- Fix personal_org_id to use dedicated personal org
UPDATE users SET personal_org_id = 'b0000000-0000-0000-0000-000000000001'
WHERE email = 'developer1@linewell.com';

-- Add developer1 to personal org as owner
INSERT INTO org_members (user_id, org_id, role)
VALUES ('a0000000-0000-0000-0000-000000000001', 'b0000000-0000-0000-0000-000000000001', 'owner')
ON CONFLICT DO NOTHING;

-- Add developer1 to linewell.com as member
INSERT INTO org_members (user_id, org_id, role)
SELECT 'a0000000-0000-0000-0000-000000000001', id, 'member'
FROM organizations WHERE slug = 'linewell.com'
ON CONFLICT DO NOTHING;

-- Test user 2: developer2@linewell.com (member)
INSERT INTO users (id, email, name, personal_org_id)
VALUES (
    'a0000000-0000-0000-0000-000000000002',
    'developer2@linewell.com',
    'Developer Two',
    'b0000000-0000-0000-0000-000000000002'
) ON CONFLICT (email) DO NOTHING;

-- Create personal org for developer2
INSERT INTO organizations (id, name, slug)
VALUES (
    'b0000000-0000-0000-0000-000000000002',
    'Developer Two''s Org',
    'personal-dev2'
) ON CONFLICT (id) DO NOTHING;

UPDATE users SET personal_org_id = 'b0000000-0000-0000-0000-000000000002'
WHERE email = 'developer2@linewell.com';

-- Add developer2 to personal org as owner
INSERT INTO org_members (user_id, org_id, role)
VALUES ('a0000000-0000-0000-0000-000000000002', 'b0000000-0000-0000-0000-000000000002', 'owner')
ON CONFLICT DO NOTHING;

-- Add developer2 to linewell.com as member
INSERT INTO org_members (user_id, org_id, role)
SELECT 'a0000000-0000-0000-0000-000000000002', id, 'member'
FROM organizations WHERE slug = 'linewell.com'
ON CONFLICT DO NOTHING;

-- Test user 3: teamlead@linewell.com (admin role - can see all org data)
INSERT INTO users (id, email, name, personal_org_id)
VALUES (
    'a0000000-0000-0000-0000-000000000003',
    'teamlead@linewell.com',
    'Team Lead',
    'b0000000-0000-0000-0000-000000000003'
) ON CONFLICT (email) DO NOTHING;

-- Create personal org for teamlead
INSERT INTO organizations (id, name, slug)
VALUES (
    'b0000000-0000-0000-0000-000000000003',
    'Team Lead''s Org',
    'personal-lead'
) ON CONFLICT (id) DO NOTHING;

UPDATE users SET personal_org_id = 'b0000000-0000-0000-0000-000000000003'
WHERE email = 'teamlead@linewell.com';

-- Add teamlead to personal org as owner
INSERT INTO org_members (user_id, org_id, role)
VALUES ('a0000000-0000-0000-0000-000000000003', 'b0000000-0000-0000-0000-000000000003', 'owner')
ON CONFLICT DO NOTHING;

-- Add teamlead to linewell.com as admin
INSERT INTO org_members (user_id, org_id, role)
SELECT 'a0000000-0000-0000-0000-000000000003', id, 'admin'
FROM organizations WHERE slug = 'linewell.com'
ON CONFLICT DO NOTHING;

-- =====================================================
-- Step 5: Create test metrics data for data isolation testing
-- =====================================================

-- Metrics for developer1 (in linewell.com org)
INSERT INTO metrics_events (event_type, timestamp, user_id, distinct_id, org_id, repo_url, author_email, tool, model, commit_sha, human_additions, ai_additions, git_diff_added_lines, git_diff_deleted_lines)
VALUES (
    1, EXTRACT(EPOCH FROM NOW())::bigint,
    'a0000000-0000-0000-0000-000000000001',
    'dev1-linewell',
    (SELECT id FROM organizations WHERE slug = 'linewell.com'),
    'https://github.com/linewell/project-alpha.git',
    'developer1@linewell.com',
    'cursor', 'claude-3.5-sonnet',
    'abc123dev1',
    50, 100, 160, 10
);

-- Metrics for developer2 (in linewell.com org - same org, different user)
INSERT INTO metrics_events (event_type, timestamp, user_id, distinct_id, org_id, repo_url, author_email, tool, model, commit_sha, human_additions, ai_additions, git_diff_added_lines, git_diff_deleted_lines)
VALUES (
    1, EXTRACT(EPOCH FROM NOW())::bigint - 3600,
    'a0000000-0000-0000-0000-000000000002',
    'dev2-linewell',
    (SELECT id FROM organizations WHERE slug = 'linewell.com'),
    'https://github.com/linewell/project-beta.git',
    'developer2@linewell.com',
    'copilot', 'gpt-4o',
    'def456dev2',
    80, 200, 290, 20
);

-- Metrics for teamlead (admin, in linewell.com org)
INSERT INTO metrics_events (event_type, timestamp, user_id, distinct_id, org_id, repo_url, author_email, tool, model, commit_sha, human_additions, ai_additions, git_diff_added_lines, git_diff_deleted_lines)
VALUES (
    1, EXTRACT(EPOCH FROM NOW())::bigint - 7200,
    'a0000000-0000-0000-0000-000000000003',
    'lead-linewell',
    (SELECT id FROM organizations WHERE slug = 'linewell.com'),
    'https://github.com/linewell/project-gamma.git',
    'teamlead@linewell.com',
    'cursor', 'claude-3.5-sonnet',
    'ghi789lead',
    30, 150, 190, 5
);

-- AgentUsage events for developer1
INSERT INTO metrics_events (event_type, timestamp, user_id, distinct_id, org_id, repo_url, author_email, tool, model, ai_additions)
VALUES (
    2, EXTRACT(EPOCH FROM NOW())::bigint - 1800,
    'a0000000-0000-0000-0000-000000000001',
    'dev1-linewell',
    (SELECT id FROM organizations WHERE slug = 'linewell.com'),
    'https://github.com/linewell/project-alpha.git',
    'developer1@linewell.com',
    'cursor', 'claude-3.5-sonnet',
    50
);

-- AgentUsage events for developer2
INSERT INTO metrics_events (event_type, timestamp, user_id, distinct_id, org_id, repo_url, author_email, tool, model, ai_additions)
VALUES (
    2, EXTRACT(EPOCH FROM NOW())::bigint - 5400,
    'a0000000-0000-0000-0000-000000000002',
    'dev2-linewell',
    (SELECT id FROM organizations WHERE slug = 'linewell.com'),
    'https://github.com/linewell/project-beta.git',
    'developer2@linewell.com',
    'copilot', 'gpt-4o',
    80
);

-- =====================================================
-- Step 6: Create test projects with org_id and user_id
-- =====================================================

-- Project for developer1
INSERT INTO projects (remote_url_hash, branch, head_commit, organization, department, org_id, user_id)
VALUES (
    'hash-project-alpha',
    'main',
    'abc123dev1',
    'linewell.com',
    'engineering',
    (SELECT id FROM organizations WHERE slug = 'linewell.com'),
    'a0000000-0000-0000-0000-000000000001'
) ON CONFLICT (remote_url_hash) DO NOTHING;

-- Project for developer2
INSERT INTO projects (remote_url_hash, branch, head_commit, organization, department, org_id, user_id)
VALUES (
    'hash-project-beta',
    'develop',
    'def456dev2',
    'linewell.com',
    'engineering',
    (SELECT id FROM organizations WHERE slug = 'linewell.com'),
    'a0000000-0000-0000-0000-000000000002'
) ON CONFLICT (remote_url_hash) DO NOTHING;

-- Project for teamlead
INSERT INTO projects (remote_url_hash, branch, head_commit, organization, department, org_id, user_id)
VALUES (
    'hash-project-gamma',
    'main',
    'ghi789lead',
    'linewell.com',
    'management',
    (SELECT id FROM organizations WHERE slug = 'linewell.com'),
    'a0000000-0000-0000-0000-000000000003'
) ON CONFLICT (remote_url_hash) DO NOTHING;

-- =====================================================
-- Step 7: Create departments in linewell.com
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
WHERE user_id IN ('a0000000-0000-0000-0000-000000000001', 'a0000000-0000-0000-0000-000000000002')
  AND org_id = (SELECT id FROM organizations WHERE slug = 'linewell.com');

-- Assign teamlead to management department
UPDATE org_members SET department_id = 'c0000000-0000-0000-0000-000000000002'
WHERE user_id = 'a0000000-0000-0000-0000-000000000003'
  AND org_id = (SELECT id FROM organizations WHERE slug = 'linewell.com');
