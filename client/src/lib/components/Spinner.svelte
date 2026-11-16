<script>
  // `inline` drops the block margin, for a spinner that sits beside a label.
  let { inline = false } = $props()
</script>

<div class="spinner" class:inline></div>

<style lang="sass">
.spinner
	width: 1.1rem
	height: 1.1rem
	border: 1.5px solid var(--c-border)
	border-top-color: var(--c-accent)
	border-radius: 50%
	animation: spin 0.8s linear infinite
	margin: var(--sp-5) auto

	&.inline
		margin: 0

@keyframes spin
	to
		transform: rotate(360deg)
</style>
