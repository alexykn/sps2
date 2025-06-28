-- Test data and validation queries for file-level schema

-- Test 1: Basic file object insertion
INSERT INTO file_objects (hash, size, created_at, ref_count, is_executable, is_symlink, symlink_target)
VALUES 
    ('a1b2c3d4e5f6789012345678901234567890123456789012345678901234567890', 1024, strftime('%s', 'now'), 1, 0, 0, NULL),
    ('b2c3d4e5f67890123456789012345678901234567890123456789012345678901', 2048, strftime('%s', 'now'), 2, 1, 0, NULL),
    ('c3d4e5f678901234567890123456789012345678901234567890123456789012', 512, strftime('%s', 'now'), 1, 0, 1, '/usr/bin/python3');

-- Test 2: Package file entries with deduplication
-- Assuming we have packages with IDs 1 and 2
INSERT INTO package_file_entries (package_id, file_hash, relative_path, permissions, uid, gid, mtime)
VALUES
    -- Package 1 files
    (1, 'a1b2c3d4e5f6789012345678901234567890123456789012345678901234567890', 'bin/tool1', 755, 0, 0, strftime('%s', 'now')),
    (1, 'b2c3d4e5f67890123456789012345678901234567890123456789012345678901', 'lib/shared.so', 644, 0, 0, strftime('%s', 'now')),
    -- Package 2 files (note: reuses file_hash b2c3... for deduplication)
    (2, 'b2c3d4e5f67890123456789012345678901234567890123456789012345678901', 'lib/shared.so', 644, 0, 0, strftime('%s', 'now')),
    (2, 'c3d4e5f678901234567890123456789012345678901234567890123456789012', 'bin/python', 755, 0, 0, strftime('%s', 'now'));

-- Test 3: Installed files tracking
INSERT INTO installed_files (state_id, package_id, file_hash, installed_path, is_directory)
VALUES
    ('state-uuid-1', 1, 'a1b2c3d4e5f6789012345678901234567890123456789012345678901234567890', '/opt/pm/live/bin/tool1', 0),
    ('state-uuid-1', 1, 'b2c3d4e5f67890123456789012345678901234567890123456789012345678901', '/opt/pm/live/lib/shared.so', 0),
    ('state-uuid-1', 2, 'c3d4e5f678901234567890123456789012345678901234567890123456789012', '/opt/pm/live/bin/python', 0);

-- Validation Query 1: Check deduplication effectiveness
SELECT 
    'Deduplication Test' as test_name,
    COUNT(DISTINCT file_hash) as unique_files,
    COUNT(*) as total_references,
    CAST(COUNT(*) AS REAL) / COUNT(DISTINCT file_hash) as dedup_ratio
FROM package_file_entries;

-- Validation Query 2: Verify referential integrity
SELECT 
    'Referential Integrity' as test_name,
    (SELECT COUNT(*) FROM package_file_entries pfe
     LEFT JOIN file_objects fo ON pfe.file_hash = fo.hash
     WHERE fo.hash IS NULL) as orphaned_entries,
    (SELECT COUNT(*) FROM installed_files if
     LEFT JOIN file_objects fo ON if.file_hash = fo.hash
     WHERE fo.hash IS NULL) as orphaned_installations;

-- Validation Query 3: Check unique constraints
SELECT 
    'Unique Constraints' as test_name,
    (SELECT COUNT(*) FROM (
        SELECT package_id, relative_path, COUNT(*) as cnt
        FROM package_file_entries
        GROUP BY package_id, relative_path
        HAVING cnt > 1
    )) as duplicate_package_files,
    (SELECT COUNT(*) FROM (
        SELECT state_id, installed_path, COUNT(*) as cnt
        FROM installed_files
        GROUP BY state_id, installed_path
        HAVING cnt > 1
    )) as duplicate_installations;

-- Validation Query 4: Performance check - file lookup
EXPLAIN QUERY PLAN
SELECT pfe.*, fo.*
FROM package_file_entries pfe
JOIN file_objects fo ON pfe.file_hash = fo.hash
WHERE pfe.package_id = 1 AND pfe.relative_path = 'bin/tool1';

-- Validation Query 5: Performance check - installation verification
EXPLAIN QUERY PLAN
SELECT if.*, fo.*
FROM installed_files if
JOIN file_objects fo ON if.file_hash = fo.hash
WHERE if.installed_path = '/opt/pm/live/bin/tool1';

-- Validation Query 6: Garbage collection candidates
SELECT 
    'GC Candidates' as test_name,
    COUNT(*) as unreferenced_files
FROM file_objects
WHERE ref_count = 0;

-- Validation Query 7: File verification cache effectiveness
INSERT INTO file_verification_cache (file_hash, installed_path, verified_at, is_valid, error_message)
VALUES
    ('a1b2c3d4e5f6789012345678901234567890123456789012345678901234567890', '/opt/pm/live/bin/tool1', strftime('%s', 'now'), 1, NULL),
    ('b2c3d4e5f67890123456789012345678901234567890123456789012345678901', '/opt/pm/live/lib/shared.so', strftime('%s', 'now') - 3600, 0, 'Hash mismatch');

SELECT 
    'Verification Cache' as test_name,
    COUNT(*) as total_cached,
    SUM(CASE WHEN is_valid = 1 THEN 1 ELSE 0 END) as valid_files,
    SUM(CASE WHEN is_valid = 0 THEN 1 ELSE 0 END) as invalid_files,
    AVG(strftime('%s', 'now') - verified_at) as avg_cache_age_seconds
FROM file_verification_cache;

-- Cleanup test data
DELETE FROM file_verification_cache WHERE file_hash IN (
    'a1b2c3d4e5f6789012345678901234567890123456789012345678901234567890',
    'b2c3d4e5f67890123456789012345678901234567890123456789012345678901',
    'c3d4e5f678901234567890123456789012345678901234567890123456789012'
);
DELETE FROM installed_files WHERE state_id = 'state-uuid-1';
DELETE FROM package_file_entries WHERE package_id IN (1, 2);
DELETE FROM file_objects WHERE hash IN (
    'a1b2c3d4e5f6789012345678901234567890123456789012345678901234567890',
    'b2c3d4e5f67890123456789012345678901234567890123456789012345678901',
    'c3d4e5f678901234567890123456789012345678901234567890123456789012'
);