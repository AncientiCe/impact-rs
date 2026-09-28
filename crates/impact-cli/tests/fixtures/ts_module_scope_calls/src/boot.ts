function init(withOption = false) {
  return withOption;
}

export async function enableOption() {
  init(true);
}

// Runs on import, outside any function body — the always-executed path.
init();

export type Options = { withOption: boolean };
