<script lang="ts">
	import { Button, EmptyState } from '@dorsk/tsumikit';
</script>

<svelte:head><title>Not found · hmm admin</title></svelte:head>

<EmptyState
	icon="search"
	title="There is nothing at this address"
	description="The link may be old or incomplete."
>
	{#snippet action()}
		<Button href="/reports">Go to reports</Button>
	{/snippet}
</EmptyState>
