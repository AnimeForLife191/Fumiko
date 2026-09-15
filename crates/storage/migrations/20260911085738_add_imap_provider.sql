-- no-transaction
PRAGMA foreign_keys = OFF;

CREATE TABLE linked_accounts_new (
    id              TEXT PRIMARY KEY,
    provider        TEXT NOT NULL CHECK (provider IN ('gmail', 'outlook', 'imap')),
    email_address   TEXT NOT NULL UNIQUE,
    display_name    TEXT,

    sync_cursor     TEXT,
    last_synced_at  INTEGER,
    last_sync_error TEXT,
    created_at      INTEGER NOT NULL,

    imap_host       TEXT,
    imap_port       INTEGER
);

INSERT INTO linked_accounts_new (
    id,
    provider,
    email_address,
    display_name,
    sync_cursor,
    last_synced_at,
    last_sync_error,
    created_at,
    imap_host,
    imap_port
)
SELECT
    id,
    provider,
    email_address,
    display_name,
    sync_cursor,
    last_synced_at,
    last_sync_error,
    created_at,
    NULL,
    NULL
FROM linked_accounts;

DROP TABLE linked_accounts;
ALTER TABLE linked_accounts_new RENAME TO linked_accounts;

PRAGMA foreign_keys = ON;
PRAGMA foreign_key_check;