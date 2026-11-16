/**
 * Whether the page on screen comes from another release than the one the
 * server runs now. A version missing on either side is never a reason to
 * reload: the dev server hands the page out without one, and a reload would
 * not give it one.
 */
export function isStale(pageVersion, serverVersion) {
  const known = (version) => typeof version === 'string' && version !== ''
  return known(pageVersion) && known(serverVersion) && pageVersion !== serverVersion
}
