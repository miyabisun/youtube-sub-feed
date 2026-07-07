export function formatDuration(iso) {
  if (!iso) return ''
  // YouTube returns ISO 8601 durations. Streams/archives longer than 24h carry a
  // date part (e.g. "P1DT2H3M4S"); fold days into hours so 1 day = 24 hours.
  const match = iso.match(/P(?:(\d+)D)?T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?/)
  if (!match) return ''
  const d = parseInt(match[1] || '0', 10)
  const h = parseInt(match[2] || '0', 10) + d * 24
  const m = parseInt(match[3] || '0', 10)
  const s = parseInt(match[4] || '0', 10)
  if (h > 0) return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
  return `${m}:${String(s).padStart(2, '0')}`
}
