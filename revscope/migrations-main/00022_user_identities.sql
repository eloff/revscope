-- Google and Apple sign-ins, keyed by the provider's stable subject id rather
-- than the email, which the person can change with the provider.
CREATE TABLE IF NOT EXISTS user_identities (
    provider TEXT NOT NULL,
    subject TEXT NOT NULL,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (provider, subject)
);

CREATE INDEX IF NOT EXISTS user_identities_user_id_idx
ON user_identities (user_id);
