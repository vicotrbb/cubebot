-- More cube paints, eyes and accessories, and an optional custom `#rrggbb` color that replaces
-- the paint (apps/orchestrator/src/agents/definitions.rs).
ALTER TABLE agents DROP CONSTRAINT agents_avatar_check;
ALTER TABLE agents ADD CONSTRAINT agents_avatar_check CHECK (
    avatar->>'paint' IN (
        'orange', 'sage', 'paper', 'wood', 'sky', 'rose', 'lemon', 'plum',
        'coral', 'mint', 'ocean', 'charcoal'
    )
    AND avatar->>'eyes' IN ('round', 'wide', 'sleepy', 'dot', 'happy', 'wink', 'star', 'heart')
    AND avatar->>'accessory' IN (
        'none', 'cap', 'glasses', 'bow', 'antenna',
        'crown', 'headphones', 'sprout', 'party_hat', 'monocle'
    )
    AND (NOT avatar ? 'color' OR avatar->>'color' ~ '^#[0-9a-f]{6}$')
);
