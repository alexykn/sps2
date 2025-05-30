def metadata():
    return struct(
        name = "test",
        version = "1.0.0"
    )

def build(ctx):
    ctx.install()