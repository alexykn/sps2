# Test recipe with intentional error
def metadata():
    return {
        "name": 123,  # Wrong type - should be string
        "version": "1.0.0"
    }

def build(ctx):
    pass