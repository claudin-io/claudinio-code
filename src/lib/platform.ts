/** Platform detection utility */
export type Platform = 'mac' | 'win' | 'linux';

export function platform(): Platform {
  if (navigator.userAgent.includes('Mac')) return 'mac';
  if (navigator.userAgent.includes('Win')) return 'win';
  return 'linux';
}

/** Returns the human-readable reveal label for the current platform */
export function revealLabel(): string {
  const p = platform();
  if (p === 'mac') return 'Reveal in Finder';
  if (p === 'win') return 'Show in Explorer';
  return 'Open in File Manager';
}

