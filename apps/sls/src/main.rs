//! sls - Store List utility for sps2
//!
//! A simple ls-like tool to explore the content-addressed store

use clap::Parser;
use sps2_config::fixed_paths;
use sps2_state::create_pool;
use sqlx::Acquire;

use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(Parser)]
#[command(name = "sls")]
#[command(about = "List store contents with real filenames", long_about = None)]
struct Cli {
    /// Path or hash prefix to list
    path: Option<String>,

    /// Use a long listing format (shows permissions, size, etc)
    #[arg(short, long)]
    long: bool,

    /// Show only hash without filename mapping
    #[arg(long)]
    hash: bool,

    /// List subdirectories recursively
    #[arg(short = 'R', long)]
    recursive: bool,

    /// Store path (defaults to /opt/pm/store)
    #[arg(long)]
    store: Option<PathBuf>,

    /// Database path (defaults to /opt/pm/state.sqlite)
    #[arg(long)]
    db: Option<PathBuf>,

    /// Show all entries (including . files)
    #[arg(short, long)]
    all: bool,

    /// Disable colored output
    #[arg(long)]
    no_color: bool,

    /// List packages instead of objects
    #[arg(short = 'p', long = "packages")]
    packages: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Exit cleanly when stdout is closed (e.g., piped to head)
    #[cfg(unix)]
    unsafe {
        // Reset SIGPIPE to default so the process terminates without a panic
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();

    let store_path = cli
        .store
        .unwrap_or_else(|| PathBuf::from(fixed_paths::STORE_DIR));
    let db_path = cli
        .db
        .unwrap_or_else(|| PathBuf::from(fixed_paths::DB_PATH));
    let use_color = !cli.no_color && std::io::stdout().is_terminal();

    if cli.packages {
        // List packages instead of objects
        let package_map = load_package_mappings(&db_path).await?;
        if let Some(path) = cli.path {
            list_specific_package(&store_path, &path, &package_map, cli.long, use_color).await?;
        } else {
            list_packages(
                &store_path,
                &package_map,
                cli.long,
                cli.recursive,
                use_color,
            )
            .await?;
        }
    } else {
        // Open database to get file mappings
        let file_map = load_file_mappings(&db_path).await?;

        if let Some(path) = cli.path {
            // User specified a path/hash
            list_specific(
                &store_path,
                &path,
                &file_map,
                cli.long,
                cli.hash,
                cli.recursive,
                use_color,
            )
            .await?;
        } else {
            // List all
            list_store(
                &store_path,
                &file_map,
                cli.long,
                cli.hash,
                cli.recursive,
                use_color,
            )
            .await?;
        }
    }

    Ok(())
}

async fn load_file_mappings(
    db_path: &Path,
) -> Result<HashMap<String, Vec<String>>, Box<dyn std::error::Error>> {
    let mut map = HashMap::new();

    // Open database connection using state crate
    let pool = create_pool(db_path).await?;
    let mut conn = pool.acquire().await?;
    let mut tx = conn.begin().await?;

    // For now, we'll still use a raw query since we need ALL mappings
    // In the future, we could add a batch function to the state crate
    use sqlx::Row;
    let rows = sqlx::query(
        r#"
        SELECT DISTINCT
            pfe.file_hash,
            pfe.relative_path,
            p.name as package_name,
            p.version as package_version
        FROM package_file_entries pfe
        JOIN packages p ON p.id = pfe.package_id
        ORDER BY pfe.file_hash, pfe.relative_path
        "#,
    )
    .fetch_all(&mut *tx)
    .await?;

    for row in rows {
        let file_hash: String = row.get("file_hash");
        let relative_path: String = row.get("relative_path");
        let package_name: String = row.get("package_name");
        let package_version: String = row.get("package_version");

        let entry = format!("{relative_path} ({package_name}:{package_version})");

        map.entry(file_hash).or_insert_with(Vec::new).push(entry);
    }

    tx.commit().await?;

    Ok(map)
}

async fn list_store(
    store_path: &Path,
    file_map: &HashMap<String, Vec<String>>,
    long_format: bool,
    hash_only: bool,
    recursive: bool,
    use_color: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let objects_dir = store_path.join("objects");

    if recursive {
        list_recursive(&objects_dir, file_map, long_format, hash_only, use_color, 0).await?;
    } else {
        // List the first-level directories
        let mut entries = fs::read_dir(&objects_dir).await?;
        let mut dirs = Vec::new();

        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_dir() {
                dirs.push(entry.file_name().to_string_lossy().to_string());
            }
        }

        dirs.sort();
        for dir in dirs {
            println!("{}/", style_blue(&dir, use_color));
        }
    }

