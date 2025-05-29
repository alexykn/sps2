use std::env;
use std::path::PathBuf;

fn main() {
    // Set SQLX_OFFLINE_DIR if not already set
    if env::var("SQLX_OFFLINE_DIR").is_err() {
        // Check multiple possible locations
        let possible_dirs = vec![
            PathBuf::from("/opt/pm/.sqlx"),
            PathBuf::from(".sqlx"),
            env::current_dir().unwrap().join(".sqlx"),
        ];
        
        for dir in possible_dirs {
            if dir.exists() {
                println!("cargo:rustc-env=SQLX_OFFLINE_DIR={}", dir.display());
                break;
            }
        }
    }
    
    // Force offline mode in production builds
    if env::var("SQLX_OFFLINE").is_err() {
        println!("cargo:rustc-env=SQLX_OFFLINE=true");
    }
}