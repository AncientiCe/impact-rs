def init(with_option=False):
    return with_option


def enable_option():
    init(True)


# Runs on import, outside any function body — the always-executed path.
init()