    Ok(())
}

async fn list_recursive(
    dir: &Path,
    file_map: &HashMap<String, Vec<String>>,
    long_format: bool,
    hash_only: bool,
    use_color: bool,
    depth: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let indent = "  ".repeat(depth);

    let mut entries = fs::read_dir(dir).await?;
    let mut items = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        items.push(entry);
    }

    // Sort entries
    items.sort_by_key(|e| e.file_name());

    for entry in items {
        let metadata = entry.metadata().await?;
        let name = entry.file_name().to_string_lossy().to_string();

        if metadata.is_dir() {
            println!("{indent}{}/", style_blue(&name, use_color));

            // Recursive listing handled at top level
            Box::pin(list_recursive(
                &entry.path(),
                file_map,
                long_format,
                hash_only,
                use_color,
                depth + 1,
            ))
            .await?;
        } else {
            // It's a file - the filename is the hash
            let full_hash = &name;

            if hash_only {
                println!("{indent}{full_hash}");
            } else if long_format {
                let size = format_size(metadata.len());
                let perms = format_permissions(&metadata);

                if let Some(names) = file_map.get(full_hash) {
                    for file_name in names {
                        println!(
                            "{}{} {:>8} {} -> {}",
                            indent,
                            perms,
                            size,
                            style_dimmed(short_hash(full_hash, 16), use_color),
                            style_green(file_name, use_color)
                        );
                    }
                } else {
                    println!(
                        "{}{} {:>8} {} (unknown)",
                        indent,
                        perms,
                        size,
                        style_dimmed(short_hash(full_hash, 16), use_color)
                    );
                }
            } else {
                // Default: short hash + filename
                if let Some(names) = file_map.get(full_hash) {
                    for file_name in names {
                        println!(
                            "{}{} {}",
                            indent,
                            style_dimmed(short_hash(full_hash, 8), use_color),
                            style_green(file_name, use_color)
                        );
                    }
                } else {
                    println!(
                        "{}{} (unknown)",
                        indent,
                        style_dimmed(short_hash(full_hash, 8), use_color)
                    );
                }
            }
        }
    }

    Ok(())
}

async fn list_specific(
    store_path: &Path,
    path_or_hash: &str,
    file_map: &HashMap<String, Vec<String>>,
    long_format: bool,
    hash_only: bool,
    recursive: bool,
    use_color: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Check if it's a hex hash prefix with at least 2 chars
    let is_hex = path_or_hash.chars().all(|c| c.is_ascii_hexdigit());
    if path_or_hash.len() >= 2 && is_hex {
        let prefix = path_or_hash.to_ascii_lowercase();
        let prefix1 = &prefix[..2];

        let objects = store_path.join("objects");
        let p1_dir = objects.join(prefix1);
        if !p1_dir.exists() {
            eprintln!("No objects found with prefix '{path_or_hash}'");
            return Ok(());
        }

        // Length-based behavior:
        // - len == 2: list second-level dirs (00-ff)
        // - len == 3: list second-level dirs starting with the 3rd nibble
        // - len >= 4: list files in p1/p2 whose name starts with full prefix
        match prefix.len() {
            2 => {
                let mut entries = fs::read_dir(&p1_dir).await?;
                let mut dirs = Vec::new();
                while let Some(e) = entries.next_entry().await? {
                    if e.file_type().await?.is_dir() {
                        dirs.push(e.file_name().to_string_lossy().to_string());
                    }
                }
                dirs.sort();
                if recursive {
                    for d in dirs {
                        println!("{}/", style_blue(&d, use_color));
                        Box::pin(list_recursive(
                            &p1_dir.join(&d),
                            file_map,
                            long_format,
                            hash_only,
                            use_color,
                            1,
                        ))
                        .await?;
                    }
                } else {
                    for d in dirs {
                        println!("{}/", style_blue(&d, use_color));
                    }
                }
            }
            3 => {
                let p2_prefix = &prefix[2..3];
                let mut entries = fs::read_dir(&p1_dir).await?;
                let mut dirs = Vec::new();
                while let Some(e) = entries.next_entry().await? {
                    if e.file_type().await?.is_dir() {
                        let name = e.file_name().to_string_lossy().to_string();
                        if name.starts_with(p2_prefix) {
                            dirs.push(name);
                        }
                    }
                }
                if dirs.is_empty() {
                    eprintln!("No objects found with prefix '{path_or_hash}'");
                } else {
                    dirs.sort();
                    if recursive {
                        for d in dirs {
                            println!("{}/", style_blue(&d, use_color));
                            Box::pin(list_recursive(
                                &p1_dir.join(&d),
                                file_map,
                                long_format,
                                hash_only,
                                use_color,
                                1,
                            ))
                            .await?;
                        }
                    } else {
                        for d in dirs {
                            println!("{}/", style_blue(&d, use_color));
                        }
                    }
                }
            }
            _ => {
                // len >= 4
                let p2 = &prefix[2..4];
                let dir = p1_dir.join(p2);
                if !dir.exists() {
                    eprintln!("No objects found with prefix '{path_or_hash}'");
                    return Ok(());
                }

                let mut entries = fs::read_dir(&dir).await?;
                let mut found = false;
                while let Some(entry) = entries.next_entry().await? {
                    let name = entry.file_name().to_string_lossy().to_string();
                    // In the current layout, 'name' is the full hash
                    if !name.starts_with(&prefix) {
                        continue;
                    }
                    found = true;
                    let full_hash = name;
                    let metadata = entry.metadata().await?;

                    if hash_only {
                        println!("{full_hash}");
                    } else if long_format {
                        let size = format_size(metadata.len());
                        let perms = format_permissions(&metadata);

                        if let Some(names) = file_map.get(&full_hash) {
                            for file_name in names {
                                println!(
                                    "{} {:>8} {} -> {}",
                                    perms,
                                    size,
                                    style_dimmed(short_hash(&full_hash, 16), use_color),
                                    style_green(file_name, use_color)
                                );
                            }
                        } else {
                            println!(
                                "{} {:>8} {} (unknown)",
                                perms,
                                size,
                                style_dimmed(short_hash(&full_hash, 16), use_color)
                            );
                        }
                    } else if let Some(names) = file_map.get(&full_hash) {
                        for file_name in names {
                            println!(
                                "{} {}",
                                style_dimmed(short_hash(&full_hash, 8), use_color),
                                style_green(file_name, use_color)
                            );
                        }
                    } else {
                        println!(
                            "{} (unknown)",
                            style_dimmed(short_hash(&full_hash, 8), use_color)
                        );
                    }
                }
                if !found {
                    eprintln!("No objects found with prefix '{path_or_hash}'");
                }
            }
        }
    } else {
        eprintln!("Invalid hash prefix: {path_or_hash}");
    }

    Ok(())
}

