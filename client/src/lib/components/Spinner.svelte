<div class="spinner"></div>

<style lang="sass">
.spinner
	width: 32px
	height: 32px
	border: 3px solid var(--c-border)
	border-top-color: var(--c-accent)
	border-radius: 50%
	animation: spin 0.8s linear infinite
	margin: var(--sp-5) auto

@keyframes spin
	to
		transform: rotate(360deg)
</style>
