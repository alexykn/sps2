//! Binary operations for macOS platform (install_name_tool, otool, codesign)

use async_trait::async_trait;
use std::path::Path;
use sps2_errors::PlatformError;

use crate::core::PlatformContext;

/// Trait for binary manipulation operations specific to macOS
#[async_trait]
pub trait BinaryOperations: Send + Sync {
    /// Get the install name of a binary using otool -D
    async fn get_install_name(&self, ctx: &PlatformContext, binary: &Path) -> Result<Option<String>, PlatformError>;
    
    /// Set the install name of a binary using install_name_tool -id
    async fn set_install_name(&self, ctx: &PlatformContext, binary: &Path, name: &str) -> Result<(), PlatformError>;
    
    /// Get dependencies of a binary using otool -L
    async fn get_dependencies(&self, ctx: &PlatformContext, binary: &Path) -> Result<Vec<String>, PlatformError>;
    
    /// Change a dependency reference using install_name_tool -change
    async fn change_dependency(&self, ctx: &PlatformContext, binary: &Path, old: &str, new: &str) -> Result<(), PlatformError>;
    
    /// Add an rpath entry using install_name_tool -add_rpath
    async fn add_rpath(&self, ctx: &PlatformContext, binary: &Path, rpath: &str) -> Result<(), PlatformError>;
    
    /// Delete an rpath entry using install_name_tool -delete_rpath
    async fn delete_rpath(&self, ctx: &PlatformContext, binary: &Path, rpath: &str) -> Result<(), PlatformError>;
    
    /// Get rpath entries using otool -l
    async fn get_rpath_entries(&self, ctx: &PlatformContext, binary: &Path) -> Result<Vec<String>, PlatformError>;
    
    /// Verify binary signature using codesign -vvv
    async fn verify_signature(&self, ctx: &PlatformContext, binary: &Path) -> Result<bool, PlatformError>;
    
    /// Sign binary using codesign
    async fn sign_binary(&self, ctx: &PlatformContext, binary: &Path, identity: Option<&str>) -> Result<(), PlatformError>;
}