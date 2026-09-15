//! View routing declarations and subcomponent exports.

pub mod add_account;
pub mod component;
pub mod dashboard;
pub mod findings;
pub mod inbox;
pub mod layout;
pub mod settings;
pub mod trash;

pub use add_account::AddAccount;
pub use dashboard::Dashboard;
pub use findings::Findings;
pub use inbox::Inbox;
pub use layout::Layout;
pub use settings::Settings;
pub use trash::Trash;