fn short_hash(s: &str, n: usize) -> &str {
    if s.len() <= n {
        s
    } else {
        &s[..n]
    }
}

fn format_permissions(metadata: &std::fs::Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();

        let file_type = if metadata.is_dir() { 'd' } else { '-' };

        let user = format!(
            "{}{}{}",
            if mode & 0o400 != 0 { 'r' } else { '-' },
            if mode & 0o200 != 0 { 'w' } else { '-' },
            if mode & 0o100 != 0 { 'x' } else { '-' }
        );

        let group = format!(
            "{}{}{}",
            if mode & 0o040 != 0 { 'r' } else { '-' },
            if mode & 0o020 != 0 { 'w' } else { '-' },
            if mode & 0o010 != 0 { 'x' } else { '-' }
        );

        let other = format!(
            "{}{}{}",
            if mode & 0o004 != 0 { 'r' } else { '-' },
            if mode & 0o002 != 0 { 'w' } else { '-' },
            if mode & 0o001 != 0 { 'x' } else { '-' }
        );

        format!("{file_type}{user}{group}{other}")
    }
    #[cfg(not(unix))]
    {
        if metadata.permissions().readonly() {
            "-r--r--r--".to_string()
        } else {
            "-rw-rw-rw-".to_string()
        }
    }
}

