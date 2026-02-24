<script>
	import { relativeTime } from '$lib/relative-time.js';
	import { formatDuration } from '$lib/format-duration.js';
	import { link } from '$lib/router.svelte.js';

	let { video } = $props();

	function getVideoUrl(v) {
		if (v.is_short) return `https://www.youtube.com/shorts/${v.id}`;
		return `https://www.youtube.com/watch?v=${v.id}`;
	}

	function getLabel(v) {
		if (v.is_short) return 'Shorts';
		if (v.is_livestream && !v.livestream_ended_at) return 'LIVE';
		if (v.is_livestream && v.livestream_ended_at) return '配信アーカイブ';
		return null;
	}

	function getLabelClass(v) {
		if (v.is_short) return 'label-shorts';
		if (v.is_livestream && !v.livestream_ended_at) return 'label-live';
		if (v.is_livestream) return 'label-archive';
		return '';
	}

	function hqThumbnail(url) {
		if (!url) return url;
		return url.replace('/mqdefault.', '/hqdefault.');
	}
</script>

<div class="video-card">
	<a class="thumbnail-link" href={getVideoUrl(video)} target="_blank" rel="noopener">
		<div class="thumbnail-wrap">
			{#if video.thumbnail_url}
				<img class="thumbnail" src={hqThumbnail(video.thumbnail_url)} alt="" loading="lazy" />
			{/if}
			{#if video.duration}
				<span class="duration">{formatDuration(video.duration)}</span>
			{/if}
			{#if getLabel(video)}
				<span class="label {getLabelClass(video)}">{getLabel(video)}</span>
			{/if}
		</div>
	</a>
	<div class="info">
		<a class="title" href={getVideoUrl(video)} target="_blank" rel="noopener">{video.title}</a>
		<div class="meta">
			{#if video.channel_title}
				<a class="channel" href={link(`/channel/${video.channel_id}`)}>{video.channel_title}</a>
			{/if}
			<span class="time">{relativeTime(video.published_at)}</span>
		</div>
	</div>
</div>

<style lang="sass">
.video-card
	display: block

.thumbnail-link
	display: block
	text-decoration: none

.thumbnail-wrap
	position: relative
	width: 100%
	aspect-ratio: 16 / 9
	background: var(--c-surface)
	border-radius: var(--radius-md)
	overflow: hidden

.thumbnail
	width: 100%
	height: 100%
	object-fit: cover

.duration
	position: absolute
	bottom: var(--sp-2)
	right: var(--sp-2)
	background: rgba(0, 0, 0, 0.8)
	color: #fff
	padding: 1px var(--sp-2)
	border-radius: var(--radius-sm)
	font-size: var(--fs-xs)

.label
	position: absolute
	top: var(--sp-2)
	left: var(--sp-2)
	padding: 1px var(--sp-2)
	border-radius: var(--radius-sm)
	font-size: var(--fs-xs)
	font-weight: bold

.label-shorts
	background: var(--c-shorts-bg)
	color: var(--c-shorts)
	border: 1px solid var(--c-shorts-border)

.label-live
	background: var(--c-live-bg)
	color: var(--c-live)
	border: 1px solid var(--c-live-border)
	animation: pulse 2s infinite

.label-archive
	background: var(--c-overlay-2)
	color: var(--c-text-sub)
	border: 1px solid var(--c-border)

@keyframes pulse
	0%, 100%
		opacity: 1
	50%
		opacity: 0.6

.info
	padding: var(--sp-2) 0

.title
	display: -webkit-box
	-webkit-line-clamp: 2
	-webkit-box-orient: vertical
	overflow: hidden
	font-size: var(--fs-md)
	line-height: 1.4
	color: inherit
	text-decoration: none

	&:hover
		text-decoration: underline

.meta
	display: flex
	align-items: center
	gap: var(--sp-3)
	margin-top: var(--sp-1)
	font-size: var(--fs-xs)
	color: var(--c-text-sub)

.channel
	color: var(--c-text-sub)
	text-decoration: none

	&:hover
		color: var(--c-text)
		text-decoration: underline

.time
	color: var(--c-text-muted)
</style>
