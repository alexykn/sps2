-- Migration 0007: Add store verification tracking
-- This migration adds columns to track verification status for content-addressed store objects

-- Add verification tracking columns to file_objects table
ALTER TABLE file_objects ADD COLUMN last_verified_at INTEGER;
ALTER TABLE file_objects ADD COLUMN verification_status TEXT NOT NULL DEFAULT 'pending' CHECK (
    verification_status IN ('pending', 'verified', 'failed', 'quarantined')
);
ALTER TABLE file_objects ADD COLUMN verification_error TEXT;
ALTER TABLE file_objects ADD COLUMN verification_attempts INTEGER NOT NULL DEFAULT 0;

-- Indexes for efficient verification queries
CREATE INDEX idx_file_objects_verification_status ON file_objects(verification_status);
CREATE INDEX idx_file_objects_last_verified_at ON file_objects(last_verified_at);
CREATE INDEX idx_file_objects_verification_pending ON file_objects(verification_status, last_verified_at) 
    WHERE verification_status = 'pending';

-- Index for finding objects that need re-verification (older than threshold)
CREATE INDEX idx_file_objects_needs_reverification ON file_objects(last_verified_at, verification_status)
    WHERE verification_status = 'verified';

-- Index for failed verification tracking
CREATE INDEX idx_file_objects_verification_failed ON file_objects(verification_status, verification_attempts)
    WHERE verification_status = 'failed';

-- Update schema version
UPDATE schema_version SET version = 7, applied_at = strftime('%s', 'now');