fn apply_ansi_style(text: &str, code: &str, use_color: bool) -> String {
    if use_color {
        format!("\x1b[{code}{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn style_dimmed(text: &str, use_color: bool) -> String {
    apply_ansi_style(text, "2m", use_color)
}

fn style_blue(text: &str, use_color: bool) -> String {
    apply_ansi_style(text, "34m", use_color)
}

fn style_green(text: &str, use_color: bool) -> String {
    apply_ansi_style(text, "32m", use_color)
}

fn style_yellow(text: &str, use_color: bool) -> String {
    apply_ansi_style(text, "33m", use_color)
}

fn style_cyan(text: &str, use_color: bool) -> String {
    apply_ansi_style(text, "36m", use_color)
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{size:.0} {}", UNITS[unit_index])
    } else {
        format!("{size:.1} {}", UNITS[unit_index])
    }
}

async fn load_package_mappings(
    db_path: &Path,
) -> Result<HashMap<String, (String, String)>, Box<dyn std::error::Error>> {
    let mut map = HashMap::new();

    // Open database connection using state crate
    let pool = create_pool(db_path).await?;
    let mut conn = pool.acquire().await?;
    let mut tx = conn.begin().await?;

    // Query packages with their hashes
    use sqlx::Row;
    let rows = sqlx::query(
        r#"
        SELECT
            hash,
            name,
            version
        FROM packages
        ORDER BY name, version
        "#,
    )
    .fetch_all(&mut *tx)
    .await?;

    for row in rows {
        let hash: String = row.get("hash");
        let name: String = row.get("name");
        let version: String = row.get("version");

        map.insert(hash, (name, version));
    }

    tx.commit().await?;

    Ok(map)
}

async fn list_packages(
    store_path: &Path,
    package_map: &HashMap<String, (String, String)>,
    long_format: bool,
    recursive: bool,
    use_color: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let packages_dir = store_path.join("packages");

    if !packages_dir.exists() {
        eprintln!(
            "Packages directory does not exist: {}",
            packages_dir.display()
        );
        return Ok(());
    }

    let mut entries = fs::read_dir(&packages_dir).await?;
    let mut packages = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir() {
            packages.push(entry);
        }
    }

    // Sort by name
    packages.sort_by_key(|e| e.file_name());

    for entry in packages {
        let hash = entry.file_name().to_string_lossy().to_string();

        if long_format {
            let metadata = entry.metadata().await?;
            let perms = format_permissions(&metadata);

            if let Some((name, version)) = package_map.get(&hash) {
                println!(
                    "{} {} ({}:{})",
                    perms,
                    style_dimmed(&hash, use_color),
                    style_cyan(name, use_color),
                    style_yellow(version, use_color)
                );
            } else {
                println!(
                    "{} {} (unknown package)",
                    perms,
                    style_dimmed(&hash, use_color)
                );
            }

            if recursive {
                // List contents of package directory
                let mut pkg_entries = fs::read_dir(entry.path()).await?;
                let mut files = Vec::new();

                while let Some(pkg_entry) = pkg_entries.next_entry().await? {
                    files.push(pkg_entry.file_name().to_string_lossy().to_string());
                }

                files.sort();
                for file in files {
                    println!("  {}", style_green(&file, use_color));
                }
            }
        } else {
            // Short format
            if let Some((name, version)) = package_map.get(&hash) {
                println!(
                    "{} -> {}:{}",
                    style_dimmed(short_hash(&hash, 16), use_color),
                    style_cyan(name, use_color),
                    style_yellow(version, use_color)
                );
            } else {
                println!(
                    "{} (unknown)",
                    style_dimmed(short_hash(&hash, 16), use_color)
                );
            }
        }
    }

    Ok(())
}

async fn list_specific_package(
    store_path: &Path,
    hash_prefix: &str,
    package_map: &HashMap<String, (String, String)>,
    long_format: bool,
    use_color: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let packages_dir = store_path.join("packages");

    if !packages_dir.exists() {
        eprintln!(
            "Packages directory does not exist: {}",
            packages_dir.display()
        );
        return Ok(());
    }

    let mut entries = fs::read_dir(&packages_dir).await?;
    let mut found = false;

    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir() {
            let hash = entry.file_name().to_string_lossy().to_string();

            if hash.starts_with(hash_prefix) {
                found = true;

                if long_format {
                    let metadata = entry.metadata().await?;
                    let perms = format_permissions(&metadata);

                    if let Some((name, version)) = package_map.get(&hash) {
                        println!(
                            "{} {} ({}:{})",
                            perms,
                            style_dimmed(&hash, use_color),
                            style_cyan(name, use_color),
                            style_yellow(version, use_color)
                        );
                    } else {
                        println!(
                            "{} {} (unknown package)",
                            perms,
                            style_dimmed(&hash, use_color)
                        );
                    }

                    // List contents of package directory
                    let mut pkg_entries = fs::read_dir(entry.path()).await?;
                    let mut files = Vec::new();

                    while let Some(pkg_entry) = pkg_entries.next_entry().await? {
                        files.push(pkg_entry.file_name().to_string_lossy().to_string());
                    }

                    files.sort();
                    for file in files {
                        println!("  {}", style_green(&file, use_color));
                    }
                } else {
                    // Short format
                    if let Some((name, version)) = package_map.get(&hash) {
                        println!(
                            "{} -> {}:{}",
                            style_dimmed(&hash, use_color),
                            style_cyan(name, use_color),
                            style_yellow(version, use_color)
                        );
                    } else {
                        println!("{} (unknown)", style_dimmed(&hash, use_color));
                    }
                }
            }
        }
    }

    if !found {
        eprintln!("No packages found with prefix '{hash_prefix}'");
    }

    Ok(())
}
