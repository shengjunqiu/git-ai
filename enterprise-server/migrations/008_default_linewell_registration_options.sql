-- Default developer self-registration options.
-- Admin APIs can manage organizations and departments later; this migration
-- seeds the initial Linewell registration path used by /auth/register.

INSERT INTO organizations (name, slug)
VALUES ('Linewell', 'linewell.com')
ON CONFLICT (slug) DO UPDATE
SET name = EXCLUDED.name;

INSERT INTO organization_domains (org_id, domain, verified)
SELECT id, 'linewell.com', true
FROM organizations
WHERE slug = 'linewell.com'
ON CONFLICT (org_id, domain) DO UPDATE
SET verified = true;

INSERT INTO departments (org_id, name, slug)
SELECT id, '技术中心', 'technology-center'
FROM organizations
WHERE slug = 'linewell.com'
ON CONFLICT (slug) DO UPDATE
SET org_id = EXCLUDED.org_id,
    name = EXCLUDED.name;

INSERT INTO departments (org_id, name, slug)
SELECT id, '研发中心', 'rd-center'
FROM organizations
WHERE slug = 'linewell.com'
ON CONFLICT (slug) DO UPDATE
SET org_id = EXCLUDED.org_id,
    name = EXCLUDED.name;
