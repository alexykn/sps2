def metadata():
    """Package metadata"""
    return {
        "name": "ripgrep",
        "version": "14.1.1",
        "description": """ripgrep is a line-oriented search tool that recursively searches the current
directory for a regex pattern while respecting gitignore rules. ripgrep has
first class support on Windows, macOS and Linux.
""",
        "license": "Unlicense OR MIT",
        "homepage": "https://github.com/BurntSushi/ripgrep"
    }

def build(ctx):
    # Clean up any leftover files from previous builds
    cleanup(ctx)
    # Clone git repository
    git(ctx, "https://github.com/BurntSushi/ripgrep", "HEAD")
    # Allow network access for dependency downloads
    allow_network(ctx, True)

    # Build using cargo build system
    cargo(ctx, [
        "--release"
    ])
