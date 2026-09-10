-- Accounts metadata: Refresh tokens are stored in the OS keyring, not SQLite.
CREATE TABLE linked_accounts (
    id              TEXT PRIMARY KEY,
    provider        TEXT NOT NULL CHECK (provider IN ('gmail', 'outlook')),
    email_address   TEXT NOT NULL UNIQUE,
    display_name    TEXT,

    -- Opaque sync token: Gmail historyId string or Outlook deltaLink URL.
    sync_cursor     TEXT,
    last_synced_at  INTEGER,
    last_sync_error TEXT,
    created_at      INTEGER NOT NULL
);

-- User-defined rules for automated classification.
CREATE TABLE watch_criteria (
    id           TEXT PRIMARY KEY,
    label        TEXT NOT NULL,
    description  TEXT NOT NULL,
    is_active    INTEGER NOT NULL DEFAULT 1,
    created_at   INTEGER NOT NULL
);

-- Metadata for synchronized messages (bodies and non-inline attachments are fetched on demand).
CREATE TABLE emails (
    id                  TEXT PRIMARY KEY,
    account_id          TEXT NOT NULL
        REFERENCES linked_accounts(id)
        ON DELETE CASCADE,
    provider_message_id TEXT NOT NULL,
    subject             TEXT,
    sender              TEXT,
    received_at         INTEGER NOT NULL,
    is_read             INTEGER NOT NULL DEFAULT 0,
    app_has_viewed      INTEGER NOT NULL DEFAULT 0,
    is_trashed          INTEGER NOT NULL DEFAULT 0,
    trashed_at          INTEGER,
    snippet             TEXT,
    created_at          INTEGER NOT NULL,

    UNIQUE(account_id, provider_message_id)
);

-- Accelerates inbox listing filtered by active status and sorted by date.
CREATE INDEX idx_emails_inbox ON emails(account_id, is_trashed, received_at DESC);

-- Accelerates global inbox view across all accounts.
CREATE INDEX idx_emails_inbox_all ON emails(is_trashed, received_at DESC);

-- Accelerates trash views and background retention pruning.
CREATE INDEX idx_emails_trash ON emails(is_trashed, trashed_at DESC);

-- Connects emails to matching criteria.
CREATE TABLE classifications (
    id           TEXT PRIMARY KEY,
    email_id     TEXT NOT NULL
        REFERENCES emails(id)
        ON DELETE CASCADE,
    criterion_id TEXT NOT NULL
        REFERENCES watch_criteria(id)
        ON DELETE CASCADE,
    confidence   REAL,
    created_at   INTEGER NOT NULL,

    UNIQUE(email_id, criterion_id)
);

CREATE INDEX idx_classifications_email ON classifications(email_id);
CREATE INDEX idx_classifications_criterion ON classifications(criterion_id);

-- Key-value settings store.
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);