//! Registry of all post-build patcher modules.

pub mod binary_string;
pub mod headers;
pub mod la_cleaner;
pub mod object_cleaner;
pub mod pkgconfig;
pub mod placeholder;
pub mod rpath;

// Re-export the concrete types so callers can use
// `patchers::PlaceholderPatcher`, etc.
pub use binary_string::BinaryStringPatcher;
pub use headers::HeaderPatcher;
pub use la_cleaner::LaFileCleaner;
pub use object_cleaner::ObjectFileCleaner;
pub use pkgconfig::PkgConfigPatcher;
pub use placeholder::PlaceholderPatcher;
pub use rpath::RPathPatcher;
