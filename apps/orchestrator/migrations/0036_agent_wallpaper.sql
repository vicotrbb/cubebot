-- A person's desktop wallpaper for one agent; no image means the guest's default. The revision
-- changes on every set or reset, so a connected agent hears about it and reapplies.
ALTER TABLE agents
    ADD COLUMN wallpaper bytea CHECK (octet_length(wallpaper) <= 4194304),
    ADD COLUMN wallpaper_revision bigint NOT NULL DEFAULT 0;
