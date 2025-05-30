# Test recipe for context passing
def metadata():
    return {
        "name": "test-context",
        "version": "1.2.3",
        "description": "Test package for context passing",
        "homepage": "https://example.com",
        "license": "MIT"
    }

def build(ctx):
    # Access metadata from context
    print("Building package: " + ctx.NAME + " version " + ctx.VERSION)
    print("Prefix: " + ctx.PREFIX)
    print("Jobs: " + str(ctx.JOBS))
    
    # For now, just pass - methods aren't implemented yet
    pass