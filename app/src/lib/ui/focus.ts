/** Svelte action: focuses the element once it is on screen (the `autofocus`
 * attribute is unreliable for elements added after page load, and blocks
 * like `{#if}` attach their DOM after actions run). */
export function focusOnMount(node: HTMLElement) {
  const frame = requestAnimationFrame(() => node.focus())
  return { destroy: () => cancelAnimationFrame(frame) }
}
