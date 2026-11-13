<script lang="ts">
	import { SectionHeader, Stack, Tabs } from '@dorsk/tsumikit';
	import type { Snippet } from 'svelte';

	let { tab, actions }: { tab: 'endpoints' | 'deliveries'; actions?: Snippet } = $props();

	const TABS = [
		{ id: 'endpoints', label: 'Endpoints', href: '/webhooks' },
		{ id: 'deliveries', label: 'Deliveries', href: '/webhooks/deliveries' },
	];
</script>

<Stack gap="var(--sp-3)">
	<SectionHeader
		title="Webhooks"
		level={2}
		size="lg"
		subtitle="Signed POST requests sent when a report arrives or changes status."
		{actions}
	/>
	<Tabs tabs={TABS} value={tab} label="Webhook views" />
</Stack>
