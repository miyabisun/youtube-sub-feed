export function formatDuration(iso) {
	if (!iso) return '';
	const match = iso.match(/PT(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?/);
	if (!match) return '';
	const h = parseInt(match[1] || '0', 10);
	const m = parseInt(match[2] || '0', 10);
	const s = parseInt(match[3] || '0', 10);
	if (h > 0) return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
	return `${m}:${String(s).padStart(2, '0')}`;
}
