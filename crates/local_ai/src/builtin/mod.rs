//! Built-in inference engine powered by a bundled `llama-server` sidecar.

pub mod catalog;
mod classifier;
pub mod downloader;
pub mod path;
pub mod server;

pub use catalog::{BUILTIN_CATALOG, GgufModelInfo};
pub use classifier::LlamaServerClassifier;
pub use downloader::ModelDownloader;
pub use path::{default_models_dir, find_llama_server_binary};
pub use server::{BUILTIN_BASE_URL, BUILTIN_LLAMA_PORT, LlamaServerService};