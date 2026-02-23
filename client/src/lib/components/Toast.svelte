<script>
	let { message = '', type = 'success' } = $props();
	let visible = $state(true);

	$effect(() => {
		const duration = type === 'error' ? 3000 : 500;
		const timer = setTimeout(() => { visible = false; }, duration);
		return () => clearTimeout(timer);
	});
</script>

{#if visible}
	<div class="toast" class:error={type === 'error'}>
		{message}
	</div>
{/if}

<style lang="sass">
.toast
	position: fixed
	bottom: var(--sp-5)
	left: 50%
	transform: translateX(-50%)
	padding: var(--sp-3) var(--sp-5)
	border-radius: var(--radius-md)
	background: var(--c-surface)
	border: 1px solid var(--c-border)
	color: var(--c-text)
	font-size: var(--fs-sm)
	z-index: 300
	pointer-events: none

	&.error
		background: var(--c-danger-bg)
		border-color: var(--c-danger-border)
		color: var(--c-danger)
</style>
