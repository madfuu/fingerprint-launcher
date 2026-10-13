/**
 * Why the startup did not end in a working app, for `showMainWindow(bootOk, failure)`: the
 * backend prints the reason, and a `FPL_SMOKE_EXIT_ON_READY` run (CI) fails on it.
 *
 * Anything that makes the first frame an error screen, or degrades it, reports here: the
 * bootstrap (settings / app info could not be read, the first render did not commit) and the
 * error boundary (rendering threw).
 */
const failures: string[] = [];

export function reportBootFailure(reason: string): void {
  failures.push(reason);
}

/** `null` while nothing failed; otherwise every reason, in order. */
export function bootFailure(): string | null {
  return failures.length === 0 ? null : failures.join('; ');
}
