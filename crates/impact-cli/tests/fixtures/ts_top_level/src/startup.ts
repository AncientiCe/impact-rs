import { compute } from './util';

// Runs at module load. Nothing declares a function here, so this call used to vanish.
export const STARTUP = compute();
