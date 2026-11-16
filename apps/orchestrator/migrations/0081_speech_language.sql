ALTER TABLE speech_settings
    ADD COLUMN language text NOT NULL DEFAULT 'english'
        CHECK (language IN ('english', 'portuguese'));
