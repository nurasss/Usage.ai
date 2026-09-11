-- Per-account notification mute (V13-06): users silence one profile's
-- alerts without disabling its refresh. Default off (0 = notify).
ALTER TABLE accounts ADD COLUMN notifications_muted INTEGER NOT NULL DEFAULT 0;
