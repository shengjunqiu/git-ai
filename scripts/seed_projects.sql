-- =====================================================
-- 向 projects + commit_stats 表插入模拟数据
-- 以匹配 metrics_events 中已有的 5 个项目
-- =====================================================

-- 1. 插入 projects
INSERT INTO projects (id, remote_url_hash, branch, head_commit, organization, department, updated_at) VALUES
(1,  'api-gateway',       'main', 'aaa1005', 'Engineering', 'Backend',  NOW()),
(2,  'web-frontend',      'main', 'bbb2005', 'Engineering', 'Frontend', NOW()),
(3,  'data-pipeline',     'main', 'hhh8003', 'Engineering', 'Data',     NOW()),
(4,  'mobile-app',        'main', 'ddd4003', 'Engineering', 'Mobile',   NOW()),
(5,  'devops-infra',      'main', 'eee5004', 'Engineering', 'DevOps',   NOW())
ON CONFLICT DO NOTHING;

-- 2. 插入 commit_stats (对应 metrics_events 中的 committed 事件)

-- ===== api-gateway (project_id=1) =====
-- 张伟
INSERT INTO commit_stats (project_id, sha, author, author_time, subject, has_authorship_note, git_diff_added_lines, git_diff_deleted_lines, ai_additions, human_additions, mixed_additions, unknown_additions, ai_accepted, total_ai_additions, total_ai_deletions, time_waiting_for_ai) VALUES
(1, 'aaa1001', 'zhang.wei@company.com', '2026-05-02T09:15:00Z', 'feat: add JWT auth middleware', true, 140, 10, 120, 15, 5, 0, 100, 120, 0, 30),
(1, 'aaa1002', 'zhang.wei@company.com', '2026-05-05T14:30:00Z', 'feat: add rate limiting', true, 96, 5, 85, 8, 3, 0, 75, 85, 0, 25),
(1, 'aaa1003', 'zhang.wei@company.com', '2026-05-07T10:45:00Z', 'feat: implement CORS middleware', true, 230, 15, 200, 20, 10, 0, 170, 200, 0, 45),
(1, 'aaa1004', 'zhang.wei@company.com', '2026-05-11T16:20:00Z', 'refactor: extract config module', true, 188, 20, 150, 30, 8, 0, 130, 150, 5, 35),
-- 孙磊
(1, 'ggg7001', 'sun.lei@enterprise.cn', '2026-05-04T15:00:00Z', 'feat: health check endpoint', true, 100, 8, 70, 25, 5, 0, 60, 70, 0, 20),
(1, 'ggg7002', 'sun.lei@enterprise.cn', '2026-05-10T11:00:00Z', 'feat: metrics endpoint', true, 98, 12, 55, 35, 8, 0, 45, 55, 0, 18),
-- 陈杰
(1, 'eee5004', 'chen.jie@startup.io', '2026-05-14T09:00:00Z', 'feat: proxy handler', true, 71, 2, 60, 8, 3, 0, 50, 60, 0, 15);

-- ===== web-frontend (project_id=2) =====
-- 李娜
INSERT INTO commit_stats (project_id, sha, author, author_time, subject, has_authorship_note, git_diff_added_lines, git_diff_deleted_lines, ai_additions, human_additions, mixed_additions, unknown_additions, ai_accepted, total_ai_additions, total_ai_deletions, time_waiting_for_ai) VALUES
(2, 'bbb2001', 'li.na@company.com', '2026-05-02T11:00:00Z', 'feat: add dashboard charts', true, 120, 8, 60, 45, 10, 5, 50, 60, 0, 22),
(2, 'bbb2002', 'li.na@company.com', '2026-05-04T09:30:00Z', 'feat: add auth hook', true, 75, 3, 40, 30, 5, 0, 35, 40, 0, 15),
(2, 'bbb2003', 'li.na@company.com', '2026-05-07T15:45:00Z', 'feat: settings page', true, 95, 12, 30, 55, 8, 2, 25, 30, 2, 12),
(2, 'bbb2004', 'li.na@company.com', '2026-05-11T11:00:00Z', 'refactor: API utilities', true, 90, 15, 25, 60, 5, 0, 20, 25, 3, 10),
-- 张伟
(2, 'aaa1005', 'zhang.wei@company.com', '2026-05-13T10:00:00Z', 'feat: API client', true, 88, 5, 75, 10, 3, 0, 65, 75, 0, 20),
-- 赵敏
(2, 'fff6001', 'zhao.min@startup.io', '2026-05-03T14:00:00Z', 'feat: user profile component', true, 123, 10, 35, 80, 5, 3, 28, 35, 0, 14),
(2, 'fff6002', 'zhao.min@startup.io', '2026-05-07T09:00:00Z', 'feat: chart component', true, 120, 15, 40, 70, 8, 2, 32, 40, 1, 16),
(2, 'fff6003', 'zhao.min@startup.io', '2026-05-11T16:30:00Z', 'feat: data table component', true, 130, 20, 50, 65, 10, 5, 40, 50, 2, 18);

