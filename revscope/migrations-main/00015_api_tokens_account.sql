-- An MCP token is minted for an account, not a project. The agent names the
-- project on each query, so binding the token to one at consent time would make
-- switching projects mean another trip through the browser, and a project added
-- later would be unreachable without one.
--
-- So project_id becomes the optional half of the scope and account_id carries
-- it: a row with both grants exactly that project (query tokens), and a row with
-- only an account grants every project on it, now and later (MCP tokens). Every
-- writer sets account_id.
ALTER TABLE api_tokens
ALTER COLUMN project_id DROP NOT NULL;

ALTER TABLE api_tokens
ADD COLUMN IF NOT EXISTS account_id UUID REFERENCES accounts (id) ON DELETE CASCADE;

-- Every token predating this is a project token, so its account is the
-- project's.
UPDATE api_tokens AS t
SET account_id = p.account_id
FROM projects AS p
WHERE t.project_id = p.id AND t.account_id IS NULL;
