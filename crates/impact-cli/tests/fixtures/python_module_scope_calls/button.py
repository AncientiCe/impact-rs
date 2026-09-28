from registry import register


def get_base_styles():
    return {}


def button():
    return None


# A curried call: the outer call invokes whatever `register(...)` returns, not
# `get_base_styles`, the last identifier inside its callee.
register("button", get_base_styles)(button)
