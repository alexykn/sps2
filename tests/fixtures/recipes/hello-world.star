# Simple hello world package recipe for testing

def metadata(m):
    m.name("hello-world")
    m.version("1.0.0")
    m.description("Simple hello world package for testing")
    m.license("MIT")
    m.homepage("https://example.com/hello-world")

def build(b):
    # Fetch source
    b.fetch("https://example.com/hello-world-1.0.0.tar.gz", 
            "blake3:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
    
    # Simple build - just compile a hello world program
    b.run("mkdir -p src")
    b.write_file("src/hello.c", """
#include <stdio.h>

int main() {
    printf("Hello, World!\\n");
    return 0;
}
""")
    
    # Compile
    b.run("clang -o hello src/hello.c")
    
    # Install
    b.run("mkdir -p $PREFIX/bin")
    b.run("cp hello $PREFIX/bin/")
    
    # Add documentation
    b.write_file("README.md", "# Hello World\n\nA simple hello world program.\n")
    b.run("mkdir -p $PREFIX/share/doc/hello-world")
    b.run("cp README.md $PREFIX/share/doc/hello-world/")