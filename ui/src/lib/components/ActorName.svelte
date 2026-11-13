<script lang="ts">
	import { Badge, Text } from '@dorsk/tsumikit';
	import { parseActor } from '../labels.ts';
	import { me } from '../session.svelte.ts';

	let { actor }: { actor: string } = $props();

	const parsed = $derived(parseActor(actor));
	const self = $derived(me.data?.actor === actor);
</script>

<Text weight="medium">{self ? 'You' : parsed.name}</Text>{#if !self && (parsed.role === 'key' || parsed.role === 'admin')}
	{' '}<Badge size="xs" tone="muted">{parsed.role}</Badge>
{/if}
