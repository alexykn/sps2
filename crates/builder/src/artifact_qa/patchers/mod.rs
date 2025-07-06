//! Registry of all post-build patcher modules.

pub mod binary_string;
pub mod codesigner;
pub mod headers;
pub mod la_cleaner;
pub mod object_cleaner;
pub mod permissions;
pub mod pkgconfig;
pub mod placeholder;
pub mod python_isolation;
pub mod rpath;

// Re-export the concrete types so callers can use
// `patchers::PlaceholderPatcher`, etc.
pub use binary_string::BinaryStringPatcher;
pub use codesigner::CodeSigner;
pub use headers::HeaderPatcher;
pub use la_cleaner::LaFileCleaner;
pub use object_cleaner::ObjectFileCleaner;
pub use permissions::PermissionsFixer;
pub use pkgconfig::PkgConfigPatcher;
pub use placeholder::PlaceholderPatcher;
pub use python_isolation::PythonIsolationPatcher;
pub use rpath::RPathPatcher;
