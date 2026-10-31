-- Phase 7: vision review. Frames live on disk (vision/<live_game_id>/) and
-- are deleted after review; the database keeps only capture stats and state.

-- rc_vision::CaptureStats as JSON (frames, bytes, timing), written at game end.
ALTER TABLE live_games ADD COLUMN vision_json TEXT;
-- NULL (not captured) | captured | reviewed | failed | deleted
ALTER TABLE live_games ADD COLUMN vision_state TEXT;
-- The vision review report (ai_reports.kind = 'vision'), once written.
ALTER TABLE live_games ADD COLUMN vision_report_id INTEGER;
