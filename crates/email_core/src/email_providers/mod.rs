//! Upstream email provider implementations for Gmail, Outlook, and IMAP.

mod gmail;
mod imap;
mod outlook;

pub use gmail::GmailProvider;
pub use imap::ImapProvider;
pub use outlook::OutlookProvider;