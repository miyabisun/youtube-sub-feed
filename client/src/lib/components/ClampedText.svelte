<script>
  // Plain text with its line breaks kept, clamped to a few lines behind an
  // inline expand link. The link appears only when the text actually overflows.
  let { text } = $props()

  let body = $state(null)
  let clamped = $state(false)
  let expanded = $state(false)

  $effect(() => {
    if (!body || expanded) return
    const measure = () => (clamped = body.scrollHeight > body.clientHeight + 1)
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(body)
    return () => observer.disconnect()
  })
</script>

<p class="content" class:expanded bind:this={body}>{text}</p>
{#if clamped || expanded}
  <button class="more" onclick={() => (expanded = !expanded)} aria-expanded={expanded}>
    {expanded ? '閉じる' : '続きを読む'}
  </button>
{/if}

<style lang="sass">
.content
	margin: 0
	white-space: pre-wrap
	overflow-wrap: anywhere
	line-height: 1.6
	display: -webkit-box
	-webkit-line-clamp: 6
	-webkit-box-orient: vertical
	overflow: hidden

	&.expanded
		display: block
		-webkit-line-clamp: unset

.more
	align-self: flex-start
	padding: 0
	background: none
	border: none
	color: var(--c-link)
	font-size: var(--fs-sm)
	cursor: pointer

@media (min-width: 768px)
	.content:not(.expanded)
		-webkit-line-clamp: 4
</style>
