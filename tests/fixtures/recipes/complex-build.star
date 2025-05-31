# Complex build recipe with dependencies

def metadata(m):
    m.name("complex-app")
    m.version("2.1.3")
    m.description("A complex application with multiple dependencies")
    m.license("Apache-2.0")
    m.homepage("https://github.com/example/complex-app")
    m.depends_on("libssl>=1.1.1,<2.0")
    m.depends_on("zlib~=1.2.11")
    m.depends_on("sqlite>=3.36.0")
    m.depends_on("curl>=7.68.0,<8.0")
    m.build_depends_on("cmake>=3.16")
    m.build_depends_on("gcc>=9.0")
    m.build_depends_on("pkg-config>=0.29")

def build(b):
    # Fetch source with verification
    b.fetch("https://github.com/example/complex-app/archive/v2.1.3.tar.gz",
            "blake3:a665a45920422f9d417e4867efdc4fb8a04a1f3cbc663dda3c5c0c0c1b7e84c5")
    
    # Extract and enter source directory
    b.run("tar xzf v2.1.3.tar.gz")
    b.chdir("complex-app-2.1.3")
    
    # Configure with CMake
    b.run("mkdir build")
    b.chdir("build")
    
    b.run("cmake .. " +
          "-DCMAKE_INSTALL_PREFIX=$PREFIX " +
          "-DCMAKE_BUILD_TYPE=Release " +
          "-DWITH_SSL=ON " +
          "-DWITH_SQLITE=ON")
    
    # Build with multiple cores
    cores = b.cpu_count()
    b.run("make -j{}".format(cores))
    
    # Run tests
    b.run("make test")
    
    # Install
    b.run("make install")
    
    # Install additional documentation
    b.chdir("..")
    b.run("mkdir -p $PREFIX/share/doc/complex-app")
    b.run("cp README.md CHANGELOG.md LICENSE $PREFIX/share/doc/complex-app/")
    b.run("cp -r docs/ $PREFIX/share/doc/complex-app/")
    
    # Create sample configuration
    b.run("mkdir -p $PREFIX/share/complex-app")
    b.write_file("$PREFIX/share/complex-app/config.example.toml", """
[application]
name = "complex-app"
version = "2.1.3"
debug = false

[database]
type = "sqlite"
path = "~/.local/share/complex-app/data.db"

[network]
timeout = 30
retries = 3
""")