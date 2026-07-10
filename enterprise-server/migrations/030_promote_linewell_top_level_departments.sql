-- Treat the source root as the existing Linewell organization, not a department.
-- Promote its direct children to top-level departments and remove stale defaults.

UPDATE departments child
SET parent_id = NULL
FROM departments root
JOIN organizations organization ON organization.id = root.org_id
WHERE organization.slug = 'linewell.com'
  AND root.code = 'F00001'
  AND child.org_id = root.org_id
  AND child.parent_id = root.id;

DELETE FROM departments root
USING organizations organization
WHERE organization.id = root.org_id
  AND organization.slug = 'linewell.com'
  AND root.code = 'F00001';

-- The original flat seed has no exact equivalent in the imported hierarchy.
-- ON DELETE SET NULL preserves existing memberships without assigning them to
-- an unrelated imported department.
DELETE FROM departments legacy
USING organizations organization
WHERE organization.id = legacy.org_id
  AND organization.slug = 'linewell.com'
  AND legacy.slug = 'rd-center';

-- Keep the imported relationship explicit for installations that previously
-- applied the hierarchy migration with partially corrected source data.
UPDATE departments child
SET parent_id = parent.id
FROM departments parent
JOIN organizations organization ON organization.id = parent.org_id
WHERE organization.slug = 'linewell.com'
  AND parent.code = 'F01330'
  AND child.org_id = parent.org_id
  AND child.code = 'F01332';