-- ===== data-pipeline (project_id=3) =====
-- 王芳
INSERT INTO commit_stats (project_id, sha, author, author_time, subject, has_authorship_note, git_diff_added_lines, git_diff_deleted_lines, ai_additions, human_additions, mixed_additions, unknown_additions, ai_accepted, total_ai_additions, total_ai_deletions, time_waiting_for_ai) VALUES
(3, 'ccc3001', 'wang.fang@company.com', '2026-05-03T08:00:00Z', 'feat: ETL transform pipeline', true, 140, 5, 15, 120, 3, 2, 10, 15, 0, 8),
(3, 'ccc3002', 'wang.fang@company.com', '2026-05-06T13:20:00Z', 'feat: schema validators', true, 107, 8, 10, 95, 2, 0, 8, 10, 0, 5),
(3, 'ccc3003', 'wang.fang@company.com', '2026-05-10T09:15:00Z', 'feat: data cleaning transforms', false, 150, 12, 0, 150, 0, 0, 0, 0, 0, 0),
-- 周涛
(3, 'hhh8001', 'zhou.tao@enterprise.cn', '2026-05-02T08:30:00Z', 'feat: pipeline core engine', false, 180, 5, 0, 180, 0, 0, 0, 0, 0, 0),
(3, 'hhh8002', 'zhou.tao@enterprise.cn', '2026-05-06T10:00:00Z', 'feat: pipeline scheduler', true, 165, 8, 5, 160, 0, 0, 3, 5, 0, 4),
(3, 'hhh8003', 'zhou.tao@enterprise.cn', '2026-05-11T09:00:00Z', 'feat: data loader', true, 123, 10, 20, 100, 3, 0, 15, 20, 0, 8),
-- 李娜
(3, 'bbb2005', 'li.na@company.com', '2026-05-13T14:30:00Z', 'feat: chart visualization', true, 100, 8, 55, 40, 5, 0, 45, 55, 1, 12);

-- ===== mobile-app (project_id=4) =====
-- 刘洋
INSERT INTO commit_stats (project_id, sha, author, author_time, subject, has_authorship_note, git_diff_added_lines, git_diff_deleted_lines, ai_additions, human_additions, mixed_additions, unknown_additions, ai_accepted, total_ai_additions, total_ai_deletions, time_waiting_for_ai) VALUES
(4, 'ddd4001', 'liu.yang@company.com', '2026-05-04T10:00:00Z', 'feat: home screen UI', true, 105, 3, 90, 10, 5, 0, 80, 90, 0, 25),
(4, 'ddd4002', 'liu.yang@company.com', '2026-05-07T11:30:00Z', 'feat: navigation setup', true, 123, 2, 110, 5, 8, 0, 95, 110, 0, 30),
(4, 'ddd4003', 'liu.yang@company.com', '2026-05-11T14:00:00Z', 'feat: card component', true, 75, 8, 45, 25, 5, 0, 38, 45, 0, 12);

-- ===== devops-infra (project_id=5) =====
-- 陈杰
INSERT INTO commit_stats (project_id, sha, author, author_time, subject, has_authorship_note, git_diff_added_lines, git_diff_deleted_lines, ai_additions, human_additions, mixed_additions, unknown_additions, ai_accepted, total_ai_additions, total_ai_deletions, time_waiting_for_ai) VALUES
(5, 'eee5001', 'chen.jie@startup.io', '2026-05-01T16:00:00Z', 'feat: EKS module', true, 200, 5, 180, 12, 8, 0, 160, 180, 0, 40),
(5, 'eee5002', 'chen.jie@startup.io', '2026-05-06T10:30:00Z', 'feat: RDS module', true, 154, 8, 130, 18, 6, 0, 110, 130, 0, 35),
(5, 'eee5003', 'chen.jie@startup.io', '2026-05-10T08:00:00Z', 'feat: API deployment manifests', true, 103, 2, 95, 5, 3, 0, 85, 95, 0, 22);
