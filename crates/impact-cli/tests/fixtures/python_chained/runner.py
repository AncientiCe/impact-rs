from factory import build_client


def run():
    return build_client().send()
