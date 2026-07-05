<div class="login-page">
	<div class="login-card">
		<h1>youtube-sub-feed</h1>
		<p>登録チャンネルの最新動画を時系列で閲覧</p>
		<p class="note">Cloudflare Access で認証してアクセスしてください。</p>
	</div>
</div>

<style lang="sass">
.login-page
	display: flex
	align-items: center
	justify-content: center
	height: 100%
	padding: var(--sp-5)

.login-card
	text-align: center
	padding: var(--sp-5)
	background: var(--c-surface)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-lg)
	max-width: 360px
	width: 100%

	h1
		margin: 0 0 var(--sp-3)
		font-size: var(--fs-xl)

	p
		margin: 0 0 var(--sp-3)
		color: var(--c-text-sub)
		font-size: var(--fs-sm)

.note
	font-size: var(--fs-xs) !important
	color: var(--c-text-muted) !important
</style>
