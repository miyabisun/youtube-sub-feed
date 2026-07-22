<script>
  // Inline SVG wrapper: 24x24 stroke icons that inherit currentColor.
  // The shapes (line/polygon/circle/…) are passed as children.
  let { children } = $props()
</script>

<svg
  class="icon"
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {@render children()}
</svg>

<style lang="sass">
.icon
	width: 1.2em
	height: 1.2em
</style>
