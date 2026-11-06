-- Locked-in subscription pricing per account.
--
-- Self-serve signup records the billing period and prices the customer was
-- offered at signup. Later catalog price changes do not rewrite these columns;
-- an account keeps its rate until we explicitly update it.
--
-- Prices are USD cents charged once per billing period. Yearly billing is
-- priced as 10× the monthly list price ("2 months free"). When actual_price
-- is below regular_price for a limited promo (e.g. founder sale),
-- discount_ends_at is when the account reverts to regular_price.

CREATE TYPE billing_period AS ENUM ('monthly', 'yearly');

ALTER TABLE accounts
    ADD COLUMN billing_period billing_period,
    ADD COLUMN regular_price_cents INTEGER,
    ADD COLUMN actual_price_cents INTEGER,
    ADD COLUMN discount_ends_at TIMESTAMPTZ;

-- Seed / complimentary accounts created before this migration.
UPDATE accounts
SET
    billing_period = 'yearly',
    regular_price_cents = 0,
    actual_price_cents = 0
WHERE billing_period IS NULL;

ALTER TABLE accounts
    ALTER COLUMN billing_period SET NOT NULL,
    ALTER COLUMN regular_price_cents SET NOT NULL,
    ALTER COLUMN actual_price_cents SET NOT NULL;

ALTER TABLE accounts
    ADD CONSTRAINT accounts_regular_price_non_negative
        CHECK (regular_price_cents >= 0),
    ADD CONSTRAINT accounts_actual_price_non_negative
        CHECK (actual_price_cents >= 0),
    ADD CONSTRAINT accounts_actual_price_lte_regular
        CHECK (actual_price_cents <= regular_price_cents);
