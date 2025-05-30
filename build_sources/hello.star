# Build recipe for hello - a simple test program
def metadata():
    # For now, return a simple dict - our parser will handle this
    return {
        "name": "hello",
        "version": "1.0.0", 
        "description": "A simple hello world program",
        "homepage": "https://github.com/spsv2/hello",
        "license": "MIT"
    }

def build(ctx):
    # Note: Our current simplified API doesn't have the full context methods yet
    # This will be enhanced in the next iteration
    pass