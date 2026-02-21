<script>
	import config from '$lib/config.js';
</script>

<div class="login-page">
	<div class="login-card">
		<h1>youtube-sub-feed</h1>
		<p>登録チャンネルの最新動画を時系列で閲覧</p>
		<a class="login-btn" href="{config.path.api}/auth/login">
			Googleでログイン
		</a>
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
	padding: var(--sp-6)
	background: var(--c-surface)
	border: 1px solid var(--c-border)
	border-radius: var(--radius-lg)
	max-width: 360px
	width: 100%

	h1
		margin: 0 0 var(--sp-3)
		font-size: var(--fs-xl)

	p
		margin: 0 0 var(--sp-5)
		color: var(--c-text-sub)
		font-size: var(--fs-sm)

.login-btn
	display: inline-block
	padding: var(--sp-3) var(--sp-5)
	background: var(--c-accent)
	color: white
	border-radius: var(--radius-md)
	font-size: var(--fs-md)
	text-decoration: none

	&:hover
		background: var(--c-accent-hover)
		color: white
</style>
