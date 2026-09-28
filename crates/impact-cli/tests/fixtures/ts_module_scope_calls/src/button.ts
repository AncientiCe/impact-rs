import { withTheme } from './theme';

const getBaseStyles = () => ({});

function Button() {
  return null;
}

// A curried call: the outer call invokes whatever `withTheme(...)` returns, not
// `getBaseStyles`, the last identifier inside its callee.
export default withTheme('button', getBaseStyles)(Button);
