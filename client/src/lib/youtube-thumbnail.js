/**
 * Build a YouTube thumbnail URL from a video ID.
 *
 * YouTube serves a deterministic thumbnail URL keyed by video ID under
 * https://i.ytimg.com/vi/{id}/{quality}.jpg, so we never need to store
 * thumbnail URLs in the DB or fetch them from the Data API.
 *
 * Quality options:
 *   - default      120x90    (always exists)
 *   - mqdefault    320x180   (always exists)
 *   - hqdefault    480x360   (always exists, our default)
 *   - sddefault    640x480   (sometimes missing on older videos)
 *   - maxresdefault 1280x720 (only when the source was uploaded in HD)
 */
export function videoThumbnail(videoId, quality = 'hqdefault') {
	if (!videoId) return null;
	return `https://i.ytimg.com/vi/${videoId}/${quality}.jpg`;
}
